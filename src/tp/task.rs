//! Future executor: each spawned task owns a `TP_WORK`; its waker submits the
//! work object, and an atomic state machine guarantees at most one submission
//! is outstanding at any time (submitting twice would run the callback
//! concurrently on two pool threads, as submissions are counted).

use super::pool::{CtxHeader, Priority};
use super::{check, Runtime, RuntimeInner};
use core::ffi::c_void;
use std::any::Any;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::mem;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::atomic::{AtomicIsize, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use windows::Win32::System::Threading::{
    CloseThreadpoolWork, CreateThreadpoolWork, SubmitThreadpoolWork, PTP_CALLBACK_INSTANCE,
    PTP_WORK,
};

/// Not scheduled, not running; a waker (or abort) may submit.
const IDLE: u8 = 0;
/// Exactly one `SubmitThreadpoolWork` outstanding.
const SCHEDULED: u8 = 1;
/// The runner is polling on a pool thread.
const RUNNING: u8 = 2;
/// A wake arrived during `RUNNING`; the runner resubmits after the poll.
const NOTIFIED: u8 = 3;
/// Abort requested; the outstanding (or newly submitted) callback cleans up.
const ABORT_REQUESTED: u8 = 4;
/// Terminal: output (or panic payload) stored in `stage`.
const COMPLETE: u8 = 5;
/// Terminal: future dropped without completing.
const ABORTED: u8 = 6;

enum Stage<F: Future> {
    Running(F),
    Finished(std::thread::Result<F::Output>),
    Aborted,
    Consumed,
}

/// Shared task record. The context pointer of the `TP_WORK` is
/// `Arc::into_raw(Arc<TaskCore<F>>)`; `repr(C)` keeps the [`CtxHeader`] first
/// so the pool's finalization callback can release that reference when the
/// work object is finalized.
#[repr(C)]
struct TaskCore<F: Future> {
    header: CtxHeader,
    state: AtomicU8,
    /// `PTP_WORK.0`, written once in `spawn` before the first submission.
    work: AtomicIsize,
    join_waker: Mutex<Option<Waker>>,
    /// Exclusive access rules: the runner touches it only in `RUNNING` (or as
    /// the sole outstanding callback in the abort path); `JoinHandle` reads it
    /// only after observing a terminal state with `Acquire`.
    stage: UnsafeCell<Stage<F>>,
    /// Keeps the pool (and thus the callback environment) alive until the
    /// last task reference is gone.
    _rt: Arc<RuntimeInner>,
}

unsafe impl<F: Future + Send> Send for TaskCore<F> where F::Output: Send {}
unsafe impl<F: Future + Send> Sync for TaskCore<F> where F::Output: Send {}

impl<F> TaskCore<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn work(&self) -> PTP_WORK {
        PTP_WORK(self.work.load(Ordering::Acquire))
    }

    /// Waker path. Submits only on `IDLE -> SCHEDULED`; a wake during a poll
    /// coalesces into `NOTIFIED` which the runner converts into exactly one
    /// resubmission.
    fn schedule(&self) {
        let mut s = self.state.load(Ordering::Acquire);
        loop {
            let (target, submit) = match s {
                IDLE => (SCHEDULED, true),
                RUNNING => (NOTIFIED, false),
                _ => return, // already scheduled/notified, aborting, or terminal
            };
            match self
                .state
                .compare_exchange_weak(s, target, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    if submit {
                        unsafe { SubmitThreadpoolWork(self.work()) };
                    }
                    return;
                }
                Err(actual) => s = actual,
            }
        }
    }

    fn request_abort(&self) {
        let mut s = self.state.load(Ordering::Acquire);
        loop {
            match s {
                // Parked: mark and submit so cleanup runs on a pool thread.
                // The submission is the transition out of IDLE, preserving
                // the single-submission invariant.
                IDLE => match self.state.compare_exchange_weak(
                    IDLE,
                    ABORT_REQUESTED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        unsafe { SubmitThreadpoolWork(self.work()) };
                        return;
                    }
                    Err(actual) => s = actual,
                },
                // An outstanding or running callback will observe the flag.
                SCHEDULED | RUNNING | NOTIFIED => match self.state.compare_exchange_weak(
                    s,
                    ABORT_REQUESTED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return,
                    Err(actual) => s = actual,
                },
                _ => return, // terminal or already requested
            }
        }
    }

    /// Polls the future. Exclusive access is guaranteed by `RUNNING`.
    unsafe fn poll_stage(&self, cx: &mut Context<'_>) -> Poll<F::Output> {
        match unsafe { &mut *self.stage.get() } {
            // The future lives inside the Arc allocation and is never moved
            // until replaced in a terminal transition, so pinning is sound.
            Stage::Running(f) => unsafe { Pin::new_unchecked(f) }.poll(cx),
            _ => unreachable!("polled a task in a terminal stage"),
        }
    }

    fn waker(&self) -> Waker {
        unsafe { Arc::increment_strong_count(self as *const Self) };
        unsafe {
            Waker::from_raw(RawWaker::new(
                self as *const Self as *const (),
                &WakerVTable::<F>::VTABLE,
            ))
        }
    }

    /// Terminal transition; runs exactly once, on a pool thread, as the sole
    /// outstanding callback. After `CloseThreadpoolWork` the work object is
    /// freed once this callback returns, and the finalization callback then
    /// releases the context reference.
    unsafe fn finish(&self, stage: Stage<F>, terminal: u8) {
        let old = mem::replace(unsafe { &mut *self.stage.get() }, stage);
        // Dropping the user's future (or its output) must not unwind across
        // the FFI boundary.
        let _ = catch_unwind(AssertUnwindSafe(move || drop(old)));

        // Publish the terminal state and take the join waker under the same
        // lock JoinHandle::poll uses, so a concurrent poll either sees the
        // state or leaves a waker we wake here — never a lost wakeup.
        let waker = {
            let mut slot = self.join_waker.lock().unwrap();
            self.state.store(terminal, Ordering::Release);
            slot.take()
        };
        if let Some(w) = waker {
            let _ = catch_unwind(AssertUnwindSafe(move || w.wake()));
        }
        unsafe { CloseThreadpoolWork(self.work()) };
    }
}

