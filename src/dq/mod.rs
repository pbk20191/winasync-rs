//! Single-threaded async runtime whose executor is a WinRT DispatcherQueue.
//! `!Send` futures run on the DQ thread; a Send waker marshals cross-thread
//! wakes back via TryEnqueue. See docs/superpowers/specs for the design.
#![allow(dead_code)]

pub mod task;
pub mod timer;
pub mod waker;

// Re-exported as this runtime's public timer surface. Nothing in this bin
// crate consumes them yet, hence the allow — they are API, not dead code.
#[allow(unused_imports)]
pub use timer::{interval, sleep, timeout, Elapsed, Interval, Sleep, Tick, Timeout};

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use task::{LocalJoinHandle, TaskObj};
use waker::{make_waker, WakerData};
use winui3::Microsoft::UI::Dispatching::{
    DispatcherQueue, DispatcherQueueController, DispatcherQueueHandler,
};

/// Send + Sync queue of task ids ready to be polled, drained on the DQ thread.
pub(crate) struct ReadyQueue(Mutex<VecDeque<u64>>);

impl ReadyQueue {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(VecDeque::new())))
    }
    pub(crate) fn push(&self, id: u64) {
        self.0.lock().unwrap().push_back(id);
    }
    pub(crate) fn drain_into(&self, out: &mut Vec<u64>) {
        let mut q = self.0.lock().unwrap();
        out.extend(q.drain(..));
    }
}

// The runtime is `!Send`/`!Sync` (its task map holds `Rc`s), so it lives
// behind a thread-local rather than being passed around. Wakers reach it
// purely through this: they only know the DQ thread will eventually run
// `drain_ready`, which looks the runtime up here.
thread_local! {
    static CURRENT: RefCell<Option<Weak<RuntimeInner>>> = const { RefCell::new(None) };
}

thread_local! {
    /// Strong-owner slot for the runtime backing `DispatcherRuntime::run_dedicated`.
    ///
    /// `run_dedicated`'s bootstrap closure runs *once*, on the dedicated DQ
    /// thread, to build the `RuntimeInner`, `set_current` it (a `Weak`, per
    /// above), and `spawn` the root task — then returns. Unlike `run` (where
    /// the caller's own stack frame holds a strong `Rc<RuntimeInner>` alive
    /// across the blocking `RunEventLoop` call), nothing on that dedicated
    /// thread's stack survives past the bootstrap closure returning:
    /// `CURRENT` only holds a `Weak`, and `LocalJoinHandle`/`LocalTask` hold
    /// no strong ref to the runtime either. Without a separate strong owner,
    /// the bootstrap closure returning would drop the runtime's only `Rc` —
    /// tearing down the task map (and the just-spawned root task with it)
    /// before the initial drain it enqueued ever runs. This thread-local is
    /// that owner: the bootstrap closure stores a strong clone here so the
    /// runtime survives until the root task itself clears it on completion
    /// (see `DedicatedRootTask::poll`).
    static DEDICATED_ANCHOR: RefCell<Option<Rc<RuntimeInner>>> = const { RefCell::new(None) };
}

/// Installs `rt` as the runtime reachable from `with_current`/`drain_ready`
/// on the calling (DQ) thread. Used by the `run` loop bootstrap (Task 4).
pub(crate) fn set_current(rt: &Rc<RuntimeInner>) {
    CURRENT.with(|c| *c.borrow_mut() = Some(Rc::downgrade(rt)));
}

pub(crate) fn with_current<R>(f: impl FnOnce(&Rc<RuntimeInner>) -> R) -> R {
    CURRENT.with(|c| {
        let weak = c.borrow().clone().expect("no dq runtime on this thread");
        let rc = weak.upgrade().expect("dq runtime dropped");
        f(&rc)
    })
}

/// Entry point enqueued by `WakerData::schedule`: runs on the DQ thread and
/// drains whatever task ids are ready.
///
/// This is invoked (indirectly, via the `DispatcherQueueHandler` closure) from
/// an `unsafe extern "system" fn` `Invoke` on the WinRT side, which has no
/// panic protection of its own — a panic that unwinds out of this function
/// would unwind across that FFI boundary and abort the process. `LocalTask`'s
/// `poll_task` already guards its own internal panic points (the future's
/// poll, dropping the old `Stage`, and waking the join waker), but this
/// top-level `catch_unwind` is the last line of defense: it also covers
/// `with_current`'s `.expect(...)` calls (which panic if `set_current` was
/// never called, or if the runtime has been dropped) and any future path
/// that might otherwise slip through unguarded. A caught panic here is
/// swallowed — there is no reasonable way to propagate it further outward
/// through the WinRT dispatcher, and by construction the runtime itself
/// still owns all task state, so any tasks other than the one that panicked
/// remain pollable on the next drain.
pub(crate) fn drain_ready() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        with_current(|rt| rt.drain());
    }));
}

