//! Task abstraction for the dispatcher queue runtime.
//!
//! Everything here is single-threaded (`!Send`): tasks are owned locally on
//! the DQ thread as `Rc`s, with no atomics. Cross-thread wakeups are handled
//! entirely by `dq::waker`, which reaches back into this module only through
//! the `Waker` handed to [`TaskObj::poll_task`].

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use crate::dq::RuntimeInner;
use crate::dq::waker::{WakerData, make_waker};

/// Type-erased, pollable task as stored in `RuntimeInner::tasks`.
pub(crate) trait TaskObj {
    /// Polls the task once with the given waker. Returns `true` if the task
    /// reached a terminal state (finished, panicked, or aborted) this call,
    /// in which case the caller (`RuntimeInner::drain`) removes it from the
    /// task map.
    fn poll_task(&self, waker: &Waker) -> bool;
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

/// Shared result slot between a `LocalTask<F>` and its `LocalJoinHandle<T>`.
///
/// `LocalTask` is stored type-erased as `Rc<dyn TaskObj>` inside
/// `RuntimeInner`, so `LocalJoinHandle<T>` cannot reach `F::Output` through
/// it; this small, separately-owned cell is the concretely-typed channel
/// between the two.
pub(crate) struct JoinShared<T> {
    slot: Option<Result<T, JoinError>>,
    waker: Option<Waker>,
    /// One-way terminal flag: set to `true` exactly when `set()` stores a
    /// result, and never reset. `slot` itself is emptied by `take()` once
    /// the `LocalJoinHandle` consumes the value, so `is_finished()` must
    /// read this instead of `slot.is_some()` — otherwise a handle that has
    /// already been awaited to completion would wrongly report `false`.
    finished: bool,
}

impl<T> JoinShared<T> {
    pub(crate) fn new() -> Self {
        Self {
            slot: None,
            waker: None,
            finished: false,
        }
    }

    /// Stores the final result and wakes anyone polling the join handle.
    pub(crate) fn set(&mut self, result: Result<T, JoinError>) {
        self.slot = Some(result);
        self.finished = true;
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    /// Takes the result if present (consumes it — a `LocalJoinHandle` only
    /// ever observes it once, on the poll that returns `Ready`).
    pub(crate) fn take(&mut self) -> Option<Result<T, JoinError>> {
        self.slot.take()
    }

    /// Sticky once true: reflects whether `set()` has ever been called,
    /// regardless of whether `take()` has since emptied `slot`.
    pub(crate) fn is_finished(&self) -> bool {
        self.finished
    }

    /// Registers the waker to be notified when `set` is next called.
    pub(crate) fn register(&mut self, waker: &Waker) {
        self.waker = Some(waker.clone());
    }
}

/// Lifecycle of a locally-owned future.
enum Stage<F: Future> {
    Running(F),
    /// Terminal: the future finished (or panicked); the actual output (or
    /// panic payload) has already been handed off to the paired
    /// `JoinShared`, since `Stage` lives behind a type-erased `Rc<dyn
    /// TaskObj>` and can't hold `F::Output` for anyone outside this module
    /// to retrieve.
    Finished,
    /// Terminal: aborted before completing.
    Aborted,
}

/// A locally-spawned (`!Send`) future, polled from `RuntimeInner::drain`.
pub(crate) struct LocalTask<F: Future> {
    stage: RefCell<Stage<F>>,
    /// Shared with the `LocalJoinHandle`; checked at the start of every poll.
    abort_requested: Rc<Cell<bool>>,
    shared: Rc<RefCell<JoinShared<F::Output>>>,
}

impl<F> TaskObj for LocalTask<F>
where
    F: Future,
{
    fn poll_task(&self, waker: &Waker) -> bool {
        if self.abort_requested.get() {
            // Overwriting `Stage::Running(fut)` here drops the user's
            // future, which can panic; and `shared.set(...)` below can run
            // an arbitrary registered join `Waker::wake()`, which can also
            // panic. Neither may unwind across the DQ FFI boundary (this is
            // ultimately invoked from an `unsafe extern "system"`
            // `DispatcherQueueHandler::Invoke` with no panic protection of
            // its own), so both are caught and swallowed — the task is
            // already terminating, so there is nothing left to report the
            // panic to. Mirrors `tp::task::TaskCore::finish`.
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let mut stage = self.stage.borrow_mut();
                if matches!(&*stage, Stage::Running(_)) {
                    *stage = Stage::Aborted;
                }
                drop(stage);
            }));
            let _ = catch_unwind(AssertUnwindSafe(|| {
                self.shared.borrow_mut().set(Err(JoinError::Aborted));
            }));
            return true;
        }

        let outcome = {
            let mut stage = self.stage.borrow_mut();
            match &mut *stage {
                Stage::Running(fut) => {
                    let mut cx = Context::from_waker(waker);
                    // SAFETY: `fut` lives inside this `Rc<LocalTask<F>>`
                    // allocation and is moved only by replacing the whole
                    // `Stage` enum on a terminal transition (never while
                    // `Running`), so treating it as pinned in place between
                    // polls is sound.
                    Some(catch_unwind(AssertUnwindSafe(|| unsafe {
                        Pin::new_unchecked(fut).poll(&mut cx)
                    })))
                }
                // Already terminal (e.g. raced with an abort request that
                // was applied by a previous drain pass in the same batch).
                _ => None,
            }
        };

