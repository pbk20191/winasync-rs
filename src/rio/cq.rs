//! The completion queue and its notification pump.
//!
//! RIO does not deliver completions; it accumulates them in a queue the
//! application drains. The pump here turns that into the push model the rest
//! of this crate uses, over a Win32 thread pool — `tp`'s if it is enabled, the process default one otherwise:
//!
//! ```text
//!   RIONotify(cq)                 arm one notification
//!   SetThreadpoolWait(wait, ev)   immediately after — see the ordering note
//!     -> callback on a pool thread
//!        RIODequeueCompletion     drain in batches until empty
//!        dispatch each result
//!        RIONotify + SetThreadpoolWait again
//! ```
//!
//! # Three things that are easy to get wrong
//!
//! **Ordering.** Microsoft is explicit that `SetThreadpoolWait` must
//! *immediately follow* `RIONotify`, not precede it: with `NotifyReset`, the
//! event is cleared by the next `RIONotify` rather than by being signalled,
//! so arming the wait first can latch an already-signalled event and fire a
//! spurious callback.
//!
//! **Single consumer.** `RIO_CQ` has no internal synchronization —
//! `RIODequeueCompletion` is effectively single-consumer. That holds here
//! because the TP_WAIT is one-shot: exactly one callback is outstanding at a
//! time, and the next notification is armed only at the end of a drain.
//!
//! **`RIO_CORRUPT_CQ`.** The dequeue reports corruption as `0xFFFFFFFF` in the
//! same `u32` as the result count, so it has to be checked explicitly or it
//! reads as four billion results.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Networking::WinSock::{
    RIORESULT, RIO_CORRUPT_CQ, RIO_CQ, RIO_EVENT_COMPLETION, RIO_IOCP_COMPLETION,
    RIO_NOTIFICATION_COMPLETION, RIO_NOTIFICATION_COMPLETION_0,
    RIO_NOTIFICATION_COMPLETION_0_0, RIO_NOTIFICATION_COMPLETION_0_1,
};
use windows::Win32::System::Threading::{
    CallbackMayRunLong, CloseThreadpoolWait, CreateEventW, CreateThreadpoolWait,
    SetThreadpoolWait, WaitForThreadpoolWaitCallbacks, PTP_CALLBACK_INSTANCE, PTP_WAIT,
    TP_CALLBACK_ENVIRON_V3,
};

use super::buffer::RioSlice;
use super::rq::RIO_INVALID_CQ;
use super::table::rio_table;
use crate::common::winsock::wsa_last_err;
use crate::common::tpctx::CtxHeader;
#[cfg(feature = "tp")]
use crate::tp::Runtime;

/// How many completions one dequeue call collects. Batching is the point of a
/// completion queue — a larger batch amortises the call over more results.
const DRAIN_BATCH: usize = 128;

/// What a finished operation hands back: the raw status (0 = success, else a
/// winsock error), the transferred byte count, and the buffer the operation
/// held.
///
/// The slice travels through the completion rather than being released by the
/// pump, so the caller gets its buffer back at exactly the moment the kernel
/// is provably done with it.
pub(crate) type CompleteFn = Box<dyn FnOnce(i32, u32, Option<RioSlice>) + Send>;

/// One in-flight operation's state, reached from the completion's
/// `RequestContext`.
///
/// It owns the [`RioSlice`] for the operation's whole life. That is not a
/// convenience: RIO cannot cancel, and completions still arrive after
/// `closesocket`, so the buffer has to stay registered until the result
/// actually appears in the queue.
pub(crate) struct Request {
    pub(crate) complete: CompleteFn,
    pub(crate) slice: Option<RioSlice>,
}

impl Request {
    /// Leaks the request into a `RequestContext` for RIO to carry.
    pub(crate) fn into_context(self) -> *const core::ffi::c_void {
        Box::into_raw(Box::new(self)) as *const core::ffi::c_void
    }

    /// Reclaims a request the queue handed back. Every posted operation
    /// completes exactly once — including with `WSAECONNABORTED` when the
    /// socket is closed under it — so this runs once per `into_context`.
    unsafe fn from_context(ctx: u64) -> Option<Box<Request>> {
        if ctx == 0 {
            return None;
        }
        Some(unsafe { Box::from_raw(ctx as *mut Request) })
    }
}

/// The queue itself. Nothing here is specific to how it gets drained — the
/// pool-driven pump's own state lives in [`PoolPump`]. Only that struct's
/// optional `RuntimeInner` anchor is `tp`-specific; the pump itself runs on
/// either pool.
struct CqState {
    cq: RIO_CQ,
    /// Whether a `RIONotify` is outstanding; `RIONotify` answers
    /// `WSAEALREADY` if one already is.
    armed: AtomicBool,
    /// Set at teardown so the pump stops re-arming.
    closing: AtomicBool,
    /// Which mechanism delivers notifications; decides whether `RIONotify`
    /// applies at all and whether this crate owns a pump.
    mode: Mode,
    /// Win32 error from whatever stopped the pump, or 0 while healthy.
    ///
    /// A pool-driven queue has no caller loop to return an error to, so a
    /// failed re-arm or a corrupt queue would otherwise be invisible: every
    /// outstanding and future operation just never completes. Recording it
    /// gives [`RioCq::pump_error`] something to report.
    pump_error: AtomicU32,
}

