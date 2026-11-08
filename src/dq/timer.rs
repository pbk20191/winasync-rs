//! `sleep`/`interval` built on WinRT `DispatcherQueueTimer`.
//!
//! Shape mirrors `tp::timer` (a `fired`/tick-count `Waker` cell polled by a
//! `Sleep`/`Interval` future), but the backend is a `DispatcherQueueTimer`
//! instead of a TP_TIMER, and — unlike `tp::timer`'s callback, which runs on
//! an arbitrary pool thread and so uses a `Mutex` — this one only ever fires
//! on the single DQ thread that created it.
//!
//! That thread-affinity does *not* let the `Tick` handler capture the shared
//! state directly, though. `DispatcherQueueTimer::Tick` takes a
//! `TypedEventHandler<DispatcherQueueTimer, IInspectable>`, and
//! `TypedEventHandler::new` (see `windows-0.62.2`'s
//! `Windows::Foundation::TypedEventHandler::new`) requires its closure to be
//! `Fn(..) -> windows_core::Result<()> + Send + 'static` — the constructor
//! itself demands `Send`, even though nothing here is ever actually invoked
//! off the DQ thread. An `Rc<RefCell<..>>` is `!Send`, so it cannot be
//! captured by that closure directly.
//!
//! The fallback (per the task brief): give every timer a `u64` id (trivially
//! `Send`) and stash its shared state in a `thread_local!` map keyed by that
//! id. The `Tick` closure captures only the id; when it fires, it looks the
//! state up in the thread-local. This is sound because the lookup always
//! happens on the same DQ thread that inserted the entry — dispatcher-queue
//! timers, like the queue itself, only ever fire on their owning thread — so
//! there is no actual cross-thread access to the `Rc`, only a `Send`-shaped
//! capture list satisfying the delegate constructor.
//!
//! `Sleep`/`Interval` remove their id from the thread-local map (and call
//! `Stop()`) on `Drop`, so the map never grows past the number of
//! currently-live timers.

use crate::dq::with_current;
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows_core::IInspectable;
use winui3::Microsoft::UI::Dispatching::DispatcherQueueTimer;

/// Converts a `Duration` into the `TimeSpan` (100ns ticks) `SetInterval`
/// expects, clamped to at least 1 tick — a zero-length interval is not a
/// meaningful "fire immediately" request to the underlying timer.
fn to_timespan(dur: Duration) -> TimeSpan {
    let ticks = (dur.as_nanos() / 100).clamp(1, i64::MAX as u128) as i64;
    TimeSpan { Duration: ticks }
}

/// Process-wide unique ids for timers registered in `TIMERS`. A plain
/// `AtomicU64` (rather than a thread-local counter) keeps id allocation
/// trivial; only uniqueness matters here, not ordering, and ids are cheap
/// enough (a `u64` never realistically wraps) that a global counter is
/// simpler than threading a thread-local one through `register_timer`.
static NEXT_TIMER_ID: AtomicU64 = AtomicU64::new(0);

fn next_timer_id() -> u64 {
    NEXT_TIMER_ID.fetch_add(1, Ordering::Relaxed)
}

/// Shared state between a timer's `Tick` handler and the `Sleep`/`Tick`
/// future polling it. `ticks` is a monotonically increasing counter rather
/// than a bare `fired` bool so `Interval` can reuse the same shape: a
/// `Sleep`'s timer is non-repeating and only ever reaches 1, while an
/// `Interval`'s keeps counting so missed ticks accumulate instead of being
/// lost (mirrors `tp::timer::TimerState`).
struct TimerState {
    ticks: u64,
    waker: Option<Waker>,
}

impl TimerState {
    fn new() -> Self {
        Self {
            ticks: 0,
            waker: None,
        }
    }
}

thread_local! {
    /// DQ-thread-local map from timer id -> shared tick state; see the
    /// module doc comment for why this indirection exists.
    static TIMERS: RefCell<HashMap<u64, Rc<RefCell<TimerState>>>> =
        RefCell::new(HashMap::new());
}

/// Allocates a fresh id and shared state, and registers the pair in `TIMERS`
/// so the (`Send`-only-capturing) `Tick` handler can find it later.
fn register_timer() -> (u64, Rc<RefCell<TimerState>>) {
    let id = next_timer_id();
    let state = Rc::new(RefCell::new(TimerState::new()));
    TIMERS.with(|t| {
        t.borrow_mut().insert(id, state.clone());
    });
    (id, state)
}

