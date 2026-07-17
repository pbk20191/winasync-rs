//! TP_WAIT based asynchronous waiting on kernel handles.

use super::pool::CtxHeader;
use super::{check, Runtime, RuntimeInner};
use core::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::{
    CloseThreadpoolWait, CreateThreadpoolWait, SetThreadpoolWait, WaitForThreadpoolWaitCallbacks,
    PTP_CALLBACK_INSTANCE, PTP_WAIT,
};

struct WaitState {
    /// `WAIT_OBJECT_0` (0) etc. once the wait has fired.
    result: Option<u32>,
    waker: Option<Waker>,
}

/// Context of a TP_WAIT; released by the environment's finalization callback.
#[repr(C)]
struct WaitShared {
    header: CtxHeader,
    state: Mutex<WaitState>,
}

unsafe fn release_wait(ctx: *mut c_void) {
    unsafe { drop(Arc::from_raw(ctx as *const WaitShared)) }
}

unsafe extern "system" fn wait_callback(
    _: PTP_CALLBACK_INSTANCE,
    ctx: *mut c_void,
    _: PTP_WAIT,
    waitresult: u32,
) {
    let shared = unsafe { &*(ctx as *const WaitShared) };
    let waker = {
        let mut state = shared.state.lock().unwrap();
        state.result = Some(waitresult);
        state.waker.take()
    };
    if let Some(w) = waker {
        let _ = catch_unwind(AssertUnwindSafe(move || w.wake()));
    }
}

/// Future returned by [`wait_for_handle`]; resolves to the wait result
/// (`WAIT_OBJECT_0` = 0, `WAIT_ABANDONED`, ...).
pub struct HandleWait {
    wait: PTP_WAIT,
    shared: Arc<WaitShared>,
    _rt: Arc<RuntimeInner>,
}

unsafe impl Send for HandleWait {}
unsafe impl Sync for HandleWait {}

impl Future for HandleWait {
    type Output = u32;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u32> {
        let mut state = self.shared.state.lock().unwrap();
        if let Some(result) = state.result {
            Poll::Ready(result)
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl Drop for HandleWait {
    fn drop(&mut self) {
        unsafe {
            // Deactivate, cancel a queued callback, wait out a running one,
            // then close. Context released via finalization. A TP_WAIT
            // deactivates itself after firing, so this is also the normal
            // post-completion teardown.
            SetThreadpoolWait(self.wait, None, None);
            WaitForThreadpoolWaitCallbacks(self.wait, true);
            CloseThreadpoolWait(self.wait);
        }
    }
}

/// Asynchronously waits for `handle` to become signaled (one-shot).
///
/// # Safety
/// `handle` must be a valid waitable handle and must stay open until the
/// returned future completes or is dropped.
pub unsafe fn wait_for_handle(rt: &Runtime, handle: HANDLE) -> HandleWait {
    let shared = Arc::new(WaitShared {
        header: CtxHeader {
            release: release_wait,
        },
        state: Mutex::new(WaitState {
            result: None,
            waker: None,
        }),
    });
    let ctx = Arc::into_raw(shared.clone());
    let wait = unsafe {
        check(CreateThreadpoolWait(
            Some(wait_callback),
            Some(ctx as *mut c_void),
            Some(rt.inner.pool.default_env()),
        ))
    };
    unsafe { SetThreadpoolWait(wait, Some(handle), None) };
    HandleWait {
        wait,
        shared,
        _rt: rt.inner.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{block_on, check, timer::sleep, Runtime};
    use super::*;
    use std::time::Duration;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{CreateEventW, SetEvent};

    #[test]
    fn wait_resolves_when_event_set() {
        let rt = Runtime::new();
        let event = unsafe { check(CreateEventW(None, true, false, None)) };

        let rt2 = rt.clone();
        let raw = event.0 as isize;
        let setter = rt.spawn(async move {
            sleep(&rt2, Duration::from_millis(30)).await;
            unsafe { check(SetEvent(HANDLE(raw as *mut _))) };
        });

        let result = block_on(unsafe { wait_for_handle(&rt, event) });
        assert_eq!(result, 0); // WAIT_OBJECT_0
        block_on(setter).unwrap();
        unsafe {
            let _ = CloseHandle(event);
        }
    }

    #[test]
    fn drop_unfired_wait() {
        let rt = Runtime::new();
        let event = unsafe { check(CreateEventW(None, true, false, None)) };
        for _ in 0..100 {
            drop(unsafe { wait_for_handle(&rt, event) });
        }
        unsafe {
            let _ = CloseHandle(event);
        }
    }
}
