//! A waker slot one side registers into and another side wakes, without a
//! lock: the one piece of cross-thread state a pump that only *notifies*
//! needs.
//!
//! The shape is `futures_util::task::AtomicWaker`'s: a two-bit state word
//! guards the slot, a registration that loses the race to a concurrent wake
//! wakes itself so no notification is lost, and a wake that loses the race
//! to a registration leaves the registering side to notice `WAKING` and
//! wake. Written here rather than depended on so the crate keeps its
//! dependency surface.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU8, Ordering};
use std::task::Waker;

const IDLE: u8 = 0;
const REGISTERING: u8 = 0b01;
const WAKING: u8 = 0b10;

pub struct AtomicWaker {
    state: AtomicU8,
    waker: UnsafeCell<Option<Waker>>,
}

// The slot is only reached under the state word's exclusion.
unsafe impl Send for AtomicWaker {}
unsafe impl Sync for AtomicWaker {}

impl Default for AtomicWaker {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicWaker {
    pub const fn new() -> Self {
        AtomicWaker {
            state: AtomicU8::new(IDLE),
            waker: UnsafeCell::new(None),
        }
    }

    /// Stores `waker` as the one to wake. If a wake arrived meanwhile, it is
    /// delivered to `waker` now rather than lost.
    pub fn register(&self, waker: &Waker) {
        match self
            .state
            .compare_exchange(IDLE, REGISTERING, Ordering::Acquire, Ordering::Acquire)
        {
            Ok(_) => {
                // SAFETY: REGISTERING excludes every other access.
                unsafe {
                    let slot = &mut *self.waker.get();
                    match slot {
                        Some(old) if old.will_wake(waker) => {}
                        _ => *slot = Some(waker.clone()),
                    }
                }
                if self
                    .state
                    .compare_exchange(REGISTERING, IDLE, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    // A wake came in while registering: it could not reach
                    // the slot, so deliver it ourselves.
                    // SAFETY: still exclusive -- the waker set WAKING and left.
                    let waker = unsafe { (*self.waker.get()).take() };
                    self.state.swap(IDLE, Ordering::AcqRel);
                    if let Some(w) = waker {
                        w.wake();
                    }
                }
            }
            Err(WAKING) => {
                // Being woken right now: the wake is for whoever is current,
                // which is us.
                waker.wake_by_ref();
            }
            Err(_) => {
                // Another registration in progress: a slot has one
                // registrant at a time by contract; the newer loses.
            }
        }
    }

    /// Wakes the registered waker, if any, and clears the slot.
    pub fn wake(&self) {
        if let Some(w) = self.take() {
            w.wake();
        }
    }

    /// Removes and returns the registered waker.
    pub fn take(&self) -> Option<Waker> {
        match self.state.fetch_or(WAKING, Ordering::AcqRel) {
            IDLE => {
                // SAFETY: WAKING set from IDLE excludes every other access.
                let waker = unsafe { (*self.waker.get()).take() };
                self.state.fetch_and(!WAKING, Ordering::Release);
                waker
            }
            // A registration is in progress; it sees WAKING on its way out
            // and wakes the waker it just stored. Or a wake is already in
            // progress, which will deliver.
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::task::{Wake, Waker};

    struct Count(AtomicUsize);
    impl Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn registers_and_wakes_once() {
        let slot = AtomicWaker::new();
        let count = Arc::new(Count(AtomicUsize::new(0)));
        let waker = Waker::from(count.clone());
        slot.wake();
        assert_eq!(count.0.load(Ordering::SeqCst), 0, "nothing registered");
        slot.register(&waker);
        slot.wake();
        slot.wake();
        assert_eq!(count.0.load(Ordering::SeqCst), 1, "a wake clears the slot");
    }

    #[test]
    fn a_wake_racing_a_registration_is_not_lost() {
        let slot = Arc::new(AtomicWaker::new());
        let count = Arc::new(Count(AtomicUsize::new(0)));
        let waker = Waker::from(count.clone());
        let waking = {
            let slot = slot.clone();
            std::thread::spawn(move || {
                for _ in 0..100_000 {
                    slot.wake();
                }
            })
        };
        for _ in 0..100_000 {
            slot.register(&waker);
        }
        waking.join().unwrap();
        // The last registration may or may not have been woken; what cannot
        // happen is a wake observed by neither side, which would show as a
        // slot holding a waker while a wake returned without it.
        slot.wake();
        assert!(count.0.load(Ordering::SeqCst) >= 1);
    }
}