struct WakerVTable<F>(PhantomData<F>);

impl<F> WakerVTable<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        |p| {
            unsafe { Arc::increment_strong_count(p as *const TaskCore<F>) };
            RawWaker::new(p, &Self::VTABLE)
        },
        |p| {
            let core = unsafe { Arc::from_raw(p as *const TaskCore<F>) };
            core.schedule();
        },
        |p| unsafe { (*(p as *const TaskCore<F>)).schedule() },
        |p| unsafe { drop(Arc::from_raw(p as *const TaskCore<F>)) },
    );
}

unsafe fn release_task<F>(ctx: *mut c_void)
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    unsafe { drop(Arc::from_raw(ctx as *const TaskCore<F>)) }
}

unsafe extern "system" fn run<F>(_: PTP_CALLBACK_INSTANCE, ctx: *mut c_void, _work: PTP_WORK)
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    // Borrow the context without consuming the work object's strong count;
    // it stays valid for the whole callback because the work object is not
    // finalized before its outstanding callback returns.
    let core = unsafe { &*(ctx as *const TaskCore<F>) };

    if core
        .state
        .compare_exchange(SCHEDULED, RUNNING, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        // Only ABORT_REQUESTED can reach the runner in a non-SCHEDULED state.
        unsafe { core.finish(Stage::Aborted, ABORTED) };
        return;
    }

    let waker = core.waker();
    let poll = catch_unwind(AssertUnwindSafe(|| {
        let mut cx = Context::from_waker(&waker);
        unsafe { core.poll_stage(&mut cx) }
    }));

    match poll {
        Ok(Poll::Pending) => {
            match core
                .state
                .compare_exchange(RUNNING, IDLE, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {} // parked; the next wake resubmits
                Err(NOTIFIED) => {
                    core.state.store(SCHEDULED, Ordering::Release);
                    // Sole outstanding submission: we consumed the previous
                    // one by running.
                    unsafe { SubmitThreadpoolWork(core.work()) };
                }
                Err(ABORT_REQUESTED) => unsafe { core.finish(Stage::Aborted, ABORTED) },
                Err(_) => unreachable!("invalid task state after poll"),
            }
        }
        Ok(Poll::Ready(out)) => unsafe { core.finish(Stage::Finished(Ok(out)), COMPLETE) },
        Err(panic) => unsafe { core.finish(Stage::Finished(Err(panic)), COMPLETE) },
    }
}

