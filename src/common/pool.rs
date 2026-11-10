//! Which thread pool a TP object is created in, and what keeps that pool's
//! callback environment valid while the object lives.
//!
//! A TP object (`TP_IO`, `TP_WAIT`, ...) is created against a callback
//! environment pointer, and the pool reads that environment for as long as
//! the object exists. So the pointer and whatever keeps it valid travel
//! together, as one [`PoolRef`]: there is no way to hand over an environment
//! and forget its owner.

use windows::Win32::System::Threading::TP_CALLBACK_ENVIRON_V3;

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
