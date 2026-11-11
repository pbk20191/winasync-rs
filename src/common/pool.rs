//! Which thread pool a TP object is created in, and what keeps that pool's
//! callback environment valid while the object lives.
//!
//! A TP object (`TP_IO`, `TP_WAIT`, ...) is created against a callback
//! environment pointer, and the pool reads that environment for as long as
//! the object exists. So the pointer and whatever keeps it valid travel
//! together, as one [`PoolRef`]: there is no way to hand over an environment
//! and forget its owner.

use windows::Win32::System::Threading::{
    CloseThreadpoolWait, SetThreadpoolWait, WaitForThreadpoolWaitCallbacks, PTP_WAIT,
    TP_CALLBACK_ENVIRON_V3,
};

/// Closes a `TP_WAIT`, letting a callback it has already queued run rather
/// than cancelling it.
///
/// Measured (MikeGrier/windows-threadpool-sys, 2026-09-28..30, and our own
/// `ioring::tests::cold_pool_teardown_sweep`): a `CloseThreadpoolWait` close
/// behind `SetThreadpoolWait(NULL)`, with the wait's callback queued, on a
/// pool that has not made a thread yet, can sever the port's request to the
/// worker factory -- after which that pool dispatches *nothing*, not a wait,
/// a timer or an overlapped completion, until some work item is submitted to
/// it. About 2.5 per 1000 cold starts with `WaitForThreadpoolWaitCallbacks(TRUE)`
/// in between, which cancels through the same kernel routine the close uses
/// (`IopCancelWaitCompletionPacket`); 0 in 20000 with `FALSE`, which runs
/// the queued callback so the close finds nothing to remove. The process
/// default pool is the exposed one: every `IoRing::new` and
/// `RioCq::with_default_pool` teardown reaches here before that pool's first
/// dispatch.
///
/// Disarm and drain twice: a callback running during the first drain may
/// have re-armed on its way out before it saw the caller's closing flag, and
/// the close must not find the wait pending.
///
/// # Safety
/// `wait` is a live wait object, closed by this call. The caller must not be
/// inside its callback (the drain would wait for itself), and the callback
/// must stop re-arming once the caller has decided to close -- a flag set
/// before this is called -- or a callback run by the second drain could arm
/// the wait the close then finds pending.
pub(crate) unsafe fn close_wait_draining(wait: PTP_WAIT) {
    unsafe {
        SetThreadpoolWait(wait, None, None);
        WaitForThreadpoolWaitCallbacks(wait, false);
        SetThreadpoolWait(wait, None, None);
        WaitForThreadpoolWaitCallbacks(wait, false);
        CloseThreadpoolWait(wait);
    }
}

/// A callback environment, and the owner that keeps it valid.
pub enum PoolRef {
    /// The process's default pool: a null environment, which needs no
    /// owner.
    ProcessDefault,
    /// An environment that lives for the rest of the process (comio's, in
    /// the server DLL).
    Static(*const TP_CALLBACK_ENVIRON_V3),
    /// A `tp` runtime's pool; the runtime is held, so its environment is.
    #[cfg(feature = "tp")]
    Runtime(crate::tp::Runtime),
}

// The environment is only read, by the pool.
unsafe impl Send for PoolRef {}
unsafe impl Sync for PoolRef {}

impl PoolRef {
    /// An environment that is never freed.
    ///
    /// # Safety
    /// `env` points at a callback environment that stays valid for the rest
    /// of the process.
    pub unsafe fn from_static(env: *const TP_CALLBACK_ENVIRON_V3) -> PoolRef {
        PoolRef::Static(env)
    }

    /// The environment to create a TP object with; `None` for the process
    /// default pool.
    pub fn env(&self) -> Option<*const TP_CALLBACK_ENVIRON_V3> {
        match self {
            PoolRef::ProcessDefault => None,
            PoolRef::Static(env) => Some(*env),
            #[cfg(feature = "tp")]
            PoolRef::Runtime(rt) => Some(rt.inner.pool.default_env()),
        }
    }

    /// Whether the environment releases a TP object's context itself,
    /// through its `FinalizationCallback` (the `tpctx` convention). When it
    /// does not -- the default pool, or an environment without one --
    /// whoever closes the object releases the context after closing it.
    pub fn finalizes(&self) -> bool {
        // SAFETY: valid per the variant's contract while `self` is alive.
        self.env().is_some_and(|env| unsafe { (*env).FinalizationCallback.is_some() })
    }
}
