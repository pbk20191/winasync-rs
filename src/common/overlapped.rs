//! Unified TP_IO overlapped-IO core shared by `tp` and `comio`.
//!
//! One registration + op-block + completion engine, parameterized by:
//! * the TP callback **environment** (`tp` uses its runtime's private pool;
//!   `comio` uses the default process pool, `pcbe = NULL`), and
//! * an optional **keepalive** kept alive for the registration's lifetime
//!   (`tp` passes its `RuntimeInner` so the pool outlives in-flight ops).
//!
//! The primitive is completion-callback based (`CompleteFn` + [`OpHandle`] with
//! explicit `cancel`), which `comio` uses directly. `tp` layers its
//! `Future`/drop-cancels API on top (see `tp::io`). The op block is generic over
//! a payload `P` that the kernel may write into (buffers); it lives in the block
//! until completion. Invariants preserved from the two cores this replaces:
//! refcount starts at 2 (caller + kernel), freed exactly once; `StartThreadpoolIo`
//! balanced 1:1 by a completion or `CancelThreadpoolIo` on sync failure; no panic
//! crosses the completion FFI boundary; `LIVE_OPS` tracks live blocks in debug.

use core::ffi::c_void;
use std::any::Any;
use std::cell::UnsafeCell;
use std::mem;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Networking::WinSock::{closesocket, SOCKET};
use windows::Win32::System::Threading::{
    CancelThreadpoolIo, CloseThreadpoolIo, CreateThreadpoolIo, StartThreadpoolIo,
    PTP_CALLBACK_INSTANCE, PTP_IO, TP_CALLBACK_ENVIRON_V3,
};
use windows::Win32::System::IO::{CancelIoEx, OVERLAPPED};

/// Number of live op blocks (debug builds) — leak tests assert this settles to 0.
#[cfg(debug_assertions)]
pub static LIVE_OPS: AtomicUsize = AtomicUsize::new(0);

/// A handle registered with a pool's TP_IO; owns the kernel handle + TP_IO.
pub struct IoReg {
    pub handle: HANDLE,
    io: PTP_IO,
    is_socket: bool,
    /// Kept alive for the registration's lifetime (e.g. `tp`'s `RuntimeInner`
    /// so the pool outlives in-flight ops). `None` for the default pool.
    _keepalive: Option<Arc<dyn Any + Send + Sync>>,
}

unsafe impl Send for IoReg {}
unsafe impl Sync for IoReg {}

impl IoReg {
    /// Registers `handle` with the TP_IO completion engine. `env` is the pool
    /// callback environment (`None` = default process pool); `keepalive` is held
    /// for the registration's life. Takes ownership of `handle` on success.
    pub fn register(
        handle: HANDLE,
        is_socket: bool,
        env: Option<*const TP_CALLBACK_ENVIRON_V3>,
        keepalive: Option<Arc<dyn Any + Send + Sync>>,
    ) -> windows_core::Result<Arc<Self>> {
        let io = unsafe { CreateThreadpoolIo(handle, Some(io_complete), None, env)? };
        Ok(Arc::new(Self {
            handle,
            io,
            is_socket,
            _keepalive: keepalive,
        }))
    }

    pub fn socket(&self) -> SOCKET {
        SOCKET(self.handle.0 as usize)
    }

    pub fn is_socket(&self) -> bool {
        self.is_socket
    }
}