/// Owns the DQ-thread task map and the cross-thread ready queue. `!Send`:
/// the task map holds `Rc<dyn TaskObj>`, so this only ever lives on the DQ
/// thread, reached via the `CURRENT` thread-local.
pub(crate) struct RuntimeInner {
    pub(crate) queue: DispatcherQueue,
    pub(crate) ready: Arc<ReadyQueue>,
    tasks: RefCell<HashMap<u64, Rc<dyn TaskObj>>>,
    next_id: Cell<u64>,
}

impl RuntimeInner {
    pub(crate) fn new(queue: DispatcherQueue) -> Rc<Self> {
        Rc::new(Self {
            queue,
            ready: ReadyQueue::new(),
            tasks: RefCell::new(HashMap::new()),
            next_id: Cell::new(0),
        })
    }

    /// Pops every ready id and polls its task. Ids are deduped within a
    /// single drain pass (a waker may have fired more than once before this
    /// ran) so a task is never polled twice in the same pass; a task that
    /// wakes itself again schedules a fresh id for the *next* drain instead.
    pub(crate) fn drain(&self) {
        let mut ids = Vec::new();
        self.ready.drain_into(&mut ids);
        if ids.is_empty() {
            return;
        }

        let mut seen = HashSet::with_capacity(ids.len());
        for id in ids {
            if !seen.insert(id) {
                continue;
            }
            let Some(task) = self.tasks.borrow().get(&id).cloned() else {
                // Already removed (finished/aborted in a previous pass, or a
                // stale id from a task that raced its own completion).
                continue;
            };
            let waker = make_waker(Arc::new(WakerData {
                queue: self.queue.clone(),
                ready: self.ready.clone(),
                id,
            }));
            if task.poll_task(&waker) {
                self.tasks.borrow_mut().remove(&id);
            }
        }
    }

    /// Spawns a `!Send` future onto this runtime: allocates an id, stores
    /// the task (type-erased) in the task map, and schedules an initial
    /// drain so it gets its first poll.
    pub(crate) fn spawn<F>(self: &Rc<Self>, fut: F) -> (u64, LocalJoinHandle<F::Output>)
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        let id = self.next_id.get();
        self.next_id.set(id + 1);

        let (task, handle) = task::new_task(self, id, fut);
        self.tasks.borrow_mut().insert(id, task as Rc<dyn TaskObj>);

        self.ready.push(id);
        // `TryEnqueue` can fail (e.g. the queue is shutting down), in which
        // case this task's id sits on `ready` with nothing left to drain it
        // — it never gets its first poll and its `LocalJoinHandle` would
        // hang forever rather than resolve. We don't currently surface this:
        // a failed enqueue means the DispatcherQueue itself is going away
        // (or already gone), at which point the whole executor is moot and
        // no further drains would run regardless. Revisit if a caller ever
        // needs a hard signal here (e.g. resolve the handle to a dedicated
        // `JoinError` variant) rather than a silent stall.
        let _ = self.queue.TryEnqueue(&DispatcherQueueHandler::new(|| {
            drain_ready();
            Ok(())
        }));

        (id, handle)
    }
}

/// Public entry points for the dispatcher-queue runtime: actually creating
/// and pumping a `DispatcherQueue` event loop. Everything above is agnostic
/// to how the queue came to exist; this is where one gets created.
pub struct DispatcherRuntime;