/// How a completion queue announces that it has results.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// An event plus a TP_WAIT; the pump is ours. Which pool it runs on is a
    /// property of the callback environment, not of this queue's behaviour,
    /// so one variant covers both.
    Pump,
    /// An event the caller waits on with its own mechanism.
    Event,
    /// An IOCP the caller owns. The caller recognises the packet and calls
    /// [`RioCq::drain`] then [`RioCq::notify`] itself.
    Iocp,
    /// No notification at all; the caller polls [`RioCq::drain`].
    Polling,
}

// RIO_CQ and the event are kernel handles shared with the pool's threads.
unsafe impl Send for CqState {}
unsafe impl Sync for CqState {}

/// State belonging to the thread-pool pump, and only to it: the event
/// `RIONotify` signals, the TP_WAIT watching it, and the pool that must
/// outlive its own callbacks.
///
/// This is also where the pool's layout contract lives. Its callback
/// environment installs a `FinalizationCallback` that treats every context as
/// beginning with a [`CtxHeader`] and calls that header's `release`, so a
/// context without the header has its first field invoked as a function
/// pointer when the callback object is closed. Keeping the header on *this*
/// struct rather than on the queue is what lets the queue exist without `tp`.
#[repr(C)]
struct PoolPump {
    header: CtxHeader,
    state: Arc<CqState>,
    event: HANDLE,
    /// Whatever the environment's owner must outlive its own callbacks —
    /// `tp`'s `RuntimeInner` for a private pool, nothing for the process
    /// default one. Type-erased exactly as `IoReg::register` does it, so this
    /// struct has one layout regardless of which features are on.
    _keepalive: Option<Arc<dyn core::any::Any + Send + Sync>>,
}

unsafe impl Send for PoolPump {}
unsafe impl Sync for PoolPump {}

/// The single release path for a pump context, run by the environment's
/// finalization callback when the TP_WAIT is closed.
unsafe fn release_pump(ctx: *mut core::ffi::c_void) {
    #[cfg(debug_assertions)]
    PUMP_RELEASES.fetch_add(1, Ordering::Relaxed);
    unsafe { drop(Arc::from_raw(ctx as *const PoolPump)) }
}

/// Pump contexts released, by either path (debug builds).
///
/// Exists to pin down *when* the environment's finalization callback runs:
/// `Drop`'s correctness argument assumes `CloseThreadpoolWait` has already
/// released the context by the time it returns.
#[cfg(debug_assertions)]
pub(crate) static PUMP_RELEASES: AtomicUsize = AtomicUsize::new(0);

/// A RIO completion queue.
///
/// # Driving it
///
/// RIO is a submission engine, not a scheduler: something has to notice that
/// the queue has results and drain it. This type supports the three ways
/// Windows offers, because a RIO queue is rarely the only engine in a
/// process:
///
/// * [`new`](Self::new) — an event and a TP_WAIT on `tp`'s configurable
///   pool. The pump is ours; nothing else is required of the caller.
/// * [`with_default_pool`](Self::with_default_pool) — the same pump on the
///   process default pool, so no runtime from this crate is needed.
/// * [`with_iocp`](Self::with_iocp) — **a completion port the caller owns**,
///   which may well belong to another library's runtime. RIO posts one packet
///   per notification carrying the caller's key and OVERLAPPED pointer; on
///   seeing it the caller calls [`drain`](Self::drain) and then
///   [`notify`](Self::notify). This is the interop path: an existing IOCP
///   loop gains RIO sockets without a second set of threads.
/// * [`with_event`](Self::with_event) — **an event the caller waits on**,
///   with whatever it already uses: `WaitForMultipleObjects`, another thread
///   pool, a foreign reactor. Only the queue is ours.
/// * [`polling`](Self::polling) — no notification at all. The caller drains
///   on its own schedule, which is the lowest-latency and highest-CPU option.
///
/// Only [`new`](Self::new) needs `tp`; the rest are plain Winsock and work
/// with the `rio` feature alone. The two pool-driven modes own their
/// consumer, so [`drain`](Self::drain), [`notify`](Self::notify) and
/// [`resize`](Self::resize) are refused on them.
pub struct RioCq {
    state: Arc<CqState>,
    /// The pump this queue owns, if any. Created and torn down as a unit.
    ///
    /// Not behind a mutex: only the constructors and `Drop` touch it, and
    /// `Drop` has `&mut self`. A lock here would add a poison-panic path to a
    /// destructor, where a panic during unwind aborts the process.
    pump: Option<Pump>,
}