/// Removes a timer's shared state from the thread-local map. Must run when
/// the owning `Sleep`/`Interval` is dropped (see both types' `Drop` impls),
/// or every timer ever created would linger in the map for the DQ thread's
/// entire lifetime.
fn unregister_timer(id: u64) {
    TIMERS.with(|t| {
        t.borrow_mut().remove(&id);
    });
}

/// Bumps the tick counter for `id` and wakes whoever is waiting, if anyone.
/// If `id` is no longer registered (the `Sleep`/`Interval` was already
/// dropped, racing a tick that was already queued before `Stop()` took
/// effect), this is a harmless no-op.
///
/// Invoked from the `Tick` delegate, which runs on the DQ thread but through
/// an `unsafe extern "system"` WinRT `Invoke` with no panic protection of its
/// own — `waker.wake()` can run arbitrary combinator code, so it's wrapped in
/// `catch_unwind` rather than allowed to unwind across that FFI boundary.
fn on_tick(id: u64) {
    // The whole body — thread-local lookup, `ticks` increment, and the wake
    // itself — runs inside a single `catch_unwind`. Previously only
    // `waker.wake()` was guarded; the lookup/increment ran outside it, so a
    // panic there (e.g. a poisoned/misbehaving `Waker` impl, or a future
    // change to this body) would unwind straight out of `on_tick` and across
    // the WinRT `Tick` delegate's `unsafe extern "system"` FFI boundary,
    // aborting the process. Widening the guard to cover the full body closes
    // that gap symmetrically with `drain_ready` and `LocalTask::poll_task`.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let waker = TIMERS.with(|t| {
            t.borrow().get(&id).map(|state| {
                let mut state = state.borrow_mut();
                state.ticks += 1;
                state.waker.take()
            })
        });
        if let Some(Some(waker)) = waker {
            waker.wake();
        }
    }));
}

/// Creates a `DispatcherQueueTimer` on the current dq runtime's queue and
/// wires its `Tick` event to `on_tick(id)`. Panics (via `with_current`) if
/// called outside a running `DispatcherRuntime` — same precondition as
/// `spawn_local`.
fn make_timer(id: u64) -> DispatcherQueueTimer {
    let timer =
        with_current(|rt| rt.queue.CreateTimer()).expect("failed to create DispatcherQueueTimer");
    let _ = timer.Tick(&TypedEventHandler::<DispatcherQueueTimer, IInspectable>::new(
        move |_sender, _args| {
            on_tick(id);
            Ok(())
        },
    ));
    timer
}

/// Future returned by [`sleep`]; resolves once the due time has passed.
pub struct Sleep {
    id: u64,
    timer: DispatcherQueueTimer,
    state: Rc<RefCell<TimerState>>,
}

/// Suspends the current task for at least `dur`. Must be called (and
/// awaited) on a thread running a `DispatcherRuntime`.
pub fn sleep(dur: Duration) -> Sleep {
    let (id, state) = register_timer();
    let timer = make_timer(id);
    // Best-effort: these calls only fail for reasons (e.g. the queue
    // shutting down) that also mean the timer will never usefully fire
    // regardless, matching the fire-and-forget style of `RuntimeInner::spawn`
    // and `WakerData::schedule` elsewhere in this module.
    let _ = timer.SetInterval(to_timespan(dur));
    let _ = timer.SetIsRepeating(false);
    let _ = timer.Start();
    Sleep { id, timer, state }
}

impl Future for Sleep {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        let mut state = this.state.borrow_mut();
        if state.ticks > 0 {
            Poll::Ready(())
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        // Stop the timer first (best-effort — a failure here just means it
        // was already stopped or the queue is going away) so no further
        // ticks are queued, then drop this id's shared state.
        let _ = self.timer.Stop();
        unregister_timer(self.id);
    }
}

/// Error returned by [`timeout`] when the deadline passes first.
#[derive(Debug, PartialEq, Eq)]
pub struct Elapsed;

/// Future returned by [`timeout`].
pub struct Timeout<F> {
    fut: F,
    sleep: Sleep,
}