/// The root future wrapped by `DispatcherRuntime::run`.
///
/// This polls the user's future manually (boxed, since `run`'s `F` isn't
/// required to be `Unpin`) and catches any panic itself with `catch_unwind`,
/// rather than letting it unwind out of this task's `poll`. That distinction
/// matters: `RuntimeInner::drain` (via `LocalTask::poll_task`) *also*
/// catches panics from a spawned future's `poll`, but only after the panic
/// has already unwound out of this type's `poll` entirely — which would
/// skip the `EnqueueEventLoopExit()` call below and hang `RunEventLoop`
/// forever (see the module-level hang-risk note in the task brief). By
/// catching here first, the exit path always runs, whether `root` finished
/// normally or panicked; `DispatcherRuntime::run` resumes the panic (if any)
/// after `RunEventLoop` has actually returned, on its own stack, well clear
/// of the WinRT FFI boundary that ultimately invoked this `poll`.
struct RunRootTask<T> {
    root: Pin<Box<dyn Future<Output = T>>>,
    done: bool,
    output: Rc<RefCell<Option<std::thread::Result<T>>>>,
    queue: DispatcherQueue,
}

impl<T> Future for RunRootTask<T> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.done {
            return Poll::Ready(());
        }
        let result = catch_unwind(AssertUnwindSafe(|| self.root.as_mut().poll(cx)));
        match result {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(value)) => {
                self.done = true;
                *self.output.borrow_mut() = Some(Ok(value));
                let _ = self.queue.EnqueueEventLoopExit();
                Poll::Ready(())
            }
            Err(payload) => {
                self.done = true;
                *self.output.borrow_mut() = Some(Err(payload));
                let _ = self.queue.EnqueueEventLoopExit();
                Poll::Ready(())
            }
        }
    }
}

/// The root future wrapped by `DispatcherRuntime::run_dedicated`.
///
/// Same shape and rationale as `RunRootTask`, but reports its outcome across
/// threads via a `Send` `mpsc::Sender` instead of an `Rc<RefCell<..>>` plus
/// an explicit `EnqueueEventLoopExit` — a dedicated-thread queue already
/// pumps its own event loop for its whole lifetime (see `run_dedicated`), so
/// there is no caller-owned `RunEventLoop` to release.
struct DedicatedRootTask<T> {
    root: Pin<Box<dyn Future<Output = T>>>,
    done: bool,
    tx: Option<std::sync::mpsc::Sender<std::thread::Result<T>>>,
}

impl<T> Future for DedicatedRootTask<T> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.done {
            return Poll::Ready(());
        }
        let result = catch_unwind(AssertUnwindSafe(|| self.root.as_mut().poll(cx)));
        match result {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(value)) => {
                self.done = true;
                if let Some(tx) = self.tx.take() {
                    let _ = tx.send(Ok(value));
                }
                clear_dedicated_thread_state();
                Poll::Ready(())
            }
            Err(payload) => {
                self.done = true;
                if let Some(tx) = self.tx.take() {
                    let _ = tx.send(Err(payload));
                }
                clear_dedicated_thread_state();
                Poll::Ready(())
            }
        }
    }
}

/// Uninstalls the dedicated-thread runtime once its root task has reached a
/// terminal state (completed or panicked): clears `CURRENT` (a `Weak`, so
/// this is mostly for consistency with `run`'s teardown — see its comment)
/// and releases `DEDICATED_ANCHOR`'s strong clone, which drops `RuntimeInner`
/// unless something else is still holding it alive. This runs on the DQ
/// thread itself, from inside `RuntimeInner::drain` (via `LocalTask::poll_task`
/// polling this future), which is still holding its own strong `Rc` for the
/// duration of that call — so releasing the anchor here is safe even though
/// it happens mid-`drain`.
fn clear_dedicated_thread_state() {
    CURRENT.with(|c| *c.borrow_mut() = None);
    DEDICATED_ANCHOR.with(|a| *a.borrow_mut() = None);
}

impl DispatcherRuntime {
    /// Creates a `DispatcherQueue` on the *current* thread, runs `root` to
    /// completion on it, and tears the queue back down before returning.
    ///
    /// Mirrors the lifecycle in `src/main.rs`: `CreateOnCurrentThread` ->
    /// `EnsureSystemDispatcherQueue` -> spawn -> `RunEventLoop` ->
    /// `ShutdownQueue`. `root` is spawned wrapped in a `RunRootTask`, which
    /// polls it manually (see that type's docs) so its completion path —
    /// store the output, `EnqueueEventLoopExit` — always runs, even if
    /// `root` panics.
    ///
    /// Note on bounds: the task brief's interface listing writes this as
    /// `fn run<F: Future>(root: F) -> F::Output`, but `RuntimeInner::spawn`
    /// (used here to register the wrapped root so it actually gets polled)
    /// requires `F: 'static` and `F::Output: 'static`; those bounds are made
    /// explicit here rather than silently relied upon.
    pub fn run<F>(root: F) -> F::Output
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        let controller =
            DispatcherQueueController::CreateOnCurrentThread()
                .expect("failed to create current-thread DispatcherQueueController");
        let queue = controller
            .DispatcherQueue()
            .expect("failed to get DispatcherQueue");
        queue
            .EnsureSystemDispatcherQueue()
            .expect("failed to ensure system DispatcherQueue");

