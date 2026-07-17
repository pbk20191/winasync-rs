//! Single-threaded async runtime whose executor is a WinRT DispatcherQueue.
//! `!Send` futures run on the DQ thread; a Send waker marshals cross-thread
//! wakes back via TryEnqueue. See docs/superpowers/specs for the design.
#![allow(dead_code)]

pub mod task;
pub mod timer;
pub mod waker;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

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
