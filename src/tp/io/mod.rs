//! TP_IO based overlapped IO: handle registration ([`IoRes`]), the refcounted
//! operation block ([`Op`]) co-owned by the kernel while in flight, and the
//! submit/complete/cancel protocol shared by files and sockets.

pub mod file;
pub mod net;

use super::RuntimeInner;
use core::ffi::c_void;
use std::cell::UnsafeCell;
use std::mem;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WIN32_ERROR};
use windows::Win32::Networking::WinSock::{closesocket, SOCKET};
use windows::Win32::System::Threading::{
    CancelThreadpoolIo, CloseThreadpoolIo, CreateThreadpoolIo, StartThreadpoolIo,
    PTP_CALLBACK_INSTANCE, PTP_IO,
};
use windows::Win32::System::IO::{CancelIoEx, OVERLAPPED};

/// Number of live operation blocks (debug builds) — lets tests assert that
/// cancel storms don't leak.
#[cfg(debug_assertions)]
pub(crate) static LIVE_OPS: AtomicUsize = AtomicUsize::new(0);

/// A handle registered with the pool's TP_IO. Owns both the kernel handle and
/// the TP_IO object.
pub(crate) struct IoRes {
    pub(crate) handle: HANDLE,
    io: PTP_IO,
    is_socket: bool,
    pub(crate) rt: Arc<RuntimeInner>,
}

unsafe impl Send for IoRes {}
unsafe impl Sync for IoRes {}

impl IoRes {
    /// Takes ownership of `handle` on success; on error the caller still owns
    /// (and must close) it.
    pub(crate) fn register(
        rt: &Arc<RuntimeInner>,
        handle: HANDLE,
        is_socket: bool,
    ) -> windows_core::Result<Arc<Self>> {
        let io = unsafe {
            // No per-object context: the per-op OVERLAPPED pointer identifies
            // the operation (finalize_ctx tolerates the null context).
            CreateThreadpoolIo(handle, Some(io_complete), None, Some(rt.pool.default_env()))?
        };
        Ok(Arc::new(Self {
            handle,
            io,
            is_socket,
            rt: rt.clone(),
        }))
    }

    pub(crate) fn socket(&self) -> SOCKET {
        SOCKET(self.handle.0 as usize)
    }
}

