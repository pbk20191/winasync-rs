//! Private thread pool with a fully configured `TP_CALLBACK_ENVIRON_V3`
//! (cleanup group, finalization callback, priority, flags, activation
//! context, race DLL) plus the low-level `Pool` closure-submission API.

use super::{check, Runtime, RuntimeInner};
use core::ffi::c_void;
use core::marker::PhantomData;
use core::ops::Deref;
use std::sync::Arc;
use windows::Win32::Foundation::{HANDLE, HMODULE};
use windows::Win32::System::ApplicationInstallationAndServicing::GetCurrentActCtx;
use windows::Win32::System::Threading::{
    CloseThreadpool, CloseThreadpoolCleanupGroup, CloseThreadpoolCleanupGroupMembers,
    CreateThreadpool, CreateThreadpoolCleanupGroup, SetThreadpoolThreadMaximum,
    SetThreadpoolThreadMinimum, TrySubmitThreadpoolCallback, PTP_CALLBACK_INSTANCE,
    TP_CALLBACK_ENVIRON_V3, TP_CALLBACK_ENVIRON_V3_0, TP_CALLBACK_PRIORITY,
    TP_CALLBACK_PRIORITY_HIGH, TP_CALLBACK_PRIORITY_LOW, TP_CALLBACK_PRIORITY_NORMAL,
};

/// Callback priority of a submission, mapping to `TP_CALLBACK_PRIORITY`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Priority {
    High,
    #[default]
    Normal,
    Low,
}

impl Priority {
    fn index(self) -> usize {
        match self {
            Priority::High => 0,
            Priority::Normal => 1,
            Priority::Low => 2,
        }
    }

    fn raw(self) -> TP_CALLBACK_PRIORITY {
        match self {
            Priority::High => TP_CALLBACK_PRIORITY_HIGH,
            Priority::Normal => TP_CALLBACK_PRIORITY_NORMAL,
            Priority::Low => TP_CALLBACK_PRIORITY_LOW,
        }
    }
}

/// Re-exported so `tp`'s own modules keep their `pool::CtxHeader` path; the
/// convention itself is shared (see [`crate::common::tpctx`]), because the
/// process default pool uses the same context layout with a different
/// release path.
pub(crate) use crate::common::tpctx::CtxHeader;

unsafe extern "system" fn finalize_ctx(_: PTP_CALLBACK_INSTANCE, ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    unsafe { ((*(ctx as *const CtxHeader)).release)(ctx) }
}

#[cfg(debug_assertions)]
pub(crate) static CANCELED_CALLBACKS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