        let rt = RuntimeInner::new(queue.clone());
        set_current(&rt);

        let output: Rc<RefCell<Option<std::thread::Result<F::Output>>>> =
            Rc::new(RefCell::new(None));
        let root_task = RunRootTask {
            root: Box::pin(root),
            done: false,
            output: output.clone(),
            queue: queue.clone(),
        };
        // `spawn` allocates an id, stores the task, and enqueues its first
        // drain — nothing further is needed to get it polled.
        let _ = rt.spawn(root_task);

        queue
            .RunEventLoop()
            .expect("DispatcherQueue::RunEventLoop failed");

        // Uninstall this runtime before tearing the queue down. `CURRENT`
        // only ever held a `Weak`, so this is mostly defensive: it turns a
        // stray `with_current` call after this point into a clean panic
        // ("no dq runtime on this thread") rather than a `Weak::upgrade`
        // racing the queue's teardown.
        CURRENT.with(|c| *c.borrow_mut() = None);
        drop(rt);

        controller
            .ShutdownQueue()
            .expect("DispatcherQueueController::ShutdownQueue failed");

        let result = output
            .borrow_mut()
            .take()
            .expect("root task did not complete before RunEventLoop returned");
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Creates a `DispatcherQueue` on a *new dedicated thread* (via
    /// `CreateOnDedicatedThread`, which — unlike `CreateOnCurrentThread` —
    /// starts pumping its own event loop immediately on that thread) and
    /// runs `make()`'s future to completion there, returning its output on
    /// the calling thread.
    ///
    /// Simplification (documented per the task brief, which calls this out
    /// as an acceptable minimal shape): this blocks the calling thread on
    /// the result via an `mpsc` channel, rather than hand back a
    /// handle/future the caller can poll independently. `F::Output` must be
    /// `Send` to cross back over that channel. The dedicated queue is shut
    /// down (`ShutdownQueue`, which joins its thread) before returning
    /// either way.
    pub fn run_dedicated<F, C>(make: C) -> F::Output
    where
        F: Future + 'static,
        F::Output: Send + 'static,
        C: FnOnce() -> F + Send + 'static,
    {
        let controller = DispatcherQueueController::CreateOnDedicatedThread()
            .expect("failed to create dedicated DispatcherQueueController");
        let queue = controller
            .DispatcherQueue()
            .expect("failed to get DispatcherQueue for dedicated thread");
        let inner_queue = queue.clone();

        let (tx, rx) = std::sync::mpsc::channel::<std::thread::Result<F::Output>>();
        // `DispatcherQueueHandler::new` requires `Fn`, not `FnOnce`, but
        // `make`/`tx` are only ever used on the single call this handler
        // actually receives; `Cell` lets a nominally-`Fn` closure still move
        // them out on that call (a second call, if it ever happened, would
        // just see `None` and no-op).
        let make_and_tx = Cell::new(Some((make, tx)));

        let enqueued = queue.TryEnqueue(&DispatcherQueueHandler::new(move || {
            if let Some((make, tx)) = make_and_tx.take() {
                let rt = RuntimeInner::new(inner_queue.clone());
                set_current(&rt);
                // `set_current` only stores a `Weak`; without a strong owner
                // surviving past this closure's return, `rt` would be the
                // *only* strong `Rc<RuntimeInner>` and would be dropped right
                // here — tearing down the task map (and the root task about
                // to be spawned into it) before the initial drain enqueued by
                // `spawn` below ever runs. Stash a strong clone here so the
                // runtime survives until `DedicatedRootTask::poll` clears it
                // on completion.
                DEDICATED_ANCHOR.with(|a| *a.borrow_mut() = Some(rt.clone()));
                let fut = make();
                let root_task = DedicatedRootTask {
                    root: Box::pin(fut),
                    done: false,
                    tx: Some(tx),
                };
                let _ = rt.spawn(root_task);
            }
            Ok(())
        }));
        assert!(
            matches!(enqueued, Ok(true)),
            "failed to enqueue root task on dedicated DispatcherQueue"
        );

        let result = rx
            .recv()
            .expect("dedicated DQ thread dropped without producing a result");
        let _ = controller.ShutdownQueue();

        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}

/// Spawns a `!Send` future onto the runtime installed on the *current*
/// thread — i.e. from inside a future already running under
/// `DispatcherRuntime::run`/`run_dedicated`, or any other code running on a
/// thread where `set_current` has been called. Panics (via `with_current`)
/// if there is no such runtime.
///
/// `RuntimeInner::spawn` already schedules the new task's first drain
/// (pushes its id onto `ready` and does a `TryEnqueue`), so this doesn't
/// need to duplicate that.
pub fn spawn_local<F>(fut: F) -> LocalJoinHandle<F::Output>
where
    F: Future + 'static,
    F::Output: 'static,
{
    with_current(|rt| rt.spawn(fut).1)
}

/// `PackageDependency` wraps raw WinRT/Win32 handles (`PACKAGEDEPENDENCY_CONTEXT`,
/// `PWSTR`) and so is neither `Send` nor `Sync`, which a plain
/// `static OnceLock<PackageDependency>` requires. This crate's dq tests are
/// only ever run single-threaded (`--test-threads=1` — every test owns a
/// current-thread `DispatcherQueue`, which is itself thread-affine), all on
/// the one test-harness worker thread, so it is never actually touched
/// concurrently; the wrapper below just asserts that to the type system.
#[cfg(test)]
struct SdkGuard(#[allow(dead_code)] winui3::bootstrap::PackageDependency);
#[cfg(test)]
unsafe impl Send for SdkGuard {}
#[cfg(test)]
unsafe impl Sync for SdkGuard {}

/// Initializes the Windows App SDK bootstrap exactly once for the test
/// process and keeps the returned `PackageDependency` alive forever (by
/// stashing it in a `static OnceLock`, never dropping it). Any dq test that
/// creates a `DispatcherQueue` (directly, or via `DispatcherRuntime::run`/
/// `run_dedicated`) must call this first.
#[cfg(test)]
pub(crate) fn ensure_sdk() {
    use std::sync::OnceLock;
    static SDK: OnceLock<SdkGuard> = OnceLock::new();
    SDK.get_or_init(|| {
        SdkGuard(
            winui3::bootstrap::PackageDependency::initialize_version(
                winui3::bootstrap::WindowsAppSDKVersion::V2,
            )
            .expect("failed to initialize Windows App SDK bootstrap for tests"),
        )
    });
}

/// Process-wide serialization lock for dq tests that create a `DispatcherQueue`
/// (directly, or via `DispatcherRuntime::run`/`run_dedicated`).
///
/// Those tests are only sound run single-threaded — see `SdkGuard`'s doc
/// comment above and the `run_tests`/`integration` module docs below — and
/// the repo's convention is `cargo test -- --test-threads=1`. But nothing
/// stops a plain `cargo test` (parallel by default) from running them
/// concurrently anyway, which would be a silent soundness violation rather
/// than a loud failure. This lock is a belt-and-suspenders backstop for that:
/// every such test acquires it (via `dq_test_guard()`) as its first line
/// after `ensure_sdk()`, so even a parallel harness serializes them. It does
/// *not* replace `--test-threads=1` as the documented/preferred way to run
/// these tests; it just makes forgetting the flag a slowdown instead of a
/// hang/flake.
#[cfg(test)]
pub(crate) static DQ_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Acquires `DQ_TEST_LOCK` for the duration of the returned guard, tolerating
/// a poisoned lock (a prior test panicking while holding it must not
/// poison-fail every subsequent dq test).
#[cfg(test)]
pub(crate) fn dq_test_guard() -> std::sync::MutexGuard<'static, ()> {
    DQ_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ready_queue_fifo() {
        let q = ReadyQueue::new();
        q.push(1); q.push(2);
        let mut out = Vec::new();
        q.drain_into(&mut out);
        assert_eq!(out, vec![1, 2]);
    }
}

/// Integration tests for `DispatcherRuntime::run`/`spawn_local` (and, via
/// the two panic-safety tests carried over from the previous task, for the
/// executor's panic containment now that there's a real event loop to run
/// them under). Each test creates and tears down its own `DispatcherQueue`
/// on the current thread, so — per the task brief — this module must be run
/// with `--test-threads=1` (`cargo test dq:: -- --test-threads=1`).
#[cfg(test)]
mod run_tests {
    use super::*;

    #[test]
    fn run_returns_value() {
        ensure_sdk();
        let _g = dq_test_guard();
        let v = DispatcherRuntime::run(async { 21 * 2 });
        assert_eq!(v, 42);
    }

    #[test]
    fn spawn_local_join() {
        ensure_sdk();
        let _g = dq_test_guard();
        let v = DispatcherRuntime::run(async {
            let h = spawn_local(async { 7 });
            h.await.unwrap()
        });
        assert_eq!(v, 7);
    }

    /// Regression coverage for the run_dedicated bootstrap bug: without a
    /// strong owner surviving past the bootstrap closure's return, the
    /// runtime (and the just-spawned root task) was dropped before its first
    /// poll, so `run_dedicated` panicked on *every* call (`rx.recv()` saw a
    /// disconnected channel). See `DEDICATED_ANCHOR`.
    #[test]
    fn run_dedicated_returns_value() {
        ensure_sdk();
        let _g = dq_test_guard();
        assert_eq!(DispatcherRuntime::run_dedicated(|| async { 40 + 2 }), 42);
    }

    /// Same regression coverage as `run_dedicated_returns_value`, but also
    /// exercises `spawn_local`/`LocalJoinHandle` on the dedicated thread: the
    /// root spawns a child task and awaits it, which only works if the
    /// runtime (and its task map) is still alive and getting drained after
    /// the bootstrap closure returns.
    #[test]
    fn run_dedicated_spawn_local_join() {
        ensure_sdk();
        let _g = dq_test_guard();
        let v = DispatcherRuntime::run_dedicated(|| async {
            let h = spawn_local(async { 19 + 23 });
            h.await.unwrap()
        });
        assert_eq!(v, 42);
    }

    #[test]
    fn cross_thread_wake_resumes_on_dq_thread() {
        ensure_sdk();
        let _g = dq_test_guard();
        // Await a comio-style op whose completion wakes from a pool thread; the
        // continuation must run on the DQ thread. `IAsyncOperation::spawn`
        // runs the closure on the Windows thread pool and completes there,
        // waking our waker from off the DQ thread.
        let same = DispatcherRuntime::run(async {
            let dq_thread = std::thread::current().id();
            let _ = windows_future::IAsyncOperation::<i32>::spawn(|| Ok(1)).await;
            std::thread::current().id() == dq_thread
        });
        assert!(same, "continuation did not resume on the DQ thread");
    }

    /// A future that panics when *dropped*, not when polled.
    struct PanicOnDrop;
    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            panic!("PanicOnDrop::drop");
        }
    }