impl Drop for IoReg {
    fn drop(&mut self) {
        // Every in-flight op holds an Arc<IoReg> released only at its completion
        // tail, so by here all started IO has completed — closing is legal. This
        // may run at the tail of the last completion callback, so
        // WaitForThreadpoolIoCallbacks is NOT used (it would self-deadlock).
        unsafe {
            CloseThreadpoolIo(self.io);
            if self.is_socket {
                closesocket(self.socket());
            } else {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

/// Outcome of issuing the overlapped call, as classified by the caller.
pub enum Issue {
    /// A completion callback will arrive (sync success still queues one — we
    /// never set FILE_SKIP_COMPLETION_PORT_ON_SUCCESS — or ERROR_IO_PENDING).
    Pending,
    /// Hard synchronous failure — no completion will arrive.
    Failed(windows_core::HRESULT),
}

/// How an op finished, handed to its completion callback.
pub enum Completion {
    /// Delivered by the kernel: raw win32 `ioresult` (0 = success) + `bytes`.
    Done(u32, usize),
    /// Synchronous submit failure, carrying the **exact** `HRESULT` (so callers
    /// like `tp` that preserve it are not downgraded to a lossy win32 code).
    SyncFail(windows_core::HRESULT),
}

/// Completion action, run exactly once (on the pool thread for `Done`, inline
/// for `SyncFail`).
pub type CompleteFn = Box<dyn FnOnce(Completion) + Send>;

struct OpShared {
    fired: bool,
    on_complete: Option<CompleteFn>,
}

struct OpHeader {
    /// 2 at issue time: caller (`OpHandle`) + kernel/completion side.
    refs: AtomicUsize,
    shared: Mutex<OpShared>,
    /// Typed deallocation of the whole `Op<P>` block (header offset is
    /// P-independent thanks to `repr(C)` with OVERLAPPED first).
    release: unsafe fn(*mut c_void),
    /// Keeps the handle + TP_IO alive while the op is in flight.
    reg: Arc<IoReg>,
}

/// Heap op block. `repr(C)` with OVERLAPPED first, so the completion's
/// `overlapped` pointer *is* the block pointer and the header sits at a fixed
/// offset for every `P`. The payload owns whatever the kernel may touch.
#[repr(C)]
pub struct Op<P> {
    overlapped: OVERLAPPED,
    header: OpHeader,
    payload: UnsafeCell<Option<P>>,
}

unsafe fn release_op<P>(op: *mut c_void) {
    #[cfg(debug_assertions)]
    LIVE_OPS.fetch_sub(1, Ordering::Relaxed);
    unsafe { drop(Box::from_raw(op as *mut Op<P>)) }
}

unsafe fn release_ref(op: *mut c_void) {
    let header = unsafe { &(*(op as *mut Op<()>)).header };
    if header.refs.fetch_sub(1, Ordering::Release) == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        let release = header.release;
        unsafe { release(op) };
    }
}

/// The single completion callback for every registered handle; only touches the
/// header (offset-identical across payload types).
unsafe extern "system" fn io_complete(
    _: PTP_CALLBACK_INSTANCE,
    _ctx: *mut c_void,
    overlapped: *mut c_void,
    ioresult: u32,
    bytes: usize,
    _io: PTP_IO,
) {
    let header = unsafe { &(*(overlapped as *mut Op<()>)).header };
    let cb = {
        let mut s = header.shared.lock().unwrap();
        s.fired = true;
        s.on_complete.take()
    };
    if let Some(cb) = cb {
        let _ = catch_unwind(AssertUnwindSafe(move || cb(Completion::Done(ioresult, bytes))));
    }
    unsafe { release_ref(overlapped) };
}

/// Opaque handle to an in-flight op (for cancellation + payload retrieval).
/// Holds the caller-side ref; dropping it releases that ref.
pub struct OpHandle<P> {
    op: *mut Op<P>,
}

unsafe impl<P: Send> Send for OpHandle<P> {}

impl<P> OpHandle<P> {
    /// Requests cancellation if the op has not yet fired a completion.
    pub fn cancel(&self) {
        let header = unsafe { &(*self.op).header };
        let in_flight = !header.shared.lock().unwrap().fired;
        if in_flight {
            unsafe {
                let _ = CancelIoEx(header.reg.handle, Some(&(*self.op).overlapped));
            }
        }
    }

    /// Whether a completion has fired.
    pub fn fired(&self) -> bool {
        unsafe { (*self.op).header.shared.lock().unwrap().fired }
    }

    /// Takes the payload out of the op block.
    ///
    /// # Safety
    /// Only sound once the op has completed (the kernel no longer touches the
    /// payload). Used by the `Future` wrapper after it observes completion.
    pub unsafe fn take_payload(&self) -> Option<P> {
        unsafe { (*(*self.op).payload.get()).take() }
    }
}

impl<P> Drop for OpHandle<P> {
    fn drop(&mut self) {
        unsafe { release_ref(self.op as *mut c_void) }
    }
}

/// Allocates an op block, arms the TP_IO (`StartThreadpoolIo` before issuing),
/// runs `issue`, and on `Failed` balances with `CancelThreadpoolIo` and calls
/// `on_complete(win32, 0)` synchronously.
///
/// # Safety
/// `issue` must issue exactly one overlapped operation on `reg.handle` using the
/// provided OVERLAPPED pointer and classify truthfully: `Failed` when a
/// completion is still coming is a use-after-free; `Pending` when none is coming
/// leaks the block and unbalances the TP_IO.
pub unsafe fn submit<P>(
    reg: &Arc<IoReg>,
    payload: P,
    on_complete: CompleteFn,
    issue: impl FnOnce(*mut OVERLAPPED, &mut P) -> Issue,
) -> OpHandle<P> {
    let op = Box::into_raw(Box::new(Op {
        overlapped: unsafe { mem::zeroed() },
        header: OpHeader {
            refs: AtomicUsize::new(2),
            shared: Mutex::new(OpShared {
                fired: false,
                on_complete: Some(on_complete),
            }),
            release: release_op::<P>,
            reg: reg.clone(),
        },
        payload: UnsafeCell::new(Some(payload)),
    }));
    #[cfg(debug_assertions)]
    LIVE_OPS.fetch_add(1, Ordering::Relaxed);

    unsafe { StartThreadpoolIo(reg.io) };
    let outcome = unsafe {
        let p = (*(*op).payload.get()).as_mut().unwrap();
        issue(&raw mut (*op).overlapped, p)
    };
    if let Issue::Failed(hr) = outcome {
        unsafe {
            CancelThreadpoolIo(reg.io);
            let cb = {
                let mut s = (*op).header.shared.lock().unwrap();
                s.fired = true;
                s.on_complete.take()
            };
            if let Some(cb) = cb {
                // Hand the exact HRESULT to the callback (no lossy win32
                // downgrade here; callers that want one do it themselves).
                let _ = catch_unwind(AssertUnwindSafe(move || cb(Completion::SyncFail(hr))));
            }
            release_ref(op as *mut c_void);
        }
    }
    OpHandle { op }
}
