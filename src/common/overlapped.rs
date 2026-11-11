//! Unified overlapped-IO core shared by `tp`, `comio` and `http`.
//!
//! One registration + op-block + completion engine, parameterized by a
//! [`Binding`]: a thread pool's `TP_IO` -- `tp`'s runtime pool, comio's
//! environment, the process default -- or a caller's completion port. The
//! binding carries what keeps its pool or port valid, and a registration
//! drops it only after its `TP_IO` and handle are closed.
//!
//! The primitive is completion-callback based (a closure stored in the op
//! block + [`OpHandle`] with explicit `cancel`), which `comio` uses directly. `tp` layers its
//! `Future`/drop-cancels API on top (see `tp::io`). The op block is generic over
//! a payload `P` that the kernel may write into (buffers); it lives in the block
//! until completion. Invariants preserved from the two cores this replaces:
//! refcount starts at 2 (caller + kernel), freed exactly once; `StartThreadpoolIo`
//! balanced 1:1 by a completion or `CancelThreadpoolIo` on sync failure; no panic
//! crosses the completion FFI boundary; `LIVE_OPS` tracks live blocks in test builds and under `counters`.

use core::ffi::c_void;
use std::cell::UnsafeCell;
use std::mem;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use crate::common::pool::PoolRef;
use std::os::windows::io::{AsRawHandle, AsRawSocket, FromRawHandle, FromRawSocket, OwnedHandle, OwnedSocket};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Networking::WinSock::SOCKET;
use windows::Win32::Storage::FileSystem::SetFileCompletionNotificationModes;
use windows::Win32::System::Threading::{
    CancelThreadpoolIo, CloseThreadpoolIo, CreateThreadpoolIo, StartThreadpoolIo,
    PTP_CALLBACK_INSTANCE, PTP_IO,
};
use windows::Win32::System::IO::{CancelIoEx, CreateIoCompletionPort, GetOverlappedResult, OVERLAPPED};

/// Number of live op blocks -- leak tests assert this settles back to 0, and
/// `comio::unload::can_unload` refuses to unload the module while it is
/// nonzero.
///
/// **The static does not exist** without `counters` (which `dylib` pulls in)
/// or `cfg(test)`. It was unconditional until 2026-11-09, and before that
/// debug-only on the reasoning that a release build should not pay an atomic
/// per operation for a diagnostic. Both were half right. It stopped being
/// only a diagnostic when `can_unload` started reading it -- an op block holds
/// a completion callback that is a function pointer into the module, so a
/// count pinned at 0 would say the module may unload with I/O in flight --
/// but this module is on the submission path of `comio`, `rio` and `tp`
/// alike, and only a `dylib` build has that question to answer.
///
/// So the condition names its readers rather than a profile. A build with
/// neither carries no counter, no atomic, and no process-wide mutable state
/// it has no use for.
#[cfg(any(test, feature = "counters"))]
pub static LIVE_OPS: AtomicUsize = AtomicUsize::new(0);

/// Ops finished inline because their handle had opted out of the completion
/// packet. Without this a skip-mode handle whose flag silently failed to take
/// effect would behave identically -- correct, just not faster -- so tests
/// assert this actually moves.
///
/// Purely a diagnostic, unlike [`LIVE_OPS`]: nothing in the library reads it.
/// Same gate, minus the reason `LIVE_OPS` has for being on in a `dylib`
/// build.
#[cfg(any(test, feature = "counters"))]
pub static INLINE_COMPLETIONS: AtomicUsize = AtomicUsize::new(0);

/// A kernel object to register: a handle or a socket, owned, so it is closed
/// however registration ends -- by the registration when it succeeds, on the
/// way out when it fails.
pub enum IoHandle {
    Handle(OwnedHandle),
    Socket(OwnedSocket),
    /// A named-pipe instance: closes its handle, then releases the count it
    /// holds -- which must not fall while the instance is still open.
    #[cfg(any(feature = "tp", feature = "comio"))]
    PipeInstance(crate::common::pipe::Instance),
}

impl From<OwnedHandle> for IoHandle {
    fn from(h: OwnedHandle) -> Self {
        IoHandle::Handle(h)
    }
}

impl From<OwnedSocket> for IoHandle {
    fn from(s: OwnedSocket) -> Self {
        IoHandle::Socket(s)
    }
}

impl IoHandle {
    /// Takes ownership of a raw handle.
    ///
    /// # Safety
    /// `handle` is open, owned by the caller, and closed by nothing else.
    pub unsafe fn from_handle(handle: HANDLE) -> Self {
        IoHandle::Handle(unsafe { OwnedHandle::from_raw_handle(handle.0) })
    }