/// A TP_WAIT pump owned by this queue.
///
/// The two variants differ in exactly one thing — **who frees the context** —
/// and that is why they are distinct rather than a flag: getting it wrong is
/// a double free one way and a leak the other.
struct Pump {
    wait: PTP_WAIT,
    event: HANDLE,
    /// `Some` exactly when the callback environment is null, since only an
    /// environment carries a `FinalizationCallback` to release the context
    /// for us. Set from `env.is_none()` in one place, so the pairing cannot
    /// drift the way two hand-written literals could.
    ctx_to_release: Option<*const PoolPump>,
}

unsafe impl Send for RioCq {}
unsafe impl Sync for RioCq {}

impl RioCq {
    /// Creates a completion queue of `size` entries and starts its pump.
    ///
    /// `size` caps how many completions can be outstanding across every
    /// socket attached to this queue: RIO requires
    /// `sum(max_outstanding) <= size`, and honouring that is what makes
    /// overflow structurally impossible rather than something to handle.
    #[cfg(feature = "tp")]
    pub fn new(rt: &Runtime, size: u32) -> windows_core::Result<Arc<Self>> {
        Self::with_pump(
            size,
            Some(rt.inner.pool.default_env()),
            Some(rt.inner.clone()),
        )
    }

    /// Creates a queue pumped from the **process default thread pool**.
    ///
    /// The same event + TP_WAIT drain loop as [`new`](Self::new), but on the
    /// pool Windows gives every process, so it needs no runtime from this
    /// crate and is available with the `rio` feature alone. For a queue that
    /// should just work without the caller building a drain loop, this is the
    /// one to reach for.
    ///
    /// # The cost of a shared pool
    ///
    /// That pool is shared with the CRT and with every other library in the
    /// process. Each notification drains the queue until it is empty, so a
    /// sustained flood of completions can hold a default-pool thread for as
    /// long as it lasts. The pump calls `CallbackMayRunLong` on entry, which
    /// asks the pool for a replacement thread immediately rather than after
    /// its starvation detector fires, so other users of the pool are not
    /// stalled behind a long drain — but the threads are still shared, and
    /// this crate cannot size or prioritise them.
    ///
    /// With `tp` enabled, [`new`](Self::new) puts the same pump on a private
    /// pool whose environment can be configured. Under heavy load prefer that,
    /// or drive the queue from a loop you control
    /// ([`with_iocp`](Self::with_iocp) / [`polling`](Self::polling)).
    pub fn with_default_pool(size: u32) -> windows_core::Result<Arc<Self>> {
        Self::with_pump(size, None, None)
    }

