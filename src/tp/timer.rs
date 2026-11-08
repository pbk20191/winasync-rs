//! TP_TIMER based scheduling: [`sleep`], [`timeout`] and [`Interval`].

use super::pool::CtxHeader;
use super::{check, Runtime, RuntimeInner};
use core::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Threading::{
    CloseThreadpoolTimer, CreateThreadpoolTimer, SetThreadpoolTimer,
    WaitForThreadpoolTimerCallbacks, PTP_CALLBACK_INSTANCE, PTP_TIMER,
};

/// A negative `FILETIME` is a due time relative to now, in 100ns units.
fn relative_due(dur: Duration) -> FILETIME {
    let ticks = (dur.as_nanos() / 100).clamp(1, i64::MAX as u128) as i64;
    let due = -ticks;
    FILETIME {
        dwLowDateTime: due as u32,
        dwHighDateTime: (due >> 32) as u32,
    }
}

struct TimerState {
    /// Number of timer firings not yet consumed by a poll.
    ticks: u64,
    waker: Option<Waker>,
}

/// Context of a TP_TIMER; released by the environment's finalization callback.
#[repr(C)]
struct TimerShared {
    header: CtxHeader,
    state: Mutex<TimerState>,
}

unsafe fn release_timer(ctx: *mut c_void) {
    unsafe { drop(Arc::from_raw(ctx as *const TimerShared)) }
}

unsafe extern "system" fn timer_callback(
    _: PTP_CALLBACK_INSTANCE,
    ctx: *mut c_void,
    _: PTP_TIMER,
) {
    let shared = unsafe { &*(ctx as *const TimerShared) };
    // Tick bump and waker handoff under one lock: a poll either sees the new
    // tick or leaves a waker that we wake here.
    let waker = {
        let mut state = shared.state.lock().unwrap();
        state.ticks += 1;
        state.waker.take()
    };
    if let Some(w) = waker {
        // An arbitrary combinator waker must not unwind across the FFI boundary.
        let _ = catch_unwind(AssertUnwindSafe(move || w.wake()));
    }
}

/// Owns a TP_TIMER and its shared state; common backing for [`Sleep`] and
/// [`Interval`].
struct RawTimer {
    timer: PTP_TIMER,
    shared: Arc<TimerShared>,
    _rt: Arc<RuntimeInner>,
}

// The TP handle is only used with thread-safe pool APIs.
unsafe impl Send for RawTimer {}
unsafe impl Sync for RawTimer {}

impl RawTimer {
    fn new(rt: &Runtime, due: Duration, period_ms: u32) -> Self {
        let shared = Arc::new(TimerShared {
            header: unsafe { CtxHeader::new(release_timer) },
            state: Mutex::new(TimerState {
                ticks: 0,
                waker: None,
            }),
        });
        let ctx = Arc::into_raw(shared.clone());
        let timer = unsafe {
            check(CreateThreadpoolTimer(
                Some(timer_callback),
                Some(ctx as *mut c_void),
                Some(rt.inner.pool.default_env()),
            ))
        };
        unsafe { SetThreadpoolTimer(timer, Some(&relative_due(due)), period_ms, None) };
        Self {
            timer,
            shared,
            _rt: rt.inner.clone(),
        }
    }

