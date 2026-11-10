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

pub mod io;
pub mod pool;
pub mod task;
pub mod timer;
pub mod wait;

#[allow(unused_imports)]
pub use crate::common::block_on::block_on;
#[allow(unused_imports)]
pub use io::async_rw::{
    AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, TcpStreamIo, WriteHalf,
};
#[allow(unused_imports)]
pub use io::file::{File, PageBuffers};
#[allow(unused_imports)]
pub use io::net::{
    parse_ip_pktinfo, ReceivedMsg, ReusableSocket, TcpListener, TcpStream, TransmitFlags,
    TransmitPackets, UdpSocket,
};
#[allow(unused_imports)]
pub use io::pipe::{NamedPipeConnector, NamedPipeListener, NamedPipeOptions, NamedPipeStream};
#[allow(unused_imports)]
pub use io::unix::{UnixListener, UnixStream};
// Who may connect to a listener, for `bind_with_security` on the local
// transports.
pub use crate::common::security::SecurityDescriptor;
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

pub(crate) use crate::common::check::check;

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

    /// Runs `fut` to completion with its body executing **on the pool**, blocking
    /// the calling thread until it finishes.
    ///
    /// Unlike the free [`block_on`] — which polls the future on the *calling*
    /// thread — this spawns `fut` onto the pool (so every poll of its body, and
    /// the synchronous code between its `await`s, runs on a pool thread) and the
    /// caller only parks on the resulting [`JoinHandle`]. The future therefore
    /// must be `Send + 'static`, as it crosses threads.
    ///
    /// # Panics
    /// If the spawned task panicked (the panic is observed via its `JoinError`).
    pub fn block_on<F>(&self, fut: F) -> F::Output
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let handle = self.spawn(fut);
        crate::common::block_on::block_on(async move {
            handle.await.expect("block_on: spawned task failed")
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_block_on_runs_body_on_pool() {
        let rt = Runtime::new();
        let caller = std::thread::current().id();
        let (before, after) = rt.block_on(async move {
            let before = std::thread::current().id();
            yield_now().await;
            let after = std::thread::current().id();
            (before, after)
        });
        // The spawned body is polled on pool threads, never the caller's.
        assert_ne!(before, caller, "first poll ran on the calling thread");
        assert_ne!(after, caller, "post-await poll ran on the calling thread");
    }

    #[test]
    fn runtime_block_on_returns_value() {
        let rt = Runtime::new();
        assert_eq!(rt.block_on(async { 20 + 22 }), 42);
    }
}