    /// The one pump constructor, parameterized by callback environment.
    ///
    /// `env` is the only thing that differs between the pools, and it decides
    /// the one rule teardown must get right: an environment carries a
    /// `FinalizationCallback` that releases the context, a null environment
    /// does not. Deriving `ctx_to_release` from `env.is_none()` right here
    /// makes that pairing structural — the alternative, two near-identical
    /// constructors each setting both by hand, is a double free if they drift
    /// one way and a leak the other.
    ///
    /// `keepalive` is whatever must outlive the pool's callbacks; `None` for
    /// the process default pool, which outlives everything.
    ///
    /// # Safety
    /// `env`, when given, must point at a callback environment that stays
    /// valid for this queue's life — which `keepalive` is there to ensure.
    fn with_pump(
        size: u32,
        env: Option<*const TP_CALLBACK_ENVIRON_V3>,
        keepalive: Option<Arc<dyn core::any::Any + Send + Sync>>,
    ) -> windows_core::Result<Arc<Self>> {
        // Manual-reset: with NotifyReset the event is cleared by the next
        // RIONotify, not by the signal.
        let event = unsafe { CreateEventW(None, true, false, None)? };
        let notify = RIO_NOTIFICATION_COMPLETION {
            Type: RIO_EVENT_COMPLETION,
            Anonymous: RIO_NOTIFICATION_COMPLETION_0 {
                Event: RIO_NOTIFICATION_COMPLETION_0_0 {
                    EventHandle: event,
                    NotifyReset: true.into(),
                },
            },
        };
        let cq = match create_cq(size, Some(&notify)) {
            Ok(cq) => cq,
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(event);
                }
                return Err(e);
            }
        };

        let state = Arc::new(CqState {
            cq,
            armed: AtomicBool::new(false),
            closing: AtomicBool::new(false),
            pump_error: AtomicU32::new(0),
            mode: Mode::Pump,
        });
        let pump = Arc::new(PoolPump {
            header: unsafe { CtxHeader::new(release_pump) },
            state: state.clone(),
            event,
            _keepalive: keepalive,
        });
        let ctx = Arc::into_raw(pump);

        let wait = match unsafe {
            CreateThreadpoolWait(Some(pump_callback), Some(ctx as *mut core::ffi::c_void), env)
        } {
            Ok(w) => w,
            Err(e) => {
                // Hand the leaked Arc back before unwinding the construction.
                unsafe { drop(Arc::from_raw(ctx)) };
                close_cq(cq);
                unsafe {
                    let _ = CloseHandle(event);
                }
                return Err(e);
            }
        };

        let this = Arc::new(RioCq {
            state: state.clone(),
            pump: Some(Pump {
                wait,
                event,
                // Only a null environment leaves the context to us.
                ctx_to_release: env.is_none().then_some(ctx),
            }),
        });
        // Arm for the first time: notify, then wait, in that order.
        arm(&state, event, wait)?;
        Ok(this)
    }

    /// Creates a queue that notifies **an IOCP the caller owns**.
    ///
    /// RIO posts one completion packet per notification, carrying `key` as
    /// the completion key and `overlapped` as the `OVERLAPPED*`, with a byte
    /// count of zero. The pointer is a tag only — RIO never writes through it
    /// — but it must be non-null and must stay valid and unique for this
    /// queue's life, since it is how the caller tells this queue's packets
    /// from its own IO.
    ///
    /// On receiving that packet the caller drains and re-arms:
    ///
    /// ```ignore
    /// // inside the caller's existing GetQueuedCompletionStatus loop
    /// if overlapped == my_rio_tag {
    ///     cq.drain();
    ///     cq.notify()?;          // arm the next notification
    /// }
    /// ```
    ///
    /// Nothing about the caller's IOCP is assumed beyond its handle, so the
    /// port may belong to another runtime entirely.
    ///
    /// # Safety
    /// `iocp` must be a valid completion port handle and `overlapped` a
    /// pointer that outlives this queue and is not used for any other
    /// operation.
    pub unsafe fn with_iocp(
        iocp: HANDLE,
        key: usize,
        overlapped: *mut core::ffi::c_void,
        size: u32,
    ) -> windows_core::Result<Arc<Self>> {
        assert!(!overlapped.is_null(), "RIO_IOCP_COMPLETION requires a non-null OVERLAPPED tag");
        let notify = RIO_NOTIFICATION_COMPLETION {
            Type: RIO_IOCP_COMPLETION,
            Anonymous: RIO_NOTIFICATION_COMPLETION_0 {
                Iocp: RIO_NOTIFICATION_COMPLETION_0_1 {
                    IocpHandle: iocp,
                    CompletionKey: key as *mut core::ffi::c_void,
                    Overlapped: overlapped,
                },
            },
        };
        let cq = create_cq(size, Some(&notify))?;
        let this = Self::externally_driven(cq, Mode::Iocp);
        // Arm the first notification here rather than making the caller
        // remember to: an unarmed queue is silent, which looks like a
        // delivery failure rather than a missing call.
        notify_state(&this.state)?;
        Ok(this)
    }

    /// Creates a queue that signals **an event the caller waits on**.
    ///
    /// For a process that already has a waiting mechanism — its own thread
    /// pool, a `WaitForMultipleObjects` loop, a foreign reactor. On being
    /// signalled the caller drains and re-arms, exactly as in
    /// [`with_iocp`](Self::with_iocp).
    ///
    /// `notify_reset` is RIO's `NotifyReset`, and it does **not** mean what
    /// its name suggests:
    ///
    /// * `true` — RIO clears the event on the **next `RIONotify`**, not when
    ///   it signals. So the event stays signalled after a notification until
    ///   the caller re-arms, and any wait armed before that `RIONotify` can
    ///   fire spuriously. This is why Microsoft's thread-pool recipe puts
    ///   `SetThreadpoolWait` *after* `RIONotify`; the same ordering applies
    ///   to any wait the caller arms.
    /// * `false` — RIO never touches the event; resetting it is the caller's
    ///   job. Correct for an auto-reset event, which clears itself on the
    ///   wait that observes it.
    ///
    /// # Safety
    /// `event` must be a valid event handle that outlives this queue, and
    /// must not be used for anything else while the queue holds it.
    pub unsafe fn with_event(
        event: HANDLE,
        notify_reset: bool,
        size: u32,
    ) -> windows_core::Result<Arc<Self>> {
        let notify = RIO_NOTIFICATION_COMPLETION {
            Type: RIO_EVENT_COMPLETION,
            Anonymous: RIO_NOTIFICATION_COMPLETION_0 {
                Event: RIO_NOTIFICATION_COMPLETION_0_0 {
                    EventHandle: event,
                    NotifyReset: notify_reset.into(),
                },
            },
        };
        let cq = create_cq(size, Some(&notify))?;
        let this = Self::externally_driven(cq, Mode::Event);
        // Armed here for the same reason as the IOCP mode: an unarmed queue
        // is silent, which reads as a delivery failure rather than a missing
        // call.
        notify_state(&this.state)?;
        Ok(this)
    }

    /// Creates a queue with no notification: the caller polls
    /// [`drain`](Self::drain).
    ///
    /// `RIONotify` is invalid on such a queue — [`notify`](Self::notify) is a
    /// no-op here rather than an error, so one drain-and-rearm loop can serve
    /// every mode.
    pub fn polling(size: u32) -> windows_core::Result<Arc<Self>> {
        let cq = create_cq(size, None)?;
        Ok(Self::externally_driven(cq, Mode::Polling))
    }

    fn externally_driven(cq: RIO_CQ, mode: Mode) -> Arc<Self> {
        Arc::new(RioCq {
            state: Arc::new(CqState {
                cq,
                armed: AtomicBool::new(false),
                closing: AtomicBool::new(false),
                pump_error: AtomicU32::new(0),
                mode,
            }),
            pump: None,
        })
    }

    /// Drains every available completion, dispatching each to its waiter.
    ///
    /// Returns how many were dispatched, or `None` if the provider reported
    /// `RIO_CORRUPT_CQ`. Only for the externally driven modes; a pool-driven
    /// queue drains itself, and a second consumer would break RIO's
    /// single-consumer rule for `RIODequeueCompletion`.
    pub fn drain(&self) -> Option<usize> {
        if self.state.is_self_pumping() {
            // Not a debug_assert: in release this would put two threads in
            // RIODequeueCompletion on an unsynchronized queue, hand the same
            // RequestContext to Box::from_raw twice, and double-free a
            // Request along with the RioSlice it holds. Refuse instead.
            return Some(0);
        }
        drain(&self.state)
    }

    /// The error that stopped this queue's pump, if one did.
    ///
    /// Only a pool-driven queue can need this: the externally driven modes
    /// return their errors from [`drain`](Self::drain) and
    /// [`notify`](Self::notify) directly. A non-`None` result here means no
    /// further completion will ever be dispatched, so every outstanding
    /// future is dead — a condition that is otherwise indistinguishable from
    /// a quiet network.
    pub fn pump_error(&self) -> Option<windows_core::Error> {
        match self.state.pump_error.load(Ordering::Acquire) {
            0 => None,
            code => Some(windows_core::Error::from_hresult(
                windows::Win32::Foundation::WIN32_ERROR(code).to_hresult(),
            )),
        }
    }

    /// Arms the next notification (`RIONotify`).
    ///
    /// Call it after each [`drain`](Self::drain); RIO notifies once per arm.
    /// A no-op in polling mode, where there is nothing to arm.
    pub fn notify(&self) -> windows_core::Result<()> {
        // Nothing to arm when polling, and nothing for a caller to arm on a
        // queue this crate pumps — re-arming under the pump would race its
        // own RIONotify.
        if self.state.mode == Mode::Polling || self.state.is_self_pumping() {
            return Ok(());
        }
        notify_state(&self.state)
    }

    /// Resizes the completion queue.
    ///
    /// Must stay at or above the sum of every attached socket's outstanding
    /// budget — that invariant is what makes the queue impossible to
    /// overflow — so grow this before growing a request queue, and shrink it
    /// only after.
    pub fn resize(&self, size: u32) -> windows_core::Result<()> {
        if self.state.is_self_pumping() {
            // Resizing reindexes the queue under whoever is dequeuing, and
            // the pump may be mid-drain on another thread right now. Neither
            // queue kind is internally synchronized, so refuse rather than
            // corrupt.
            return Err(windows_core::Error::from_hresult(
                windows::Win32::Foundation::WIN32_ERROR(WSAEINVAL).to_hresult(),
            ));
        }
        let table = rio_table()?;
        let ok = unsafe { (table.raw().RIOResizeCompletionQueue.unwrap())(self.state.cq, size) };
        if ok.as_bool() {
            Ok(())
        } else {
            Err(wsa_last_err())
        }
    }

    pub(crate) fn raw(&self) -> RIO_CQ {
        self.state.cq
    }
}

