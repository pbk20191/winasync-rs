//! Drive a future to completion on the calling thread, parking on a Win32
//! auto-reset event between polls.

use super::check;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject, INFINITE};

struct Parker {
    event: HANDLE,
}

// The event handle is only ever passed to thread-safe kernel calls.
unsafe impl Send for Parker {}
unsafe impl Sync for Parker {}

impl Parker {
    fn new() -> Arc<Self> {
        // Auto-reset: a wake while nobody waits is consumed by the next wait,
        // and multiple wakes coalesce into one — the poll loop tolerates both.
        let event = unsafe { check(CreateEventW(None, false, false, None)) };
        Arc::new(Self { event })
    }

    fn unpark(&self) {
        unsafe { check(SetEvent(self.event)) };
    }

    fn park(&self) {
        unsafe { WaitForSingleObject(self.event, INFINITE) };
    }

    fn waker(self: &Arc<Self>) -> Waker {
        unsafe { Waker::from_raw(raw_waker(Arc::into_raw(self.clone()))) }
    }
}

impl Drop for Parker {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.event);
        }
    }
}

fn raw_waker(parker: *const Parker) -> RawWaker {
    static VTABLE: RawWakerVTable = RawWakerVTable::new(
        |p| {
            unsafe { Arc::increment_strong_count(p as *const Parker) };
            raw_waker(p as *const Parker)
        },
        |p| {
            let parker = unsafe { Arc::from_raw(p as *const Parker) };
            parker.unpark();
        },
        |p| unsafe { (*(p as *const Parker)).unpark() },
        |p| unsafe { drop(Arc::from_raw(p as *const Parker)) },
    );
    RawWaker::new(parker as *const (), &VTABLE)
}

/// Runs the future to completion on the calling thread. Spawned tasks keep
/// running on their pool threads; only the top-level future is driven here.
pub fn block_on<F: Future>(f: F) -> F::Output {
    let parker = Parker::new();
    let waker = parker.waker();
    let mut cx = Context::from_waker(&waker);
    let mut f = pin!(f);
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => parker.park(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_future() {
        assert_eq!(block_on(async { 42 }), 42);
    }

    #[test]
    fn wake_from_thread() {
        use std::sync::Mutex;

        struct Shared {
            done: Mutex<(bool, Option<Waker>)>,
        }
        struct WokenElsewhere(Arc<Shared>, bool);
        impl Future for WokenElsewhere {
            type Output = u32;
            fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u32> {
                let shared = self.0.clone();
                let mut guard = shared.done.lock().unwrap();
                if guard.0 {
                    return Poll::Ready(7);
                }
                guard.1 = Some(cx.waker().clone());
                if !self.1 {
                    self.1 = true;
                    let shared = shared.clone();
                    drop(guard);
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        let mut guard = shared.done.lock().unwrap();
                        guard.0 = true;
                        if let Some(w) = guard.1.take() {
                            w.wake();
                        }
                    });
                }
                Poll::Pending
            }
        }

        let shared = Arc::new(Shared {
            done: Mutex::new((false, None)),
        });
        assert_eq!(block_on(WokenElsewhere(shared, false)), 7);
    }
}
