//! The thread-pool wait that wakes the ring's driver when its event fires,
//! and the ring's teardown.
//!
//! The pump touches nothing of the ring's: not its queues, not its slots.
//! It holds the ring's event handle and a waker slot, and when the event
//! fires it wakes whoever registered there -- the task that last polled an
//! operation -- and re-arms. That task pops. This is what lets the ring be
//! a single-owner object with no lock in it: the only state the pool thread
//! shares with the owner is the waker slot, which is lock-free.

use super::*;
use crate::common::atomic_waker::AtomicWaker;

/// The thread-pool wait that wakes the driver when the ring's event fires.
pub(super) struct Pump {
    pub(super) wait: PTP_WAIT,
    /// The wait's context; valid until the wait is closed.
    pub(super) ctx: *const PumpCtx,
    /// `Some` exactly when the environment has no `FinalizationCallback` to
    /// release the context ([`PoolRef::finalizes`]).
    pub(super) ctx_to_release: Option<*const PumpCtx>,
}

/// The pump callback's context. Starts with a [`CtxHeader`] because a `tp`
/// environment's finalization callback treats every context as one.
#[repr(C)]
pub(super) struct PumpCtx {
    pub(super) header: CtxHeader,
    /// Whom to wake: shared with the ring, which registers the driver.
    pub(super) driver: Arc<AtomicWaker>,
    pub(super) event: HANDLE,
    /// Set by [`Pump::stop`] before it drains: a callback that runs during
    /// the drain must not re-arm the wait the close is about to find.
    pub(super) closing: std::sync::atomic::AtomicBool,
    /// The pool the wait lives in; released with the context, after the wait
    /// is closed.
    pub(super) _pool: PoolRef,
}

unsafe impl Send for PumpCtx {}
unsafe impl Sync for PumpCtx {}

pub(super) unsafe fn release_ctx(ctx: *mut core::ffi::c_void) {
    unsafe { drop(Arc::from_raw(ctx as *const PumpCtx)) }
}

impl Pump {
    pub(super) fn start(inner: &Rc<Inner>, pool: PoolRef) -> Result<Pump> {
        // Read before the pool moves into the context.
        let env = pool.env();
        let release_ourselves = !pool.finalizes();
        let ctx = Arc::into_raw(Arc::new(PumpCtx {
            header: unsafe { CtxHeader::new(release_ctx) },
            driver: inner.driver.clone(),
            event: inner.event,
            closing: std::sync::atomic::AtomicBool::new(false),
            _pool: pool,
        }));
        let wait = match unsafe {
            CreateThreadpoolWait(Some(pump_callback), Some(ctx as *mut core::ffi::c_void), env)
        } {
            Ok(wait) => wait,
            Err(e) => {
                unsafe { drop(Arc::from_raw(ctx)) };
                return Err(e);
            }
        };
        unsafe { SetThreadpoolWait(wait, Some(inner.event), None) };
        Ok(Pump {
            wait,
            ctx,
            ctx_to_release: release_ourselves.then_some(ctx),
        })
    }

    /// Stops the wait and frees the context.
    ///
    /// Drained, not cancelled: a callback already queued runs (and wakes a
    /// driver that will find nothing) before the wait is closed. See
    /// `close_wait_draining` for the pool stall this avoids.
    pub(super) fn stop(self) {
        unsafe {
            (*self.ctx)
                .closing
                .store(true, std::sync::atomic::Ordering::Release);
            crate::common::pool::close_wait_draining(self.wait);
        }
        if let Some(ctx) = self.ctx_to_release {
            unsafe { drop(Arc::from_raw(ctx)) };
        }
    }
}

unsafe extern "system" fn pump_callback(
    _instance: PTP_CALLBACK_INSTANCE,
    ctx: *mut core::ffi::c_void,
    wait: PTP_WAIT,
    _result: u32,
) {
    let ctx = unsafe { &*(ctx as *const PumpCtx) };
    // Re-armed first, unless the ring's teardown has begun. The event is
    // signalled when the completion queue goes from empty to non-empty
    // (measured, not documented); the driver pops to empty, so a completion
    // landing after its last pop signals again and the wait fires at once.
    if !ctx.closing.load(std::sync::atomic::Ordering::Acquire) {
        unsafe { SetThreadpoolWait(wait, Some(ctx.event), None) };
    }
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctx.driver.wake()));
}

// ---------------------------------------------------------------------------
// Teardown
// ---------------------------------------------------------------------------

impl Drop for Inner {
    fn drop(&mut self) {
        let pump = self.pump.get_mut().take();
        let ring = self.ring.get_mut();

        // Every operation still in the ring belongs to a future that is gone,
        // and each was cancelled when its future dropped. Wait for the kernel
        // to finish with their buffers; give up after a while rather than
        // hang a destructor, and leak what is left rather than free memory
        // the kernel may still write.
        let _ = submit_locked(ring);
        let mut rounds = 0;
        while ring.in_flight > 0 && rounds < TEARDOWN_ROUNDS {
            let (n, wake, dead) = drain_locked(ring);
            wake.into_iter().for_each(Waker::wake);
            drop(dead);
            if n == 0 {
                rounds += 1;
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        if ring.in_flight > 0 {
            for slot in ring.slots.drain(..) {
                if let SlotState::Waiting { kept, .. } = slot.state {
                    std::mem::forget(kept);
                }
            }
        }

        // The pump never holds the ring, so this cannot be running inside
        // its own callback; stopping it here is a plain drain.
        if let Some(pump) = pump {
            pump.stop();
        }
        unsafe {
            let _ = CloseIoRing(ring.handle);
            let _ = CloseHandle(self.event);
        }
    }
}
