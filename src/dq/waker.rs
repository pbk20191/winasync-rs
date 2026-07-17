//! Waker implementation for the dispatcher queue runtime.
//!
//! The waker holds only `Send` data (an agile `DispatcherQueue`, an
//! `Arc<ReadyQueue>`, and a `u64` task id) so it can be invoked from any
//! thread — in particular, from an IO-completion pool thread driving
//! `comio`/`windows-future` completions. `wake()` pushes the task's id onto
//! the ready queue and asks the `DispatcherQueue` to run a drain handler on
//! the DQ thread. That handler captures nothing `!Send`; it reaches the
//! (single-threaded, `!Send`) runtime purely through the `CURRENT`
//! thread-local in `crate::dq`.

use crate::dq::ReadyQueue;
use std::sync::Arc;
use std::task::{RawWaker, RawWakerVTable, Waker};
use winui3::Microsoft::UI::Dispatching::{DispatcherQueue, DispatcherQueueHandler};

/// Send-only payload backing a [`Waker`] for a single dq task.
///
/// `DispatcherQueue` is an agile WinRT object: `TryEnqueue` is safe to call
/// from any thread, which is the only operation ever performed on `queue`
/// here. Combined with `Arc<ReadyQueue>` (already `Send + Sync`, backed by a
/// `Mutex`) and a plain `u64`, every field is safe to share/send across
/// threads, so this is also derivable automatically — the explicit impls
/// below just document the invariant relied on.
pub(crate) struct WakerData {
    pub(crate) queue: DispatcherQueue,
    pub(crate) ready: Arc<ReadyQueue>,
    pub(crate) id: u64,
}

// SAFETY: see the doc comment above — only `TryEnqueue` (agile) is ever
// called on `queue`, and it may be called from any thread.
unsafe impl Send for WakerData {}
unsafe impl Sync for WakerData {}

impl WakerData {
    /// Marks this task ready and asks the DQ thread to drain it. The
    /// enqueued closure captures nothing at all (hence trivially `Send`);
    /// it reaches the runtime solely via the `CURRENT` thread-local.
    fn schedule(&self) {
        self.ready.push(self.id);
        let _ = self.queue.TryEnqueue(&DispatcherQueueHandler::new(|| {
            crate::dq::drain_ready();
            Ok(())
        }));
    }
}

static VTABLE: RawWakerVTable = RawWakerVTable::new(
    |p| {
        unsafe { Arc::increment_strong_count(p as *const WakerData) };
        RawWaker::new(p, &VTABLE)
    },
    |p| {
        let data = unsafe { Arc::from_raw(p as *const WakerData) };
        data.schedule();
    },
    |p| unsafe { (*(p as *const WakerData)).schedule() },
    |p| unsafe { drop(Arc::from_raw(p as *const WakerData)) },
);

/// Builds a `Waker` over the given `WakerData`. Cloning/dropping the waker
/// manages the `Arc`'s strong count; waking pushes the id and enqueues a
/// drain on the DQ thread.
pub(crate) fn make_waker(data: Arc<WakerData>) -> Waker {
    unsafe { Waker::from_raw(RawWaker::new(Arc::into_raw(data) as *const (), &VTABLE)) }
}

#[cfg(test)]
mod tests {
    use crate::dq::ReadyQueue;
    // A waker built over a ReadyQueue pushes the id on wake (queue enqueue is
    // covered by integration tests that own a DispatcherQueue).
    #[test]
    fn wake_pushes_id() {
        let ready = ReadyQueue::new();
        // Construct WakerData with a null-ish queue is impossible (needs a
        // real DispatcherQueue). So this test only asserts the ReadyQueue
        // push path used by `WakerData::schedule` via a direct call:
        ready.push(42);
        let mut out = Vec::new();
        ready.drain_into(&mut out);
        assert_eq!(out, vec![42]);
    }
}