pub(crate) fn spawn<F>(rt: &Runtime, fut: F, priority: Option<Priority>) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let core = Arc::new(TaskCore {
        header: unsafe { CtxHeader::new(release_task::<F>) },
        // Starting in SCHEDULED matches the initial submission below and
        // keeps every waker transition uniform.
        state: AtomicU8::new(SCHEDULED),
        work: AtomicIsize::new(0),
        join_waker: Mutex::new(None),
        stage: UnsafeCell::new(Stage::Running(fut)),
        _rt: rt.inner.clone(),
    });

    let ctx = Arc::into_raw(core.clone()) as *mut c_void;
    let env = match priority {
        Some(p) => rt.inner.pool.env(p),
        None => rt.inner.pool.default_env(),
    };
    let work = unsafe { check(CreateThreadpoolWork(Some(run::<F>), Some(ctx), Some(env))) };
    core.work.store(work.0, Ordering::Release);
    unsafe { SubmitThreadpoolWork(work) };

    JoinHandle {
        core: core as Arc<dyn ErasedTask<F::Output>>,
    }
}

/// Type-erased view of a `TaskCore<F>` for `JoinHandle<T>`.
trait ErasedTask<T>: Send + Sync {
    fn state(&self) -> &AtomicU8;
    fn join_slot(&self) -> &Mutex<Option<Waker>>;
    /// Must only be called once, after observing `COMPLETE` with `Acquire`.
    unsafe fn take_output(&self) -> std::thread::Result<T>;
    fn abort(&self);
}

impl<F> ErasedTask<F::Output> for TaskCore<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn state(&self) -> &AtomicU8 {
        &self.state
    }

    fn join_slot(&self) -> &Mutex<Option<Waker>> {
        &self.join_waker
    }

    unsafe fn take_output(&self) -> std::thread::Result<F::Output> {
        // COMPLETE was observed: the runner made its last `stage` write
        // before the Release store, and JoinHandle is the only reader.
        match mem::replace(unsafe { &mut *self.stage.get() }, Stage::Consumed) {
            Stage::Finished(result) => result,
            _ => unreachable!("output taken twice or before completion"),
        }
    }

    fn abort(&self) {
        self.request_abort();
    }
}

/// Why a joined task did not produce its output.
pub enum JoinError {
    /// The task panicked; the payload can be rethrown with
    /// `std::panic::resume_unwind`.
    Panicked(Box<dyn Any + Send + 'static>),
    /// The task was aborted before completing.
    Aborted,
}

impl std::fmt::Debug for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JoinError::Panicked(_) => f.write_str("JoinError::Panicked(..)"),
            JoinError::Aborted => f.write_str("JoinError::Aborted"),
        }
    }
}

/// Owned handle to a spawned task. Await it for the output; dropping it
/// detaches the task (it keeps running and keeps the pool alive).
pub struct JoinHandle<T> {
    core: Arc<dyn ErasedTask<T>>,
}

impl<T> JoinHandle<T> {
    /// Requests cancellation. If the task is parked, its future is dropped on
    /// a pool thread (cancelling any pending IO it owns); if it is mid-poll,
    /// the abort takes effect when the poll returns. Awaiting the handle then
    /// yields `Err(JoinError::Aborted)` — unless the task won the race by
    /// completing first.
    pub fn abort(&self) {
        self.core.abort();
    }

    /// Whether the task has reached a terminal state.
    pub fn is_finished(&self) -> bool {
        matches!(self.core.state().load(Ordering::Acquire), COMPLETE | ABORTED)
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let ready = |core: &Arc<dyn ErasedTask<T>>, state: u8| match state {
            COMPLETE => Some(match unsafe { core.take_output() } {
                Ok(v) => Ok(v),
                Err(payload) => Err(JoinError::Panicked(payload)),
            }),
            ABORTED => Some(Err(JoinError::Aborted)),
            _ => None,
        };

        if let Some(out) = ready(&self.core, self.core.state().load(Ordering::Acquire)) {
            return Poll::Ready(out);
        }
        // Register the waker, then re-check under the same lock the runner
        // holds while publishing the terminal state — no lost wakeup.
        let mut slot = self.core.join_slot().lock().unwrap();
        if let Some(out) = ready(&self.core, self.core.state().load(Ordering::Acquire)) {
            return Poll::Ready(out);
        }
        *slot = Some(cx.waker().clone());
        Poll::Pending
    }
}

#[cfg(test)]
#[path = "task_tests.rs"]
mod tests;