    /// Takes ownership of a raw socket.
    ///
    /// # Safety
    /// As [`from_handle`](Self::from_handle), for a socket.
    pub unsafe fn from_socket(socket: SOCKET) -> Self {
        IoHandle::Socket(unsafe { OwnedSocket::from_raw_socket(socket.0 as u64) })
    }

    fn raw(&self) -> HANDLE {
        match self {
            IoHandle::Handle(h) => HANDLE(h.as_raw_handle()),
            IoHandle::Socket(s) => HANDLE(s.as_raw_socket() as *mut c_void),
            #[cfg(any(feature = "tp", feature = "comio"))]
            IoHandle::PipeInstance(i) => i.handle,
        }
    }

    fn is_socket(&self) -> bool {
        matches!(self, IoHandle::Socket(_))
    }
}

/// Where a registered handle's completions go.
pub enum Binding {
    /// A thread pool, through a `TP_IO` created in that pool's environment.
    Pool(PoolRef),
    /// A completion port someone else drains, handing each completion to
    /// [`dispatch`].
    Port(PortRef),
}

impl Binding {
    /// The process's default thread pool.
    pub const fn process_default() -> Binding {
        Binding::Pool(PoolRef::ProcessDefault)
    }
}

/// A completion port, held open by every registration on it: a port closed
/// under a pending operation loses its completion, and the operation with
/// it.
pub struct PortRef(Arc<OwnedHandle>);

impl PortRef {
    /// `port` must be a completion port; association with anything else
    /// fails at registration.
    pub fn new(port: Arc<OwnedHandle>) -> PortRef {
        PortRef(port)
    }
}

/// A handle registered with the completion engine: owns the kernel handle,
/// its `TP_IO` when it has one, and what must outlive both.
pub struct IoReg {
    pub handle: HANDLE,
    is_socket: bool,
    /// Set once [`set_skip_completion_on_success`](Self::set_skip_completion_on_success)
    /// succeeds; read by issue closures to decide whether a synchronous success
    /// still queues a completion packet.
    skip_on_success: AtomicBool,
    /// `None` on a [`Binding::Port`]: nothing to arm there, and completions
    /// arrive through [`dispatch`].
    io: Option<PTP_IO>,
    /// Taken in `drop` to close it at the right moment.
    owned: Option<IoHandle>,
    /// Dropped last: the pool or port the `TP_IO` and handle belonged to.
    _binding: Binding,
}

unsafe impl Send for IoReg {}
unsafe impl Sync for IoReg {}

impl IoReg {
    /// Registers `io` with `binding`. Takes `io` either way: on failure it
    /// is closed before this returns.
    pub fn register(io: impl Into<IoHandle>, binding: Binding) -> windows_core::Result<Arc<Self>> {
        let owned = io.into();
        let handle = owned.raw();
        let tp_io = match &binding {
            Binding::Pool(pool) => Some(unsafe { CreateThreadpoolIo(handle, Some(io_complete), None, pool.env())? }),
            Binding::Port(port) => {
                let port = HANDLE(port.0.as_raw_handle());
                unsafe { CreateIoCompletionPort(handle, Some(port), port_key(), 0)? };
                None
            }
        };
        Ok(Arc::new(Self {
            handle,
            is_socket: owned.is_socket(),
            skip_on_success: AtomicBool::new(false),
            io: tp_io,
            owned: Some(owned),
            _binding: binding,
        }))
    }

    pub fn socket(&self) -> SOCKET {
        SOCKET(self.handle.0 as usize)
    }

