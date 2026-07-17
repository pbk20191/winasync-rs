//! Single-threaded async runtime whose executor is a WinRT DispatcherQueue.
//! `!Send` futures run on the DQ thread; a Send waker marshals cross-thread
//! wakes back via TryEnqueue. See docs/superpowers/specs for the design.
#![allow(dead_code)]

pub mod task;
pub mod timer;
pub mod waker;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};

use task::{LocalJoinHandle, TaskObj};
use waker::{make_waker, WakerData};
use winui3::Microsoft::UI::Dispatching::{DispatcherQueue, DispatcherQueueHandler};

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
pub(crate) fn drain_ready() {
    with_current(|rt| rt.drain());
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
        let _ = self.queue.TryEnqueue(&DispatcherQueueHandler::new(|| {
            drain_ready();
            Ok(())
        }));

        (id, handle)
    }
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