    /// Deferred from the task-3 brief (there was no `run()` yet to host it
    /// under): abort a spawned task whose future is holding a value that
    /// panics on drop. Aborting drops the future in place — see
    /// `LocalTask::poll_task`'s abort branch — which panics; that panic must
    /// be contained by the executor's own guards rather than unwinding out
    /// through the WinRT `DispatcherQueueHandler::Invoke` FFI boundary and
    /// aborting the process. The join handle should still resolve (either
    /// `Aborted`, or `Ok` if the task raced its own completion first), and
    /// `run()` must still return normally.
    #[test]
    fn aborted_task_panic_on_drop_is_contained() {
        ensure_sdk();
        let _g = dq_test_guard();
        let result = DispatcherRuntime::run(async {
            let h = spawn_local(async {
                let _guard = PanicOnDrop;
                // Never resolves on its own; only `abort()` ends this task.
                std::future::pending::<()>().await
            });
            h.abort();
            h.await
        });
        assert!(
            matches!(result, Err(task::JoinError::Aborted) | Ok(())),
            "unexpected join result for an aborted panic-on-drop task: {result:?}"
        );
        // Reaching this line at all (process still running, `run()`
        // returned normally) is the main assertion.
    }

    /// Deferred from the task-3 brief: a spawned task that panics *during
    /// poll* (as opposed to on drop, above). The panic must be caught by
    /// `LocalTask::poll_task`'s `catch_unwind` around the future's `poll`
    /// call, surfaced as `JoinError::Panicked`, and must not prevent `run()`
    /// from completing normally.
    #[test]
    fn spawned_task_panic_in_poll_is_contained() {
        ensure_sdk();
        let _g = dq_test_guard();
        let result = DispatcherRuntime::run(async {
            let h = spawn_local(async {
                panic!("boom");
                #[allow(unreachable_code)]
                42
            });
            h.await
        });
        match result {
            Err(task::JoinError::Panicked(_)) => {}
            other => panic!("expected JoinError::Panicked, got {other:?}"),
        }
    }
}

