//! General-purpose io + task + scheduler + rust-async runtime built on the
//! Win32 Thread Pool API (TP_POOL / TP_WORK / TP_TIMER / TP_WAIT / TP_IO).
//!
//! Structure (in the style of the `windows-threading` crate, extended):
//! * [`Pool`] — low-level pool: fire-and-forget closure submission, scoped
//!   submission, and `join()` via the cleanup group barrier.
//! * [`Runtime`] — async runtime sharing the same pool machinery: `spawn`
//!   returning [`JoinHandle`], TP_TIMER based [`sleep`]/[`timeout`]/[`Interval`],
//!   TP_WAIT based [`wait_for_handle`], and TP_IO based async file/socket IO.
//! * [`block_on`] — drive a future to completion on the calling thread with an
//!   event-based parker.
//!
//! # Unsafe invariants (module-wide)
//!
//! 1. Per task at most one `SubmitThreadpoolWork` is outstanding at any time —
//!    only the `IDLE -> SCHEDULED` waker transition and the runner's consumption
//!    of `NOTIFIED` submit. This is what makes the future exclusively owned
//!    while the task is `RUNNING`.
//! 2. Every TP object context starts with a [`pool::CtxHeader`]; the callback
//!    environment's `FinalizationCallback` is the *single* release path, firing
//!    exactly once when the object is finalized (individual `Close*` or
//!    `CloseThreadpoolCleanupGroupMembers`). Callbacks and wrappers never free
//!    contexts directly.
//! 3. `TP_CALLBACK_ENVIRON_V3` lives in a `Box` so its address is stable.
//!    Individual member closes happen only while the owning wrapper is alive;
//!    `CloseThreadpoolCleanupGroupMembers` runs only in teardown after every
//!    `Arc` to the pool is gone, so the two can never race.
//! 4. An in-flight OVERLAPPED op block is never freed by the future side —
//!    cancellation is `CancelIoEx` plus a refcount release; the completion
//!    callback frees the block.
//! 5. `StartThreadpoolIo` is balanced 1:1 by either a delivered completion or
//!    `CancelThreadpoolIo` on synchronous submit failure.
//! 6. Never call `WaitForThreadpool*Callbacks` from a callback of the same
//!    object (deadlock).
//! 7. No panic may cross the FFI boundary: the task runner uses
//!    `catch_unwind`; every other callback is panic-free by construction.

// Library-style API surface inside a binary crate: much of it is exercised
// only by the unit tests and the demo.
#![allow(dead_code)]

pub mod block_on;
pub mod io;
pub mod pool;
pub mod task;
pub mod timer;
pub mod wait;

#[allow(unused_imports)]
pub use block_on::block_on;
#[allow(unused_imports)]
pub use io::file::File;
#[allow(unused_imports)]
pub use io::net::{TcpListener, TcpStream};
#[allow(unused_imports)]
pub use pool::{Pool, PoolBuilder, Priority, Scope};
#[allow(unused_imports)]
pub use task::{JoinError, JoinHandle};
#[allow(unused_imports)]
pub use timer::{interval, sleep, timeout, Elapsed, Interval, Sleep, Tick, Timeout};
#[allow(unused_imports)]
pub use wait::{wait_for_handle, HandleWait};

use pool::PoolInner;
use std::sync::Arc;

/// When used correctly the thread pool APIs only fail on memory exhaustion;
/// map such failures to a panic (same policy as `windows-threading`).
pub(crate) fn check<T>(result: windows_core::Result<T>) -> T {
    result.expect("threadpool allocation failed")
}

/// A handle to an async runtime backed by a private Win32 thread pool.
///
/// Cheap to clone; the pool is torn down (waiting for all outstanding
/// callbacks via `CloseThreadpoolCleanupGroupMembers`) when the last clone
/// *and* the last spawned task have finished.
#[derive(Clone)]
pub struct Runtime {
    pub(crate) inner: Arc<RuntimeInner>,
}

pub(crate) struct RuntimeInner {
    pub(crate) pool: PoolInner,
}

impl Runtime {
    /// Creates a runtime with default pool settings.
    pub fn new() -> Self {
        PoolBuilder::new().build_runtime()
    }

    /// Spawns a future onto the pool at the pool's default priority.
    pub fn spawn<F>(&self, fut: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        task::spawn(self, fut, None)
    }

    /// Spawns a future with an explicit callback priority.
    pub fn spawn_with_priority<F>(&self, priority: Priority, fut: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        task::spawn(self, fut, Some(priority))
    }

    /// Submits a plain closure at the pool's default priority (fire and forget).
    pub fn submit<F: FnOnce() + Send + 'static>(&self, f: F) {
        unsafe { pool::try_submit(self.inner.pool.default_env(), f) }
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

/// Cooperatively yields once: wakes itself and returns `Pending` a single time.
pub fn yield_now() -> impl Future<Output = ()> {
    struct YieldNow(bool);
    impl Future for YieldNow {
        type Output = ();
        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<()> {
            if self.0 {
                std::task::Poll::Ready(())
            } else {
                self.0 = true;
                cx.waker().wake_by_ref();
                std::task::Poll::Pending
            }
        }
    }
    YieldNow(false)
}