// Invoked by CloseThreadpoolCleanupGroupMembers(fCancelPendingCallbacks=TRUE)
// for each pending callback it cancels. Contexts are released by finalize_ctx
// when the member object itself is finalized, so this only observes.
unsafe extern "system" fn on_group_cancel(_object_ctx: *mut c_void, _cleanup_ctx: *mut c_void) {
    #[cfg(debug_assertions)]
    CANCELED_CALLBACKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Owns a `TP_POOL`, its cleanup group, and one callback environment per
/// [`Priority`], all sharing the pool and group. Environments are boxed so
/// their addresses stay stable for the life of every object created with them.
pub(crate) struct PoolInner {
    envs: Box<[TP_CALLBACK_ENVIRON_V3; 3]>,
    default_priority: Priority,
}

unsafe impl Send for PoolInner {}
unsafe impl Sync for PoolInner {}

impl PoolInner {
    fn new(b: &PoolBuilder) -> Self {
        let (pool, group) = unsafe {
            let pool = check(CreateThreadpool(None));
            if let Some(min) = b.min_threads {
                check(SetThreadpoolThreadMinimum(pool, min));
            }
            if let Some(max) = b.max_threads {
                SetThreadpoolThreadMaximum(pool, max);
            }
            (pool, check(CreateThreadpoolCleanupGroup()))
        };

        // Flags bitfield of TP_CALLBACK_ENVIRON_V3: bit 0 = LongFunction, bit 1 = Persistent.
        let mut flags = 0u32;
        if b.long_function {
            flags |= 1;
        }
        if b.persistent {
            flags |= 2;
        }

        let env_for = |p: Priority| TP_CALLBACK_ENVIRON_V3 {
            Version: 3,
            Pool: pool,
            CleanupGroup: group,
            CleanupGroupCancelCallback: Some(on_group_cancel),
            RaceDll: b.race_dll,
            ActivationContext: b.activation_context,
            FinalizationCallback: Some(finalize_ctx),
            u: TP_CALLBACK_ENVIRON_V3_0 { Flags: flags },
            CallbackPriority: p.raw(),
            Size: size_of::<TP_CALLBACK_ENVIRON_V3>() as u32,
        };

        Self {
            envs: Box::new([
                env_for(Priority::High),
                env_for(Priority::Normal),
                env_for(Priority::Low),
            ]),
            default_priority: b.priority,
        }
    }

    pub(crate) fn env(&self, priority: Priority) -> *const TP_CALLBACK_ENVIRON_V3 {
        &self.envs[priority.index()]
    }

    pub(crate) fn default_env(&self) -> *const TP_CALLBACK_ENVIRON_V3 {
        self.env(self.default_priority)
    }

    /// Waits for all outstanding callbacks and closes all remaining member
    /// objects of the cleanup group. Safe only while no wrapper can perform a
    /// concurrent individual close — i.e. from `Pool::join` (simple
    /// submissions only) or from `Drop` (no wrappers left, by Arc chain).
    fn close_members(&self, cancel_pending: bool) {
        unsafe {
            CloseThreadpoolCleanupGroupMembers(
                self.envs[0].CleanupGroup,
                cancel_pending,
                None,
            );
        }
    }

    pub(crate) fn join(&self) {
        self.close_members(false);
    }
}

impl Drop for PoolInner {
    fn drop(&mut self) {
        // By the Arc chain invariant nothing can create members or close them
        // individually anymore; close stragglers (contexts released via
        // finalize_ctx), then the group, then the pool.
        unsafe {
            self.close_members(true);
            CloseThreadpoolCleanupGroup(self.envs[0].CleanupGroup);
            CloseThreadpool(self.envs[0].Pool);
        }
    }
}

/// Context of a `TrySubmitThreadpoolCallback` submission. The callback takes
/// the closure out; the box itself is freed by `finalize_ctx` when the
/// internal work object is finalized (also covering callbacks canceled by
/// `CloseThreadpoolCleanupGroupMembers`, whose closures are then dropped
/// unexecuted).
#[repr(C)]
struct SimpleCtx<F> {
    header: CtxHeader,
    f: Option<F>,
}

unsafe fn release_simple<F>(ctx: *mut c_void) {
    unsafe { drop(Box::from_raw(ctx as *mut SimpleCtx<F>)) }
}

unsafe extern "system" fn simple_callback<F: FnOnce() + Send>(
    _: PTP_CALLBACK_INSTANCE,
    ctx: *mut c_void,
) {
    // A panic here aborts (extern "system"); acceptable for fire-and-forget.
    let ctx = unsafe { &mut *(ctx as *mut SimpleCtx<F>) };
    if let Some(f) = ctx.f.take() {
        f();
    }
}

/// Submits a closure through `TrySubmitThreadpoolCallback` with the given
/// environment.
///
/// # Safety
/// The caller must guarantee the closure outlives its execution (either
/// `'static` or bounded by a scope join) and that `env` stays valid until the
/// internal object is finalized.
pub(crate) unsafe fn try_submit<F: FnOnce() + Send>(
    env: *const TP_CALLBACK_ENVIRON_V3,
    f: F,
) {
    let ctx = Box::into_raw(Box::new(SimpleCtx {
        header: CtxHeader {
            release: release_simple::<F>,
        },
        f: Some(f),
    }));
    unsafe {
        check(TrySubmitThreadpoolCallback(
            Some(simple_callback::<F>),
            Some(ctx as *mut c_void),
            Some(env),
        ));
    }
}

/// Builder for [`Pool`] / [`Runtime`], exposing the `TP_CALLBACK_ENVIRON_V3`
/// knobs: callback priority, LongFunction/Persistent flags, activation
/// context, race DLL, and pool thread limits.
pub struct PoolBuilder {
    priority: Priority,
    long_function: bool,
    persistent: bool,
    activation_context: isize,
    race_dll: *mut c_void,
    min_threads: Option<u32>,
    max_threads: Option<u32>,
}

impl PoolBuilder {
    pub fn new() -> Self {
        Self {
            priority: Priority::Normal,
            long_function: false,
            persistent: false,
            activation_context: 0,
            race_dll: core::ptr::null_mut(),
            min_threads: None,
            max_threads: None,
        }
    }

    /// Default callback priority for submissions on this pool.
    pub fn priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Hints that callbacks run long, so the pool creates worker threads more
    /// eagerly (`Flags.LongFunction`).
    pub fn long_function(mut self, v: bool) -> Self {
        self.long_function = v;
        self
    }

    /// Runs callbacks on a persistent thread — required by APIs with thread
    /// affinity such as `RegNotifyChangeKeyValue` (`Flags.Persistent`).
    pub fn persistent(mut self, v: bool) -> Self {
        self.persistent = v;
        self
    }

    /// Activation context (SxS) to activate around every callback, so pool
    /// threads can e.g. activate manifest-registered WinRT classes exactly
    /// like the thread that owns `hactctx`.
    pub fn activation_context(mut self, hactctx: HANDLE) -> Self {
        self.activation_context = hactctx.0 as isize;
        self
    }

    /// Captures the calling thread's current activation context via
    /// `GetCurrentActCtx` and applies it to every callback.
    pub fn inherit_activation_context(mut self) -> Self {
        let mut ctx = HANDLE::default();
        unsafe { check(GetCurrentActCtx(&mut ctx)) };
        self.activation_context = ctx.0 as isize;
        self
    }

    /// Pins `module` in memory while callbacks are pending (`RaceDll`),
    /// preventing the DLL hosting the callback code from unloading under them.
    pub fn race_dll(mut self, module: HMODULE) -> Self {
        self.race_dll = module.0;
        self
    }

    /// Sets the minimum and maximum number of pool threads.
    pub fn thread_limits(mut self, min: u32, max: u32) -> Self {
        self.min_threads = Some(min);
        self.max_threads = Some(max);
        self
    }

    /// Builds a low-level closure-submission [`Pool`].
    pub fn build(self) -> Pool {
        Pool(Arc::new(PoolInner::new(&self)))
    }

    /// Builds an async [`Runtime`].
    pub fn build_runtime(self) -> Runtime {
        Runtime {
            inner: Arc::new(RuntimeInner {
                pool: PoolInner::new(&self),
            }),
        }
    }
}

impl Default for PoolBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// A private thread pool for plain closure submission, in the style of
/// `windows-threading`. `join`/`scope` use the cleanup group as the barrier
/// waiting for all submitted closures.
pub struct Pool(pub(crate) Arc<PoolInner>);

impl Pool {
    /// Creates a pool with default settings.
    pub fn new() -> Self {
        PoolBuilder::new().build()
    }

    pub fn builder() -> PoolBuilder {
        PoolBuilder::new()
    }

    /// Convenience function for creating a new pool and calling [`scope`][Self::scope].
    pub fn with_scope<'env, F>(f: F)
    where
        F: for<'scope> FnOnce(&'scope Scope<'scope, 'env>),
    {
        Pool::new().scope(f);
    }

    /// Submits the closure to the pool (fire and forget).
    pub fn submit<F: FnOnce() + Send + 'static>(&self, f: F) {
        // Safe: 'static closure, and the internal object holds no reference
        // that outlives the pool because Pool::drop joins first.
        unsafe { try_submit(self.0.default_env(), f) }
    }

    /// Submits the closure at an explicit priority.
    pub fn submit_with_priority<F: FnOnce() + Send + 'static>(&self, priority: Priority, f: F) {
        unsafe { try_submit(self.0.env(priority), f) }
    }

    /// Creates a scope in which non-`'static` closures can be submitted;
    /// waits for all of them (and any other outstanding submissions) before
    /// returning.
    pub fn scope<'env, F>(&self, f: F)
    where
        F: for<'scope> FnOnce(&'scope Scope<'scope, 'env>),
    {
        struct DropGuard<'a>(&'a Pool);
        impl Drop for DropGuard<'_> {
            fn drop(&mut self) {
                self.0.join();
            }
        }
        // Ensure we always join, even if `f` panics.
        let _guard = DropGuard(self);
        let scope = Scope {
            pool: self,
            scope: PhantomData,
            env: PhantomData,
        };
        f(&scope);
    }

    /// Waits for all outstanding submissions to finish
    /// (`CloseThreadpoolCleanupGroupMembers` without canceling).
    pub fn join(&self) {
        self.0.join();
    }
}

impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}

/// A scope for submitting closures that borrow from the environment.
/// See [`Pool::scope`].
pub struct Scope<'scope, 'env: 'scope> {
    pool: &'scope Pool,
    scope: PhantomData<&'scope mut &'scope ()>,
    env: PhantomData<&'env mut &'env ()>,
}

impl<'scope> Scope<'scope, '_> {
    /// Submits a closure that may borrow data outliving the scope.
    pub fn submit<F: FnOnce() + Send + 'scope>(&'scope self, f: F) {
        // Safe: Pool::scope joins before returning, bounding the closure's
        // execution within 'scope.
        unsafe { try_submit(self.pool.0.default_env(), f) }
    }
}

impl Deref for Scope<'_, '_> {
    type Target = Pool;
    fn deref(&self) -> &Self::Target {
        self.pool
    }
}