impl CqState {
    /// Whether this crate owns the pump, i.e. whether a caller touching the
    /// queue directly would be a second consumer.
    fn is_self_pumping(&self) -> bool {
        matches!(self.mode, Mode::Pump)
    }

    /// First error wins: later failures are consequences of the first.
    fn record_pump_error(&self, code: u32) {
        let _ = self
            .pump_error
            .compare_exchange(0, code, Ordering::AcqRel, Ordering::Acquire);
    }
}

/// `RIOCreateCompletionQueue`, with `None` meaning a polling queue.
fn create_cq(
    size: u32,
    notify: Option<&RIO_NOTIFICATION_COMPLETION>,
) -> windows_core::Result<RIO_CQ> {
    let table = rio_table()?;
    let cq = unsafe {
        (table.raw().RIOCreateCompletionQueue.unwrap())(
            size,
            notify.map_or(core::ptr::null(), |n| n as *const _),
        )
    };
    if cq == RIO_INVALID_CQ {
        return Err(wsa_last_err());
    }
    Ok(cq)
}

/// Dispatches whatever is still in the queue before it is destroyed.
///
/// Sockets hold an `Arc<RioCq>`, so they close first and this runs moments
/// later — while the `WSAECONNABORTED` completions their `closesocket` just
/// produced are still arriving. Throwing the queue away without dequeuing
/// them leaks every outstanding `Request`: its `RioSlice`, and through that
/// the `Arc<RioBufferPool>`, so `RIODeregisterBuffer` never runs and the
/// slab stays page-locked for the life of the process. The waiting
/// `oneshot::Sender`s would never drop either, leaving any still-held
/// receiver pending forever instead of resolving.
///
/// Bounded: the kernel queues those completions promptly, and a teardown is
/// not a place to wait indefinitely for a peer.
fn drain_before_close(state: &CqState) {
    const MAX_IDLE_ROUNDS: u32 = 50;
    let mut idle = 0;
    while idle < MAX_IDLE_ROUNDS {
        match drain(state) {
            Some(0) => {
                idle += 1;
                std::thread::yield_now();
            }
            // Progress: keep going without counting against the budget.
            Some(_) => idle = 0,
            // Corrupt queue — nothing more can be dequeued from it.
            None => return,
        }
    }
}

