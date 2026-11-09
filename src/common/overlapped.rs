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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Networking::WinSock::{closesocket, SOCKET};
use windows::Win32::Storage::FileSystem::SetFileCompletionNotificationModes;
use windows::Win32::System::Threading::{
    CancelThreadpoolIo, CloseThreadpoolIo, CreateThreadpoolIo, StartThreadpoolIo,
    PTP_CALLBACK_INSTANCE, PTP_IO, TP_CALLBACK_ENVIRON_V3,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

/// Number of live op blocks — leak tests assert this settles back to 0.
///
/// Maintained in every profile. It was debug-only until 2026-11-09, on the
/// reasoning that a release build should not pay an atomic per operation for
/// a diagnostic. The counter stopped being a diagnostic when
/// `comio::unload::can_unload` started reading it: an op block holds a
/// completion callback that is a function pointer into the module, so a count
/// that is pinned at 0 in release would tell `DllCanUnloadNow` it may unload
/// a module with I/O in flight. The cost is one relaxed `fetch_add` against a
/// submission that also performs `StartThreadpoolIo` and a syscall.
///
/// The leak assertions that read it are still `#[cfg(debug_assertions)]`, so
/// they did not change behaviour here — but they could now be ungated and
/// would mean something, which they could not before.
pub static LIVE_OPS: AtomicUsize = AtomicUsize::new(0);

/// Ops finished inline because their handle had opted out of the completion
/// packet. Without this a skip-mode handle whose flag silently failed to take
/// effect would behave identically — correct, just not faster — so tests
/// assert this actually moves.
///
/// Updated in debug builds only, which [`LIVE_OPS`] no longer is: nothing
/// reads this one for correctness, so it keeps the rule they used to share.
/// The assertion that it moves is gated to debug builds to match.
pub static INLINE_COMPLETIONS: AtomicUsize = AtomicUsize::new(0);

/// A handle registered with a pool's TP_IO; owns the kernel handle + TP_IO.
pub struct IoReg {
    pub handle: HANDLE,
    io: PTP_IO,
    is_socket: bool,
    /// Set once [`set_skip_completion_on_success`](Self::set_skip_completion_on_success)
    /// succeeds; read by issue closures to decide whether a synchronous success
    /// still queues a completion packet.
    skip_on_success: AtomicBool,
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
            skip_on_success: AtomicBool::new(false),
            _keepalive: keepalive,
        }))
    }

    pub fn socket(&self) -> SOCKET {
        SOCKET(self.handle.0 as usize)
    }

    pub fn is_socket(&self) -> bool {
        self.is_socket
    }

    /// Stops the kernel from queuing a completion packet when an op on this
    /// handle completes **synchronously** — the hot path for a socket whose
    /// data is already buffered, or a cached file read. The packet, its pool
    /// thread hand-off, and the context switch all disappear; the issuing
    /// thread finishes the op itself.
    ///
    /// Safe to enable on any handle registered here: issue closures report
    /// [`Issue::SyncSuccess`] as a fact and [`submit`] applies this mode, so
    /// there is no per-op contract to get wrong. Still enable it before
    /// issuing ops rather than racing one in flight — an op issued before the
    /// mode is set and completing after it is delivered normally, but there is
    /// nothing to gain from the race.
    ///
    /// Also sets `FILE_SKIP_SET_EVENT_ON_HANDLE`, which is free here: this
    /// engine never waits on the handle's event.
    pub fn set_skip_completion_on_success(&self) -> windows_core::Result<()> {
        /// `FILE_SKIP_COMPLETION_PORT_ON_SUCCESS | FILE_SKIP_SET_EVENT_ON_HANDLE`
        /// from winbase.h — not exposed by the `windows` crate.
        const SKIP_FLAGS: u8 = 0x1 | 0x2;
        unsafe { SetFileCompletionNotificationModes(self.handle, SKIP_FLAGS)? };
        self.skip_on_success.store(true, Ordering::Release);
        Ok(())
    }

    /// Whether [`set_skip_completion_on_success`](Self::set_skip_completion_on_success)
    /// is in effect — issue closures branch on this.
    pub fn skips_completion_on_success(&self) -> bool {
        self.skip_on_success.load(Ordering::Acquire)
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

/// What the overlapped call did, as observed by the issuer.
///
/// These are **facts about the call**, not decisions about what happens next.
/// In particular an issuer never decides whether a completion packet is coming:
/// that depends on whether the handle opted into
/// `FILE_SKIP_COMPLETION_PORT_ON_SUCCESS`, which [`submit`] knows and the issue
/// closure does not need to. Keeping the policy in one place is what makes the
/// skip-completion mode safe to enable on any handle — an issuer that reports
/// truthfully cannot get it wrong.
pub enum Issue {
    /// The call completed **synchronously and successfully**. A completion
    /// packet still follows unless the handle opted out of one.
    SyncSuccess,
    /// The call is in flight (`ERROR_IO_PENDING` / `WSA_IO_PENDING`); a
    /// completion packet will arrive.
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
    // Release, not Relaxed, for the same reason `Arc` uses it on the way
    // down: a thread that observes this counter at zero and acts on it (see
    // `comio::unload`) must see everything this op did.
    LIVE_OPS.fetch_sub(1, Ordering::Release);
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
    LIVE_OPS.fetch_add(1, Ordering::Relaxed);

    unsafe { StartThreadpoolIo(reg.io) };
    let outcome = unsafe {
        let p = (*(*op).payload.get()).as_mut().unwrap();
        issue(&raw mut (*op).overlapped, p)
    };
    // Decide here — not in the issue closure — whether a completion packet is
    // still coming. When none is, the TP_IO must be unarmed and the op
    // finished inline.
    let inline = match outcome {
        Issue::Pending => None,
        // A sync success on an ordinary handle still queues a packet; only a
        // handle that opted out needs finishing here.
        Issue::SyncSuccess if !reg.skips_completion_on_success() => None,
        Issue::SyncSuccess => {
            #[cfg(debug_assertions)]
            INLINE_COMPLETIONS.fetch_add(1, Ordering::Relaxed);
            // The kernel wrote Internal/InternalHigh before returning, so the
            // transferred count is readable without waiting.
            let mut bytes = 0u32;
            let ok = unsafe {
                GetOverlappedResult(reg.handle, &raw const (*op).overlapped, &mut bytes, false)
            };
            Some(Completion::Done(0, if ok.is_ok() { bytes as usize } else { 0 }))
        }
        // Hand the exact HRESULT to the callback (no lossy win32 downgrade
        // here; callers that want one do it themselves).
        Issue::Failed(hr) => Some(Completion::SyncFail(hr)),
    };
    if let Some(completion) = inline {
        unsafe {
            CancelThreadpoolIo(reg.io);
            let cb = {
                let mut s = (*op).header.shared.lock().unwrap();
                s.fired = true;
                s.on_complete.take()
            };
            if let Some(cb) = cb {
                let _ = catch_unwind(AssertUnwindSafe(move || cb(completion)));
            }
            release_ref(op as *mut c_void);
        }
    }
    OpHandle { op }
}