/// Proves the two layers actually compose: a `comio` named-pipe echo driven
/// from inside a `dq` `DispatcherRuntime::run` future. The server lives on a
/// plain `std::thread` and drives its half with `tp::block_on` (`comio` is
/// runtime-agnostic, per its own module docs); the client is the `run` root
/// future itself and drives its half via plain `.await` on the WinRT async
/// operations `NamedPipeConnector`/`ReadAsync`/`WriteAsync` return — exercising
/// the executor/waker bridge against a real overlapped-IO completion
/// (delivered from a thread-pool callback) rather than the synthetic
/// `IAsyncOperation::spawn` used by `cross_thread_wake_resumes_on_dq_thread`
/// above.
///
/// Adapted from the task-6 brief's sketch: the brief passes
/// `IAsyncOperationWithProgress` values straight to `tp::block_on`, but
/// `block_on` requires `F: Future` and these WinRT operation types only
/// implement `IntoFuture` (see `comio`'s own tests, e.g.
/// `pipe::tests::named_pipe_echo`, which all call `.into_future()` before
/// `block_on`) — `.await` performs that conversion implicitly, but a bare
/// function call does not. The client-side `run` future uses plain `.await`
/// throughout (as `run_tests::cross_thread_wake_resumes_on_dq_thread`
/// already does), so no `.into_future()` is needed there.
// Cross-runtime integration tests: dq driving comio over a tp-backed server.
#[cfg(all(test, feature = "comio", feature = "tp"))]
mod integration {
    use super::*;