impl<F: Future> Future for Timeout<F> {
    type Output = Result<F::Output, Elapsed>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Structural pinning of `fut`; `sleep` is Unpin (no self-references).
        let this = unsafe { self.get_unchecked_mut() };
        if let Poll::Ready(v) = unsafe { Pin::new_unchecked(&mut this.fut) }.poll(cx) {
            return Poll::Ready(Ok(v));
        }
        match Pin::new(&mut this.sleep).poll(cx) {
            // Dropping `Timeout` afterwards drops `fut`, cancelling any pending
            // work it owns.
            Poll::Ready(()) => Poll::Ready(Err(Elapsed)),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Awaits `fut` for at most `dur`; on expiry resolves to `Err(Elapsed)`. Must
/// be called (and awaited) on a thread running a `DispatcherRuntime`.
pub fn timeout<F: Future>(dur: Duration, fut: F) -> Timeout<F> {
    Timeout {
        fut,
        sleep: sleep(dur),
    }
}

/// A periodic timer. Each [`tick`][Interval::tick] resolves on the next
/// firing; missed firings are not lost (they accumulate as pending ticks).
pub struct Interval {
    id: u64,
    timer: DispatcherQueueTimer,
    state: Rc<RefCell<TimerState>>,
    consumed: u64,
}

/// Creates an interval whose first tick fires after one full `period`. Must
/// be called on a thread running a `DispatcherRuntime`.
pub fn interval(period: Duration) -> Interval {
    let (id, state) = register_timer();
    let timer = make_timer(id);
    let _ = timer.SetInterval(to_timespan(period));
    let _ = timer.SetIsRepeating(true);
    let _ = timer.Start();
    Interval {
        id,
        timer,
        state,
        consumed: 0,
    }
}

impl Interval {
    /// Waits for the next periodic firing.
    pub fn tick(&mut self) -> Tick<'_> {
        Tick(self)
    }
}

impl Drop for Interval {
    fn drop(&mut self) {
        let _ = self.timer.Stop();
        unregister_timer(self.id);
    }
}

/// Future returned by [`Interval::tick`].
pub struct Tick<'a>(&'a mut Interval);

impl Future for Tick<'_> {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let interval = &mut *self.0;
        let mut state = interval.state.borrow_mut();
        if state.ticks > interval.consumed {
            interval.consumed += 1;
            Poll::Ready(())
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dq::DispatcherRuntime;
    use std::time::Instant;

    #[test]
    fn sleep_waits() {
        crate::dq::ensure_sdk();
        let _g = crate::dq::dq_test_guard();
        let elapsed = DispatcherRuntime::run(async {
            let start = Instant::now();
            sleep(Duration::from_millis(50)).await;
            start.elapsed()
        });
        assert!(elapsed >= Duration::from_millis(40), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    }

    #[test]
    fn interval_ticks() {
        crate::dq::ensure_sdk();
        let _g = crate::dq::dq_test_guard();
        let elapsed = DispatcherRuntime::run(async {
            let start = Instant::now();
            let mut iv = interval(Duration::from_millis(20));
            for _ in 0..3 {
                iv.tick().await;
            }
            start.elapsed()
        });
        assert!(elapsed >= Duration::from_millis(50), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    }

    #[test]
    fn timeout_both_arms() {
        crate::dq::ensure_sdk();
        let _g = crate::dq::dq_test_guard();
        DispatcherRuntime::run(async {
            // Slow future loses to a short deadline.
            let slow = sleep(Duration::from_millis(200));
            assert_eq!(timeout(Duration::from_millis(50), slow).await, Err(Elapsed));
            // Fast future beats a long deadline.
            let fast = sleep(Duration::from_millis(20));
            assert_eq!(timeout(Duration::from_millis(500), fast).await, Ok(()));
        });
    }

    /// Dropping a `Sleep` before it ever fires must not panic or hang a
    /// subsequent run — regression coverage for the `unregister_timer` +
    /// `Stop()` cleanup in `Sleep::drop`.
    #[test]
    fn drop_before_fire() {
        crate::dq::ensure_sdk();
        let _g = crate::dq::dq_test_guard();
        DispatcherRuntime::run(async {
            for _ in 0..100 {
                drop(sleep(Duration::from_secs(10)));
            }
        });
    }
}