        match outcome {
            None => true,
            Some(Ok(Poll::Pending)) => false,
            Some(Ok(Poll::Ready(value))) => {
                // Same rationale as the abort branch above: overwriting
                // `Stage::Running(fut)` drops the (already-completed)
                // future, and `shared.set(...)` can run an arbitrary join
                // waker — neither may unwind across the FFI boundary. The
                // future's own poll panic (captured above by the outer
                // `catch_unwind`) is unaffected and still delivered as
                // `JoinError::Panicked` via `shared.set` in the branch below.
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    *self.stage.borrow_mut() = Stage::Finished;
                }));
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    self.shared.borrow_mut().set(Ok(value));
                }));
                true
            }
            Some(Err(payload)) => {
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    *self.stage.borrow_mut() = Stage::Finished;
                }));
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    self.shared
                        .borrow_mut()
                        .set(Err(JoinError::Panicked(payload)));
                }));
                true
            }
        }
    }
}

/// Owned handle to a locally-spawned task. Await it for the output;
/// dropping it detaches the task (it keeps running — `RuntimeInner::tasks`
/// holds its own strong reference regardless of this handle).
pub struct LocalJoinHandle<T> {
    shared: Rc<RefCell<JoinShared<T>>>,
    abort_requested: Rc<Cell<bool>>,
    id: u64,
    rt: Weak<RuntimeInner>,
}

impl<T> LocalJoinHandle<T> {
    /// Requests cancellation. If the task is currently idle (not already
    /// scheduled), this also wakes it so the abort is applied on the next
    /// drain; if a poll is already in flight this drain pass, the abort
    /// still takes effect the next time the task is polled. Awaiting the
    /// handle then yields `Err(JoinError::Aborted)` unless the task won the
    /// race by completing first.
    pub fn abort(&self) {
        self.abort_requested.set(true);
        if let Some(rt) = self.rt.upgrade() {
            let data = Arc::new(WakerData {
                queue: rt.queue.clone(),
                ready: rt.ready.clone(),
                id: self.id,
            });
            make_waker(data).wake();
        }
    }

    /// Whether the task has reached a terminal state (its result, if any,
    /// is available to this handle).
    pub fn is_finished(&self) -> bool {
        self.shared.borrow().is_finished()
    }
}

impl<T> Future for LocalJoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut shared = self.shared.borrow_mut();
        if let Some(result) = shared.take() {
            return Poll::Ready(result);
        }
        shared.register(cx.waker());
        Poll::Pending
    }
}

/// Allocates a `LocalTask<F>` plus its paired `LocalJoinHandle`. Does not
/// itself register the task with a `RuntimeInner` — see
/// `RuntimeInner::spawn`, which owns id allocation and scheduling.
pub(crate) fn new_task<F>(
    rt: &Rc<RuntimeInner>,
    id: u64,
    fut: F,
) -> (Rc<LocalTask<F>>, LocalJoinHandle<F::Output>)
where
    F: Future,
{
    let abort_requested = Rc::new(Cell::new(false));
    let shared = Rc::new(RefCell::new(JoinShared::new()));
    let task = Rc::new(LocalTask {
        stage: RefCell::new(Stage::Running(fut)),
        abort_requested: abort_requested.clone(),
        shared: shared.clone(),
    });
    let handle = LocalJoinHandle {
        shared,
        abort_requested,
        id,
        rt: Rc::downgrade(rt),
    };
    (task, handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn join_shared_delivers_value() {
        let shared = std::rc::Rc::new(std::cell::RefCell::new(JoinShared::<i32>::new()));
        shared.borrow_mut().set(Ok(5));
        assert!(matches!(shared.borrow_mut().take(), Some(Ok(5))));
    }

    /// `is_finished()` must stay `true` after `take()` empties `slot` — it
    /// is a one-way terminal flag, not a proxy for "has an unread result".
    /// Regression test for the bug where `is_finished()` read
    /// `slot.is_some()` directly and went stale (back to `false`) the
    /// moment a `LocalJoinHandle` consumed its result via `take()`.
    #[test]
    fn join_shared_is_finished_stays_true_after_take() {
        let shared = JoinShared::<i32>::new();
        let shared = std::cell::RefCell::new(shared);

        assert!(!shared.borrow().is_finished());

        shared.borrow_mut().set(Ok(7));
        assert!(shared.borrow().is_finished());

        // Consuming the result (as `LocalJoinHandle::poll` does on its
        // `Ready` poll) must not un-finish the task.
        assert!(matches!(shared.borrow_mut().take(), Some(Ok(7))));
        assert!(shared.borrow().is_finished());

        // A second `take()` correctly finds nothing left, but `is_finished`
        // still reports the sticky terminal state.
        assert!(shared.borrow_mut().take().is_none());
        assert!(shared.borrow().is_finished());
    }
}
