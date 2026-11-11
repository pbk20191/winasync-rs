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