fn close_cq(cq: RIO_CQ) {
    if let Ok(table) = rio_table() {
        unsafe { (table.raw().RIOCloseCompletionQueue.unwrap())(cq) };
    }
}

/// Arms one notification. Skipped while a previous one is outstanding, since
/// `RIONotify` answers `WSAEALREADY` in that case.
fn notify_state(state: &CqState) -> windows_core::Result<()> {
    if state.closing.load(Ordering::Acquire) {
        return Ok(());
    }
    let table = rio_table()?;
    if !state.armed.swap(true, Ordering::AcqRel) {
        let r = unsafe { (table.raw().RIONotify.unwrap())(state.cq) };
        if r != 0 {
            state.armed.store(false, Ordering::Release);
            return Err(windows_core::Error::from_hresult(
                windows::Win32::Foundation::WIN32_ERROR(r as u32).to_hresult(),
            ));
        }
    }
    Ok(())
}

/// `RIONotify` followed immediately by `SetThreadpoolWait`, per Microsoft's
/// documented pattern — pool mode only.
fn arm(state: &Arc<CqState>, event: HANDLE, wait: PTP_WAIT) -> windows_core::Result<()> {
    if state.closing.load(Ordering::Acquire) {
        return Ok(());
    }
    notify_state(state)?;
    // Must follow RIONotify: with NotifyReset the event is cleared by that
    // call, so arming first could latch a stale signal.
    unsafe { SetThreadpoolWait(wait, Some(event), None) };
    Ok(())
}

/// Drains every available completion, dispatching each to its request.
///
/// Returns the number dispatched, or `None` if the queue reported
/// `RIO_CORRUPT_CQ` — in which case the pump stops rather than looping on a
/// queue the provider has declared unusable.
fn drain(state: &CqState) -> Option<usize> {
    let table = rio_table().ok()?;
    let mut total = 0usize;
    let mut results = [RIORESULT::default(); DRAIN_BATCH];
    loop {
        let n = unsafe {
            (table.raw().RIODequeueCompletion.unwrap())(
                state.cq,
                results.as_mut_ptr(),
                DRAIN_BATCH as u32,
            )
        };
        if n == RIO_CORRUPT_CQ {
            return None;
        }
        if n == 0 {
            return Some(total);
        }
        for r in &results[..n as usize] {
            if let Some(req) = unsafe { Request::from_context(r.RequestContext) } {
                let Request { complete, slice } = *req;
                // Hand the buffer on with the result: now, and only now, is
                // the kernel provably finished with it.
                let (status, bytes) = (r.Status, r.BytesTransferred);
                let _ = catch_unwind(AssertUnwindSafe(move || complete(status, bytes, slice)));
            }
        }
        total += n as usize;
        if (n as usize) < DRAIN_BATCH {
            return Some(total);
        }
    }
}

/// TP_WAIT callback: one at a time, which is what keeps the single-consumer
/// rule for `RIODequeueCompletion`.
/// Clears [`IN_PUMP`] however the callback leaves, including by unwind.
struct RestoreInPump;
impl Drop for RestoreInPump {
    fn drop(&mut self) {
        IN_PUMP.with(|f| f.set(false));
    }
}