    /// Resolves the `n+1`-th tick: `Ready` once `ticks > consumed`.
    fn poll_tick(&self, consumed: &mut u64, cx: &mut Context<'_>) -> Poll<()> {
        let mut state = self.shared.state.lock().unwrap();
        if state.ticks > *consumed {
            *consumed += 1;
            Poll::Ready(())
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl Drop for RawTimer {
    fn drop(&mut self) {
        unsafe {
            // Stop future firings, cancel queued-not-started callbacks, wait
            // for a possibly running one (it is tiny), then close. The context
            // Arc is released by the finalization callback. This never runs
            // inside this timer's own callback, so it cannot self-deadlock.
            SetThreadpoolTimer(self.timer, None, 0, None);
            WaitForThreadpoolTimerCallbacks(self.timer, true);
            CloseThreadpoolTimer(self.timer);
        }
    }
}

/// Future returned by [`sleep`]; resolves once the due time has passed.
pub struct Sleep {
    raw: RawTimer,
    consumed: u64,
}

impl Future for Sleep {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        this.raw.poll_tick(&mut this.consumed, cx)
    }
}

/// Suspends the current task for at least `dur`.
pub fn sleep(rt: &Runtime, dur: Duration) -> Sleep {
    Sleep {
        raw: RawTimer::new(rt, dur, 0),
        consumed: 0,
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
        // Structural pinning of `fut`; `sleep` is Unpin-safe (no self-references).
        let this = unsafe { self.get_unchecked_mut() };
        if let Poll::Ready(v) = unsafe { Pin::new_unchecked(&mut this.fut) }.poll(cx) {
            return Poll::Ready(Ok(v));
        }
        match Pin::new(&mut this.sleep).poll(cx) {
            // Dropping `Timeout` afterwards drops `fut`, cancelling any
            // pending IO it owns.
            Poll::Ready(()) => Poll::Ready(Err(Elapsed)),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Awaits `fut` for at most `dur`; on expiry resolves to `Err(Elapsed)`.
pub fn timeout<F: Future>(rt: &Runtime, dur: Duration, fut: F) -> Timeout<F> {
    Timeout {
        fut,
        sleep: sleep(rt, dur),
    }
}

/// A periodic timer. Each [`tick`][Self::tick] resolves on the next firing;
/// missed firings are not lost (they accumulate as pending ticks).
pub struct Interval {
    raw: RawTimer,
    consumed: u64,
}

/// Creates an interval whose first tick fires after one full `period`.
pub fn interval(rt: &Runtime, period: Duration) -> Interval {
    let period_ms = u32::try_from(period.as_millis().max(1)).expect("interval period too large");
    Interval {
        raw: RawTimer::new(rt, period, period_ms),
        consumed: 0,
    }
}

impl Interval {
    /// Waits for the next periodic firing.
    pub fn tick(&mut self) -> Tick<'_> {
        Tick(self)
    }
}

/// Future returned by [`Interval::tick`].
pub struct Tick<'a>(&'a mut Interval);

impl Future for Tick<'_> {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let interval = &mut *self.0;
        interval.raw.poll_tick(&mut interval.consumed, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{block_on, Runtime};
    use super::*;
    use std::time::Instant;

    #[test]
    fn sleep_waits() {
        let rt = Runtime::new();
        let start = Instant::now();
        block_on(sleep(&rt, Duration::from_millis(50)));
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(40), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
    }

    #[test]
    fn sleep_in_spawned_task() {
        let rt = Runtime::new();
        let rt2 = rt.clone();
        let h = rt.spawn(async move {
            sleep(&rt2, Duration::from_millis(20)).await;
            5
        });
        assert_eq!(block_on(h).unwrap(), 5);
    }

    #[test]
    fn timeout_both_arms() {
        let rt = Runtime::new();
        let slow = sleep(&rt, Duration::from_millis(200));
        assert_eq!(
            block_on(timeout(&rt, Duration::from_millis(50), slow)),
            Err(Elapsed)
        );
        let fast = sleep(&rt, Duration::from_millis(20));
        assert_eq!(block_on(timeout(&rt, Duration::from_millis(500), fast)), Ok(()));
    }

    #[test]
    fn drop_before_fire() {
        let rt = Runtime::new();
        for _ in 0..1000 {
            drop(sleep(&rt, Duration::from_secs(10)));
        }
    }

    #[test]
    fn interval_ticks() {
        let rt = Runtime::new();
        let start = Instant::now();
        block_on(async {
            let mut iv = interval(&rt, Duration::from_millis(20));
            for _ in 0..3 {
                iv.tick().await;
            }
        });
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(50), "{elapsed:?}");
    }
}