    /// The `tp` runtime this handle is registered with. Only for handles
    /// `tp` registered, which always are.
    #[cfg(feature = "tp")]
    pub(crate) fn runtime_inner(&self) -> &Arc<crate::tp::RuntimeInner> {
        match &self._binding {
            Binding::Pool(PoolRef::Runtime(rt)) => &rt.inner,
            _ => unreachable!("a tp handle registered elsewhere"),
        }
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
        //
        // The order is the point: the TP_IO, then the handle, then (as the
        // last field) the pool or port.
        if let Some(io) = self.io {
            unsafe { CloseThreadpoolIo(io) };
        }
        drop(self.owned.take());
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
#[derive(Clone, Copy, Debug)]
pub enum Completion {
    /// Delivered by the kernel: raw win32 `ioresult` (0 = success) + `bytes`.
    Done(u32, usize),
    /// Synchronous submit failure, carrying the **exact** `HRESULT` (so callers
    /// like `tp` that preserve it are not downgraded to a lossy win32 code).
    SyncFail(windows_core::HRESULT),
}

/// What the completion path and the caller's side exchange: the outcome,
/// and who to wake when it lands. In the block, so a future over the op
/// needs no allocation of its own.
struct Shared {
    /// Set by whichever completion path runs.
    fired: bool,
    /// The outcome, until [`OpHandle::poll_completion`] takes it.
    result: Option<Completion>,
    /// Registered by a poll that found no result yet.
    waker: Option<Waker>,
}

struct OpHeader {
    /// 2 at issue time: caller (`OpHandle`) + kernel/completion side.
    refs: AtomicUsize,
    shared: Mutex<Shared>,
    /// Runs the block's completion callback: typed, like `release`, so the
    /// callback is stored inline in the block rather than boxed.
    complete: unsafe fn(*mut c_void, Completion),
    /// Typed deallocation of the whole `Op<P, F>` block (header offset is
    /// P-independent thanks to `repr(C)` with OVERLAPPED first).
    release: unsafe fn(*mut c_void),
    /// Keeps the handle + TP_IO alive while the op is in flight.
    reg: Arc<IoReg>,
}

/// Heap op block. `repr(C)` with OVERLAPPED first, so the completion's
/// `overlapped` pointer *is* the block pointer and the header sits at a fixed
/// offset for every `P` and `F`; the callback comes last, so the payload's
/// offset does not depend on it either, and an `Op<P, ()>` is a valid view
/// of the fields before it. The payload owns whatever the kernel may touch;
/// the callback runs exactly once, on the pool thread for a delivered
/// completion, inline for a synchronous failure.
#[repr(C)]
pub struct Op<P, F = ()> {
    overlapped: OVERLAPPED,
    header: OpHeader,
    payload: UnsafeCell<Option<P>>,
    on_complete: UnsafeCell<Option<F>>,
}

/// Test builds only: what every live op block is -- its payload type and
/// where it was submitted -- so a leak check that fails can say *which*
/// op was left behind, not just that one was. `LIVE_OPS` alone could not,
/// and a leak seen once on a CI runner (2026-11-11: one block, never
/// completed, failing every later check) could not be traced.
#[cfg(test)]
pub static LIVE_SITES: Mutex<Vec<(usize, &'static str, &'static core::panic::Location<'static>)>> =
    Mutex::new(Vec::new());

/// Test builds only: the live op blocks, one per line, for a leak
/// assertion's message.
#[cfg(test)]
pub fn live_ops_report() -> String {
    let sites = LIVE_SITES.lock().unwrap();
    if sites.is_empty() {
        return String::from("no live op blocks");
    }
    sites
        .iter()
        .map(|(_, payload, site)| format!("
  live op: {payload} submitted at {site}"))
        .collect()
}

unsafe fn release_op<P, F>(op: *mut c_void) {
    // Release, not Relaxed, for the same reason `Arc` uses it on the way
    // down: a thread that observes this counter at zero and acts on it (see
    // `comio::unload`) must see everything this op did.
    #[cfg(any(test, feature = "counters"))]
    LIVE_OPS.fetch_sub(1, Ordering::Release);
    #[cfg(test)]
    LIVE_SITES.lock().unwrap().retain(|(ptr, _, _)| *ptr != op as usize);
    unsafe { drop(Box::from_raw(op as *mut Op<P, F>)) }
}

unsafe fn release_ref(op: *mut c_void) {
    let header = unsafe { &(*(op as *mut Op<(), ()>)).header };
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
    unsafe { complete(overlapped, ioresult, bytes) }
}

/// Finishes the op whose block starts at `overlapped`: runs its callback and
/// drops the kernel's reference.
///
/// # Safety
/// `overlapped` is an op block's `OVERLAPPED` whose completion was delivered
/// and not finished before.
unsafe fn complete(overlapped: *mut c_void, ioresult: u32, bytes: usize) {
    unsafe { finish(overlapped, Completion::Done(ioresult, bytes)) }
}

/// Finishes the op at `op` with `c`: its callback, then the kernel's
/// reference.
///
/// # Safety
/// `op` is an op block that has not been finished before.
unsafe fn finish(op: *mut c_void, c: Completion) {
    let header = unsafe { &(*(op as *mut Op<(), ()>)).header };
    unsafe {
        (header.complete)(op, c);
        release_ref(op);
    }
}

/// The typed half of [`finish`]: runs the callback, then records the
/// outcome and wakes whoever polls for it.
unsafe fn complete_op<P, F: FnOnce(Completion)>(op: *mut c_void, c: Completion) {
    let op = unsafe { &*(op as *mut Op<P, F>) };
    // Exactly one completion path runs per op, so the callback is this
    // one's to take.
    if let Some(cb) = unsafe { (*op.on_complete.get()).take() } {
        let _ = catch_unwind(AssertUnwindSafe(move || cb(c)));
    }
    let waker = {
        let mut s = op.header.shared.lock().unwrap();
        s.fired = true;
        s.result = Some(c);
        s.waker.take()
    };
    if let Some(w) = waker {
        w.wake();
    }
}

/// The completion key every handle of [`Binding::Port`] is
/// associated under: the address of a private static, so no other code has
/// a reason to hold it. Code that associates or posts under arbitrary keys
/// (the `iocp` module) refuses this one, which is what lets [`dispatch`]
/// trust a completion that carries it.
pub fn port_key() -> usize {
    static ANCHOR: u8 = 0;
    &ANCHOR as *const u8 as usize
}

/// Finishes an operation of a port-associated handle from the completion
/// its port delivered: `key` and `overlapped` as dequeued. Returns `false`,
/// touching nothing, when the key is not [`port_key`] -- the completion is
/// someone else's.
///
/// # Safety
/// A completion carrying [`port_key`] must be one the port delivered for a
/// handle registered through [`Binding::Port`], dispatched once.
/// What the `iocp` module lets safe code put on a port cannot carry the key.
pub unsafe fn dispatch(key: usize, overlapped: *mut OVERLAPPED) -> bool {
    if key != port_key() || overlapped.is_null() {
        return false;
    }
    // The port reports an NTSTATUS; the Win32 code the callbacks expect is
    // what `GetOverlappedResult` makes of it.
    // The byte count is read straight from the block: on a warning such as
    // `ERROR_MORE_DATA` it is still the count the operation reported.
    let header = unsafe { &(*(overlapped as *mut Op<(), ()>)).header };
    let bytes = unsafe { (*overlapped).InternalHigh };
    let mut ignored = 0u32;
    let ioresult = match unsafe { GetOverlappedResult(header.reg.handle, overlapped, &mut ignored, false) } {
        Ok(()) => 0,
        Err(e) => windows::Win32::Foundation::WIN32_ERROR::from_error(&e).map_or(e.code().0 as u32, |w| w.0),
    };
    unsafe { complete(overlapped.cast(), ioresult, bytes) };
    true
}

/// Opaque handle to an in-flight op (for cancellation + payload retrieval).
/// Holds the caller-side ref; dropping it releases that ref.
pub struct OpHandle<P> {
    /// The block's prefix view: everything before the callback, whose type
    /// the handle does not know (see [`Op`]).
    op: *mut Op<P, ()>,
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

    /// The outcome, once: `Pending` registers `cx`'s waker for when it
    /// lands. After `Ready`, the kernel is done with the block and
    /// [`take_payload`](Self::take_payload) is sound.
    pub fn poll_completion(&self, cx: &mut Context<'_>) -> Poll<Completion> {
        let mut s = unsafe { (*self.op).header.shared.lock().unwrap() };
        match s.result.take() {
            Some(c) => Poll::Ready(c),
            None => {
                s.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
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
#[cfg_attr(test, track_caller)]
pub unsafe fn submit<P, F: FnOnce(Completion) + Send + 'static>(
    reg: &Arc<IoReg>,
    payload: P,
    on_complete: F,
    issue: impl FnOnce(*mut OVERLAPPED, &mut P) -> Issue,
) -> OpHandle<P> {
    let op = Box::into_raw(Box::new(Op {
        overlapped: unsafe { mem::zeroed() },
        header: OpHeader {
            refs: AtomicUsize::new(2),
            shared: Mutex::new(Shared {
                fired: false,
                result: None,
                waker: None,
            }),
            complete: complete_op::<P, F>,
            release: release_op::<P, F>,
            reg: reg.clone(),
        },
        payload: UnsafeCell::new(Some(payload)),
        on_complete: UnsafeCell::new(Some(on_complete)),
    }));
    #[cfg(any(test, feature = "counters"))]
    LIVE_OPS.fetch_add(1, Ordering::Relaxed);
    #[cfg(test)]
    LIVE_SITES.lock().unwrap().push((
        op as usize,
        core::any::type_name::<P>(),
        core::panic::Location::caller(),
    ));

    if let Some(io) = reg.io {
        unsafe { StartThreadpoolIo(io) };
    }
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
            #[cfg(any(test, feature = "counters"))]
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
            if let Some(io) = reg.io {
                CancelThreadpoolIo(io);
            }
            finish(op as *mut c_void, completion);
        }
    }
    OpHandle {
        op: op as *mut Op<P, ()>,
    }
}