unsafe extern "system" fn pump_callback(
    instance: PTP_CALLBACK_INSTANCE,
    ctx: *mut core::ffi::c_void,
    wait: PTP_WAIT,
    _wait_result: u32,
) {
    // The drain runs until the queue is empty, which under a sustained
    // producer is unbounded. Tell the pool so it can add a thread rather than
    // waiting out its ~500ms starvation detector — this is per-callback and
    // needs no callback environment, so it works on the default pool too.
    // FALSE only means the pool already has a replacement thread ready.
    let _ = unsafe { CallbackMayRunLong(instance) };
    IN_PUMP.with(|f| f.set(true));
    let _restore = RestoreInPump;
    // Borrow the context without consuming the reference the TP_WAIT holds.
    let pump = unsafe { Arc::from_raw(ctx as *const PoolPump) };
    let pump = core::mem::ManuallyDrop::new(pump);
    let state = &pump.state;

    // The notification has fired; the next RIONotify will clear the event.
    state.armed.store(false, Ordering::Release);

    let drained = catch_unwind(AssertUnwindSafe(|| drain(state)));
    match drained {
        Ok(Some(_)) => {
            // A failed re-arm is terminal for a pool-driven queue: nothing
            // will notify again. Record it so the owner can see why its
            // futures stopped resolving.
            if let Err(e) = arm(state, pump.event, wait) {
                state.record_pump_error(e.code().0 as u32);
            }
        }
        // The queue reported RIO_CORRUPT_CQ, the function table went away, or
        // the drain panicked. Stop pumping rather than spin; outstanding
        // requests will never complete, so say so rather than going quiet.
        Ok(None) => state.record_pump_error(WSAECONNRESET),
        Err(_) => state.record_pump_error(WSAECONNRESET),
    }
}

/// Stand-in win32 code for "this queue is finished"; RIO reports corruption
/// as a sentinel rather than an error code.
const WSAECONNRESET: u32 = 10054;
const WSAEINVAL: u32 = 10022;