    #[test]
    fn dq_named_pipe_echo() {
        crate::dq::ensure_sdk();
        let _g = crate::dq::dq_test_guard();
        let name = format!(r"\\.\pipe\dq_comio_{}", std::process::id());
        let sname = name.clone();
        // Server on a plain thread using tp::block_on (comio is runtime-agnostic).
        let server = std::thread::spawn(move || {
            use crate::comio::buffer::new_buffer;
            use crate::comio::NamedPipeListener;
            use crate::tp::block_on;
            use std::future::IntoFuture;
            use windows::Storage::Streams::{
                IBuffer, IInputStream, IOutputStream, InputStreamOptions,
            };
            use windows_core::Interface;

            let l = NamedPipeListener::create(&sname);
            let s = block_on(l.accept()).unwrap();
            let i: IInputStream = s.cast().unwrap();
            let o: IOutputStream = s.cast().unwrap();
            let rb = new_buffer(16).unwrap();
            let got: IBuffer = block_on(
                i.ReadAsync(&rb.cast::<IBuffer>().unwrap(), 16, InputStreamOptions(0))
                    .unwrap()
                    .into_future(),
            )
            .unwrap();
            block_on(o.WriteAsync(&got).unwrap().into_future()).unwrap();
        });
        std::thread::sleep(std::time::Duration::from_millis(100));

        let echoed = DispatcherRuntime::run(async move {
            use crate::comio::buffer::{buffer_ptr, new_buffer};
            use crate::comio::NamedPipeConnector;
            use windows::Storage::Streams::{
                IBuffer, IInputStream, IOutputStream, InputStreamOptions,
            };
            use windows_core::Interface;

            let c = NamedPipeConnector::connect(&name).await.unwrap();
            let o: IOutputStream = c.cast().unwrap();
            let i: IInputStream = c.cast().unwrap();

            let wb = new_buffer(4).unwrap();
            unsafe {
                buffer_ptr(&wb.clone().into())
                    .unwrap()
                    .copy_from_nonoverlapping(b"echo".as_ptr(), 4);
            }
            wb.SetLength(4).unwrap();
            o.WriteAsync(&wb.cast::<IBuffer>().unwrap())
                .unwrap()
                .await
                .unwrap();

            let rb = new_buffer(4).unwrap();
            let got: IBuffer = i
                .ReadAsync(&rb.cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0))
                .unwrap()
                .await
                .unwrap();
            unsafe { std::slice::from_raw_parts(buffer_ptr(&got).unwrap(), 4).to_vec() }
        });

        assert_eq!(&echoed, b"echo");
        server.join().unwrap();
    }
}
