//! The thread-pool wait that drains the ring when its event fires, and
//! the teardown that has to reach it.

use super::*;

/// The thread-pool wait that drains the ring when its event fires.
pub(super) struct Pump {
    pub(super) wait: PTP_WAIT,
    /// The wait's context; valid until the wait is closed.
    pub(super) ctx: *const PumpCtx,
    /// `Some` exactly when the environment has no `FinalizationCallback` to
    /// release the context ([`PoolRef::finalizes`]).
    pub(super) ctx_to_release: Option<*const PumpCtx>,
}

// A thread-pool object and a pointer this pump owns.
unsafe impl Send for Pump {}

/// The pump callback's context. Starts with a [`CtxHeader`] because a `tp`
/// environment's finalization callback treats every context as one.
#[repr(C)]
pub(super) struct PumpCtx {
    pub(super) header: CtxHeader,
    /// Weak: the ring's owners decide when it goes, not its pump.
    pub(super) inner: Weak<Inner>,
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

thread_local! {
    /// Set while this thread runs a pump callback: a teardown reached from
    /// inside one must not wait for the callback it is running in.
    static IN_PUMP: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

impl Pump {
    pub(super) fn start(inner: &Arc<Inner>, pool: PoolRef) -> Result<Pump> {
        // Read before the pool moves into the context.
        let env = pool.env();
        let release_ourselves = !pool.finalizes();
        let ctx = Arc::into_raw(Arc::new(PumpCtx {
            header: unsafe { CtxHeader::new(release_ctx) },
            inner: Arc::downgrade(inner),
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

    /// Stops the wait and frees the context. Must not run inside the pump's
    /// own callback.
    ///
    /// Drained, not cancelled: a callback already queued runs (and finds the
    /// ring gone, or drains it one last time) before the wait is closed. See
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

pub(super) struct InPump;

impl Drop for InPump {
    fn drop(&mut self) {
        IN_PUMP.with(|f| f.set(false));
    }
}

unsafe extern "system" fn pump_callback(
    instance: PTP_CALLBACK_INSTANCE,
    ctx: *mut core::ffi::c_void,
    wait: PTP_WAIT,
    _result: u32,
) {
    // A drain can run as long as completions keep coming.
    let _ = unsafe { CallbackMayRunLong(instance) };
    IN_PUMP.with(|f| f.set(true));
    let _in_pump = InPump;
    let ctx = unsafe { &*(ctx as *const PumpCtx) };
    let Some(inner) = ctx.inner.upgrade() else {
        return;
    };
    let (wake, dead) = {
        let mut ring = inner.ring.lock().unwrap();
        let (_, wake, dead) = drain_locked(&mut ring);
        // Re-armed while the ring is certainly alive, unless its teardown
        // has begun. The event is signalled when the completion queue goes
        // from empty to non-empty (measured, not documented), so having
        // popped to empty, a completion posted after the last pop signals
        // it and the wait fires again at once rather than missing it.
        if !ctx.closing.load(std::sync::atomic::Ordering::Acquire) {
            unsafe { SetThreadpoolWait(wait, Some(ctx.event), None) };
        }
        (wake, dead)
    };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wake.into_iter().for_each(Waker::wake);
        drop(dead);
    }));
    // If this was the last owner, `Inner::drop` runs here, with `IN_PUMP`
    // set, and leaves stopping this wait to another thread.
    drop(inner);
}

// ---------------------------------------------------------------------------
// Teardown
// ---------------------------------------------------------------------------

impl Drop for Inner {
    fn drop(&mut self) {
        let pump = self.pump.get_mut().unwrap().take();
        let ring = self.ring.get_mut().unwrap();

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

        let handle = ring.handle;
        let event = self.event;
        let close = move || unsafe {
            let _ = CloseIoRing(handle);
            let _ = CloseHandle(event);
        };
        match pump {
            None => close(),
            Some(pump) if IN_PUMP.with(|f| f.get()) => {
                // Inside our own callback: waiting for it would be waiting
                // for ourselves. Another thread stops it moments later.
                struct Later(Pump, HIORING, HANDLE);
                unsafe impl Send for Later {}
                impl Later {
                    fn run(self) {
                        let Later(pump, handle, event) = self;
                        pump.stop();
                        unsafe {
                            let _ = CloseIoRing(handle);
                            let _ = CloseHandle(event);
                        }
                    }
                }
                let later = Later(pump, handle, event);
                // A method call, so the closure captures `later` whole rather
                // than its non-`Send` fields one by one.
                std::thread::spawn(move || later.run());
            }
            Some(pump) => {
                pump.stop();
                close();
            }
        }
    }
}