thread_local! {
    /// Set while this thread is inside [`pump_callback`].
    ///
    /// A completion dispatched by the drain runs its waiter's waker inline,
    /// and that waker may poll a task that drops the last socket — and with
    /// it the last `Arc<RioCq>`. `Drop` would then wait for the very callback
    /// it is running inside, which is the deadlock
    /// `crate::tp` invariant 6 forbids. This flag lets teardown notice and
    /// step aside instead.
    static IN_PUMP: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// Teardown moved off the pump thread; see the `IN_PUMP` check in
/// [`RioCq::drop`].
struct Teardown {
    pump: Pump,
    cq: RIO_CQ,
}

// Every field is a kernel handle or a raw pointer this value now owns
// exclusively.
unsafe impl Send for Teardown {}

impl Teardown {
    fn run(self) {
        unsafe {
            SetThreadpoolWait(self.pump.wait, None, None);
            WaitForThreadpoolWaitCallbacks(self.pump.wait, true);
            SetThreadpoolWait(self.pump.wait, None, None);
            WaitForThreadpoolWaitCallbacks(self.pump.wait, true);
            CloseThreadpoolWait(self.pump.wait);
        }
        if let Some(ctx) = self.pump.ctx_to_release {
            unsafe { drop(Arc::from_raw(ctx)) };
        }
        close_cq(self.cq);
        unsafe {
            let _ = CloseHandle(self.pump.event);
        }
    }
}

impl Drop for RioCq {
    fn drop(&mut self) {
        self.state.closing.store(true, Ordering::Release);

        let pump = self.pump.take();
        let Some(pump) = pump else {
            // Externally driven: no pump of ours, and no context was handed
            // to a callback object.
            drain_before_close(&self.state);
            close_cq(self.state.cq);
            return;
        };

        if IN_PUMP.with(|f| f.get()) {
            // We are *inside* this queue's own pump callback: a completion we
            // just dispatched dropped the last owner. Waiting for the callback
            // to finish would be waiting for ourselves, so hand the teardown
            // to a thread that is not this one. The callback returns moments
            // later and the wait there completes immediately.
            let salvage = Teardown {
                pump,
                cq: self.state.cq,
            };
            std::thread::spawn(move || salvage.run());
            return;
        }

        unsafe {
            // Cancel the pending wait, then let any running callback finish.
            SetThreadpoolWait(pump.wait, None, None);
            WaitForThreadpoolWaitCallbacks(pump.wait, true);
            // A callback that started before `closing` was set may have
            // re-armed on its way out; cancel that too before closing the
            // wait object, which must not be pending.
            SetThreadpoolWait(pump.wait, None, None);
            WaitForThreadpoolWaitCallbacks(pump.wait, true);
            CloseThreadpoolWait(pump.wait);
        }

        // On an environment with a finalization callback the close above
        // already released the context — measured, not assumed: see
        // `finalization_timing::close_threadpool_wait_releases_the_context_before_returning`,
        // which fails loudly if that ever becomes deferred. A null
        // environment has no such callback, so the reference is ours to drop,
        // and only now, with every callback provably finished.
        if let Some(ctx) = pump.ctx_to_release {
            unsafe { drop(Arc::from_raw(ctx)) };
        }

        drain_before_close(&self.state);

        // Close the queue before the event it notifies through.
        close_cq(self.state.cq);
        unsafe {
            let _ = CloseHandle(pump.event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::buffer::RioBufferPool;
    use super::super::rq::RqConfig;
    use super::super::socket::RioUdpSocket;
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    use windows::Win32::System::IO::{
        CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED,
    };

    fn v4() -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
    }

    /// RIO driven by a completion port this crate does not own and knows
    /// nothing about — the interop path for an existing IOCP runtime.
    ///
    /// No thread pool, no TP_WAIT: the loop below is all a foreign runtime
    /// would need to add.
    #[test]
    fn rio_driven_by_a_foreign_iocp() {
        // A bare completion port, as any other library would have.
        let iocp = unsafe {
            CreateIoCompletionPort(windows::Win32::Foundation::INVALID_HANDLE_VALUE, None, 0, 1)
        }
        .expect("iocp");

        // The tag RIO stamps on its notifications. Boxed so it has a stable,
        // unique address for the queue's whole life.
        let tag: Box<OVERLAPPED> = Box::new(unsafe { core::mem::zeroed() });
        let tag_ptr = Box::into_raw(tag);
        const RIO_KEY: usize = 0x5249_4F00; // 'RIO' + 0 — any value the caller picks

        let cq = unsafe {
            RioCq::with_iocp(iocp, RIO_KEY, tag_ptr as *mut core::ffi::c_void, 64)
        }
        .expect("iocp-driven cq");

        let pool = RioBufferPool::new(512, 4).expect("pool");
        // sum(max_outstanding) must not exceed the queue size, and RIO
        // enforces it: two sockets at 8+8 fit a 64-entry queue with room to
        // spare, whereas RqConfig::default() (64+64 each) would be refused
        // with WSAENOBUFS at bind.
        let rq = RqConfig {
            max_outstanding_recv: 8,
            max_outstanding_send: 8,
        };
        let server = RioUdpSocket::bind(&cq, v4(), rq).expect("server");
        let client = RioUdpSocket::bind(&cq, v4(), rq).expect("client");
        client.connect(server.local_addr().unwrap()).unwrap();

        let rx = server.recv(pool.acquire().unwrap()).expect("post recv");
        let mut out = pool.acquire().unwrap();
        out[..4].copy_from_slice(b"iocp");
        out.set_len(4);
        let tx = client.send(out).expect("post send");

        // The foreign runtime's loop: wait on its own port, recognise RIO's
        // packet by the tag, drain, re-arm.
        let mut drained = 0usize;
        for _ in 0..8 {
            let mut bytes = 0u32;
            let mut key = 0usize;
            let mut ovl: *mut OVERLAPPED = core::ptr::null_mut();
            let ok = unsafe {
                GetQueuedCompletionStatus(iocp, &mut bytes, &mut key, &mut ovl, 2000)
            };
            if ok.is_err() {
                break;
            }
            assert_eq!(key, RIO_KEY, "RIO stamped the key we gave it");
            assert_eq!(ovl, tag_ptr, "and the OVERLAPPED tag we gave it");
            assert_eq!(bytes, 0, "a notification carries no byte count");
            drained += cq.drain().expect("queue not corrupt");
            cq.notify().expect("re-arm");
            if drained >= 2 {
                break;
            }
        }
        assert_eq!(drained, 2, "both the send and the receive completed");

        let (sent, _) = block_on_ready(tx);
        assert_eq!(sent.unwrap(), 4);
        let (got, slice) = block_on_ready(rx);
        let n = got.unwrap();
        assert_eq!(&slice[..n], b"iocp");

        drop(server);
        drop(client);
        drop(cq);
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(iocp);
            drop(Box::from_raw(tag_ptr));
        }
    }

    /// Polls a future that is already complete. The completions were
    /// dispatched by the drain above, so no executor is involved.
    fn block_on_ready<F: Future>(fut: F) -> F::Output {
        crate::common::block_on(fut)
    }
}

#[cfg(all(test, feature = "tp", debug_assertions))]
mod finalization_timing {
    use super::*;
    use crate::tp::Runtime;

    /// Is the context released by the time `CloseThreadpoolWait` returns?
    ///
    /// `Drop` assumes so. If it were deferred, the `tp`-environment path
    /// would release the pump — and whatever keepalive it holds — later, on a
    /// pool thread, which is how a pool ends up being torn down from inside
    /// itself.
    #[test]
    fn close_threadpool_wait_releases_the_context_before_returning() {
        let rt = Runtime::new();
        let before = PUMP_RELEASES.load(Ordering::Relaxed);
        let cq = RioCq::new(&rt, 64).expect("cq");
        drop(cq);
        let immediately = PUMP_RELEASES.load(Ordering::Relaxed);

        // Give a deferred release every chance, so a failure here says
        // "deferred", not "slow".
        let mut eventually = immediately;
        for _ in 0..200 {
            if eventually > before {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            eventually = PUMP_RELEASES.load(Ordering::Relaxed);
        }

        assert!(
            eventually > before,
            "the pump context was never released at all"
        );
        assert_eq!(
            immediately, eventually,
            "finalization is DEFERRED: the context outlived CloseThreadpoolWait,              so Drop must not assume the keepalive is already gone"
        );
    }
}