impl Drop for IoRes {
    fn drop(&mut self) {
        // Every in-flight op holds an Arc<IoRes> that the completion callback
        // releases only at its tail, so at this point every started IO has had
        // its completion delivered — closing immediately is legal. This may
        // run at the tail of the last completion callback itself, which is
        // exactly why WaitForThreadpoolIoCallbacks is NOT used here (it would
        // self-deadlock).
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

#[derive(Clone, Copy)]
enum Done {
    /// Completion delivered: raw Win32 error (0 = success) and bytes transferred.
    Completed(u32, usize),
    /// The submission failed synchronously; no completion will arrive.
    FailedSync(windows_core::HRESULT),
}

struct OpShared {
    done: Option<Done>,
    waker: Option<Waker>,
}

struct OpHeader {
    /// 2 at issue time: one for the future, one for the kernel/completion side.
    refs: AtomicUsize,
    shared: Mutex<OpShared>,
    /// Typed deallocation of the whole `Op<P>` block.
    release: unsafe fn(*mut c_void),
    /// Keeps the handle and TP_IO alive while the op is in flight.
    res: Arc<IoRes>,
}

/// Heap-allocated operation block. `repr(C)` with the OVERLAPPED first, so the
/// completion callback's `overlapped` pointer *is* the block pointer, and the
/// header sits at the same offset for every `P`. The payload owns everything
/// the kernel may touch (data buffers, WSARecv flags word, AcceptEx address
/// buffer, the accept socket), so freeing the block frees them exactly once.
#[repr(C)]
pub(crate) struct Op<P> {
    overlapped: OVERLAPPED,
    header: OpHeader,
    payload: UnsafeCell<Option<P>>,
}

unsafe fn release_op<P>(op: *mut c_void) {
    #[cfg(debug_assertions)]
    LIVE_OPS.fetch_sub(1, Ordering::Relaxed);
    unsafe { drop(Box::from_raw(op as *mut Op<P>)) }
}

unsafe fn release_op_ref(op: *mut c_void) {
    let header = unsafe { &(*(op as *mut Op<()>)).header };
    if header.refs.fetch_sub(1, Ordering::Release) == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        let release = header.release;
        unsafe { release(op) };
    }
}

/// The single completion callback for every registered handle. Only touches
/// the header (offset-identical across payload types).
unsafe extern "system" fn io_complete(
    _: PTP_CALLBACK_INSTANCE,
    _ctx: *mut c_void,
    overlapped: *mut c_void,
    ioresult: u32,
    bytes: usize,
    _io: PTP_IO,
) {
    let header = unsafe { &(*(overlapped as *mut Op<()>)).header };
    // Status write and waker handoff under one lock: a poll either sees the
    // result or leaves a waker that is woken here — no lost wakeup.
    let waker = {
        let mut shared = header.shared.lock().unwrap();
        shared.done = Some(Done::Completed(ioresult, bytes));
        shared.waker.take()
    };
    if let Some(w) = waker {
        let _ = catch_unwind(AssertUnwindSafe(move || w.wake()));
    }
    // Tail: release the kernel-side reference (may free the block and, if it
    // held the last Arc<IoRes>, close the handle).
    unsafe { release_op_ref(overlapped) };
}

/// Outcome of issuing the overlapped call, as classified by the caller.
pub(crate) enum Issue {
    /// A completion callback will arrive — the call succeeded synchronously
    /// (still queued, since FILE_SKIP_COMPLETION_PORT_ON_SUCCESS is never
    /// set) or reported ERROR_IO_PENDING / WSA_IO_PENDING.
    Pending,
    /// Hard synchronous failure — no completion will arrive.
    Failed(windows_core::HRESULT),
}

/// Allocates an op block, arms the TP_IO (`StartThreadpoolIo` *before*
/// issuing), runs `issue`, and balances with `CancelThreadpoolIo` on hard
/// failure.
///
/// # Safety
/// `issue` must issue exactly one overlapped operation on `res.handle` using
/// the provided OVERLAPPED pointer and classify its outcome truthfully:
/// reporting `Failed` when a completion is still coming leads to
/// use-after-free; reporting `Pending` when none is coming leaks the block
/// and permanently unbalances the TP_IO.
pub(crate) unsafe fn submit<P>(
    res: &Arc<IoRes>,
    payload: P,
    issue: impl FnOnce(*mut OVERLAPPED, &mut P) -> Issue,
) -> OpFuture<P> {
    let op = Box::into_raw(Box::new(Op {
        overlapped: unsafe { mem::zeroed() },
        header: OpHeader {
            refs: AtomicUsize::new(2),
            shared: Mutex::new(OpShared {
                done: None,
                waker: None,
            }),
            release: release_op::<P>,
            res: res.clone(),
        },
        payload: UnsafeCell::new(Some(payload)),
    }));
    #[cfg(debug_assertions)]
    LIVE_OPS.fetch_add(1, Ordering::Relaxed);

    unsafe { StartThreadpoolIo(res.io) };
    let outcome = unsafe {
        let payload = (*(*op).payload.get()).as_mut().unwrap();
        issue(&raw mut (*op).overlapped, payload)
    };
    if let Issue::Failed(hr) = outcome {
        unsafe {
            CancelThreadpoolIo(res.io);
            (*op).header.shared.lock().unwrap().done = Some(Done::FailedSync(hr));
            // No completion will arrive: retire the kernel-side reference.
            release_op_ref(op as *mut c_void);
        }
    }
    OpFuture { op, done: false }
}

/// Future of one overlapped operation; resolves to the transfer result and
/// hands the payload (with its buffers) back to the caller.
pub(crate) struct OpFuture<P> {
    op: *mut Op<P>,
    done: bool,
}

unsafe impl<P: Send> Send for OpFuture<P> {}

impl<P> Future for OpFuture<P> {
    type Output = (windows_core::Result<usize>, P);

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        assert!(!self.done, "OpFuture polled after completion");
        let header = unsafe { &(*self.op).header };
        let done = {
            let mut shared = header.shared.lock().unwrap();
            match shared.done {
                Some(done) => done,
                None => {
                    shared.waker = Some(cx.waker().clone());
                    return Poll::Pending;
                }
            }
        };
        self.done = true;
        // The kernel is finished with the block (completion delivered or the
        // op never started), and the callback only touches the header — the
        // payload is exclusively ours again.
        let payload = unsafe { (*(*self.op).payload.get()).take().unwrap() };
        let result = match done {
            Done::Completed(0, bytes) => Ok(bytes),
            Done::Completed(ioresult, _) => {
                Err(windows_core::Error::from_hresult(WIN32_ERROR(ioresult).to_hresult()))
            }
            Done::FailedSync(hr) => Err(windows_core::Error::from_hresult(hr)),
        };
        unsafe { release_op_ref(self.op as *mut c_void) };
        Poll::Ready((result, payload))
    }
}

impl<P> Drop for OpFuture<P> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let header = unsafe { &(*self.op).header };
        let in_flight = header.shared.lock().unwrap().done.is_none();
        if in_flight {
            // Request cancellation; the completion (ERROR_OPERATION_ABORTED,
            // or success if it raced) still arrives and the callback frees
            // the block together with its buffers. ERROR_NOT_FOUND just means
            // the op already completed.
            unsafe {
                let _ = CancelIoEx(header.res.handle, Some(&(*self.op).overlapped));
            }
        }
        unsafe { release_op_ref(self.op as *mut c_void) };
    }
}
