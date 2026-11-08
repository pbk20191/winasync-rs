//! A runtime-neutral single-shot channel: one `Sender` hands exactly one value
//! to one `Receiver` (a `Future`).
//!
//! Depends only on `std::sync` + `core::task::Waker` — it knows nothing about
//! `tp`, `dq`, or any executor, so it is the handoff between *any* two contexts
//! (pool task -> UI task, world thread -> render thread, …). The receiver
//! resolves on whatever executor polls it, woken by whatever thread sends.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

/// Returned by [`Receiver`] when the [`Sender`] was dropped without sending.
#[derive(Debug, PartialEq, Eq)]
pub struct Canceled;

struct Inner<T> {
    value: Option<T>,
    waker: Option<Waker>,
    /// The sender fulfilled its promise (`send` was called).
    completed: bool,
    /// The sender was dropped without sending.
    sender_dropped: bool,
    /// The receiver was dropped; a later `send` has nowhere to go.
    receiver_dropped: bool,
}

/// The sending half. Consumed by [`send`](Self::send); dropping it without
/// sending cancels the receiver.
pub struct Sender<T> {
    inner: Arc<Mutex<Inner<T>>>,
}

/// The receiving half: a `Future` resolving to the sent value, or
/// `Err(Canceled)` if the sender dropped first.
pub struct Receiver<T> {
    inner: Arc<Mutex<Inner<T>>>,
}

/// Creates a connected `(Sender, Receiver)` pair.
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let inner = Arc::new(Mutex::new(Inner {
        value: None,
        waker: None,
        completed: false,
        sender_dropped: false,
        receiver_dropped: false,
    }));
    (
        Sender {
            inner: inner.clone(),
        },
        Receiver { inner },
    )
}

impl<T> Sender<T> {
    /// Sends `value` and wakes the receiver. Returns `Err(value)` if the
    /// receiver was already dropped (nothing will ever receive it).
    pub fn send(self, value: T) -> Result<(), T> {
        let waker = {
            let mut inner = self.inner.lock().unwrap();
            if inner.receiver_dropped {
                return Err(value);
            }
            inner.value = Some(value);
            inner.completed = true;
            inner.waker.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
        Ok(())
    }

    /// Whether the receiver has been dropped (a `send` would be returned).
    pub fn is_canceled(&self) -> bool {
        self.inner.lock().unwrap().receiver_dropped
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let waker = {
            let mut inner = self.inner.lock().unwrap();
            if inner.completed {
                return;
            }
            inner.sender_dropped = true;
            inner.waker.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    }
}

impl<T> Future for Receiver<T> {
    type Output = Result<T, Canceled>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(v) = inner.value.take() {
            return Poll::Ready(Ok(v));
        }
        if inner.sender_dropped {
            return Poll::Ready(Err(Canceled));
        }
        inner.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.inner.lock().unwrap().receiver_dropped = true;
    }
}

#[cfg(all(test, any(feature = "tp", feature = "comio")))]
mod tests {
    use super::*;
    use crate::common::block_on;

    #[test]
    fn send_then_receive() {
        let (tx, rx) = channel::<u32>();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            tx.send(42).unwrap();
        });
        assert_eq!(block_on(rx), Ok(42));
    }

    #[test]
    fn send_before_poll() {
        let (tx, rx) = channel::<&'static str>();
        tx.send("hi").unwrap();
        assert_eq!(block_on(rx), Ok("hi"));
    }

    #[test]
    fn sender_dropped_is_canceled() {
        let (tx, rx) = channel::<u32>();
        drop(tx);
        assert_eq!(block_on(rx), Err(Canceled));
    }

    #[test]
    fn receiver_dropped_send_errs() {
        let (tx, rx) = channel::<u32>();
        drop(rx);
        assert_eq!(tx.send(7), Err(7));
    }
}
