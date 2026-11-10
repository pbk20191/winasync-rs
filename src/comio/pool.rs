//! The one way `comio` puts code on the thread pool.
//!
//! Three paths used to reach the pool, and none of them said whose code they
//! were running: TP_IO registrations went in with a null callback environment,
//! the listener's accept chain re-armed through `windows_threading::submit`,
//! and a busy named pipe waited inside `windows_future::IAsyncOperation::spawn`.
//! All three run code that lives in this module, and in the server DLL that
//! matters twice over.
//!
//! **Counting.** A queued or running work item is code in this module that
//! will run, and `DllCanUnloadNow` must not answer `S_OK` while one exists.
//! TP_IO completions were already counted through `LIVE_OPS`; the other two
//! were not. [`submit`] carries a `ModuleRef` inside the job, so a work item
//! is counted from the moment it is queued until it has finished running.
//!
//! **The race with unloading.** Counting answers the host that asks.
//! [`callback_env`] is for the host that does not: it sets the environment's
//! `RaceDll` -- what `SetThreadpoolCallbackLibrary` sets, the function being
//! an inline field assignment -- to this module's handle.
//!
//! What that buys was measured, because the obvious reading is wrong
//! (`tests/racedll.rs`). It does **not** keep the DLL mapped while I/O is
//! pending: a `FreeLibrary` with a read outstanding unmaps the module whether
//! `RaceDll` is set or not. What it does is settle the race its name refers
//! to. When that read later completes, the pool has a callback for a module
//! that is gone; without `RaceDll` it calls it anyway and the process dies
//! with `STATUS_ACCESS_VIOLATION`, and with `RaceDll` it declines and the
//! process survives, leaking the operation block. In a host that has already
//! broken the protocol, that is the right trade.
//!
//! For a callback that is already *running* when the host unloads, the same
//! mechanism should hold the module until it returns -- the pool cannot
//! decline a call race-free without taking a reference across it. That part
//! is inferred from how the guarantee has to work, not measured; it is what
//! would cover a job's `ModuleRef` dropping in the callback's epilogue, after
//! the count already reads zero.
//!
//! What pinning does not cover is a vtable a host still holds. That is what
//! the object count and `DllCanUnloadNow` are for, and nothing here replaces
//! them.

#[cfg(feature = "dylib")]
use super::unload;
use super::unload::ModuleRef;
use core::ffi::c_void;
use windows::Win32::System::Threading::{
    CloseThreadpoolTimer, CreateThreadpoolTimer, PTP_CALLBACK_INSTANCE, PTP_TIMER,
    SetThreadpoolTimer, TP_CALLBACK_ENVIRON_V3, TrySubmitThreadpoolCallback,
};

/// The callback environment every pool object `comio` creates should use.
///
/// `None` -- the process default, exactly what a null `pcbe` meant before --
/// everywhere but the server DLL. In the rlib there is no module to pin: the
/// code lives in the executable, which outlives every callback anyway.
///
/// Built once and never freed, because the address has to stay valid for the
/// life of every object created with it. That is the conservative reading,
/// and the one `tp::pool` already follows by boxing its environments.
#[cfg(feature = "dylib")]
pub(crate) fn callback_env() -> Option<*const TP_CALLBACK_ENVIRON_V3> {
    use std::sync::OnceLock;
    use windows::Win32::System::Threading::{TP_CALLBACK_ENVIRON_V3_0, TP_CALLBACK_PRIORITY_NORMAL};

    // Raw pointers make the struct `!Sync`; it is written once, here, and
    // only read after.
    struct Env(TP_CALLBACK_ENVIRON_V3);
    unsafe impl Send for Env {}
    unsafe impl Sync for Env {}

    static ENV: OnceLock<Option<Env>> = OnceLock::new();
    ENV.get_or_init(|| {
        // Recorded by `DllMain` on attach, before any code here can run. Null
        // in a `dylib` build that is not the DLL -- the crate's own tests --
        // and caching `None` for that case is correct: it never changes.
        let module = unload::module();
        if module.is_null() {
            return None;
        }
        // Field for field what `InitializeThreadpoolEnvironment` produces,
        // plus `RaceDll`.
        Some(Env(TP_CALLBACK_ENVIRON_V3 {
            Version: 3,
            Pool: Default::default(),
            CleanupGroup: Default::default(),
            CleanupGroupCancelCallback: None,
            RaceDll: module,
            ActivationContext: Default::default(),
            FinalizationCallback: None,
            u: TP_CALLBACK_ENVIRON_V3_0 { Flags: 0 },
            CallbackPriority: TP_CALLBACK_PRIORITY_NORMAL,
            Size: size_of::<TP_CALLBACK_ENVIRON_V3>() as u32,
        }))
    })
    .as_ref()
    .map(|env| &env.0 as *const TP_CALLBACK_ENVIRON_V3)
}

/// [`callback_env`] as a [`PoolRef`], for creating TP objects in it.
pub(crate) fn pool_ref() -> crate::common::pool::PoolRef {
    match callback_env() {
        // SAFETY: the environment is a process-lifetime static.
        Some(env) => unsafe { crate::common::pool::PoolRef::from_static(env) },
        None => crate::common::pool::PoolRef::ProcessDefault,
    }
}

/// Not the server DLL: there is no module to pin.
#[cfg(not(feature = "dylib"))]
pub(crate) fn callback_env() -> Option<*const TP_CALLBACK_ENVIRON_V3> {
    None
}

struct Job {
    run: Box<dyn FnOnce() + Send>,
    _module: ModuleRef,
}

/// Runs `f` on the process thread pool, counted and pinned.
///
/// The replacement for `windows_threading::submit` inside `comio`, which
/// submits with a null environment and carries no count. Never fails: if the
/// pool refuses the item -- in practice only when it cannot allocate -- `f`
/// runs on the calling thread instead. Every caller here prefers a deeper
/// stack, or a caller briefly blocked, to dropping the work.
///
/// A panic in `f` aborts the process, as it did under `windows_threading`:
/// the callback is an `extern "system"` frame, and unwinding out of one is not
/// something Rust permits.
pub(crate) fn submit(f: impl FnOnce() + Send + 'static) {
    let job = Box::into_raw(Box::new(Job {
        run: Box::new(f),
        _module: ModuleRef::new(),
    }));
    let queued = unsafe { TrySubmitThreadpoolCallback(Some(run_job), Some(job.cast()), callback_env()) };
    if queued.is_err() {
        // Not queued, so the pool will never call back: ownership of the job
        // is still ours, and running it here keeps the contract.
        unsafe { run_job(PTP_CALLBACK_INSTANCE::default(), job.cast()) };
    }
}

/// [`submit`], after `delay`: a one-shot thread-pool timer, counted and
/// pinned like any other job.
///
/// If the pool cannot make a timer, `f` waits out `delay` on a pool thread
/// instead -- slower, but the work still happens.
pub(crate) fn submit_after(delay: std::time::Duration, f: impl FnOnce() + Send + 'static) {
    let job = Box::into_raw(Box::new(TimerJob {
        job: Job {
            run: Box::new(f),
            _module: ModuleRef::new(),
        },
        timer: PTP_TIMER::default(),
    }));
    let timer = unsafe { CreateThreadpoolTimer(Some(run_timer_job), Some(job.cast()), callback_env()) };
    let Ok(timer) = timer else {
        let TimerJob { job, .. } = *unsafe { Box::from_raw(job) };
        submit(move || {
            std::thread::sleep(delay);
            (job.run)();
        });
        return;
    };
    unsafe { (*job).timer = timer };
    // A negative due time is relative, in 100 ns ticks.
    let ticks = -(i64::try_from(delay.as_nanos() / 100).unwrap_or(i64::MAX));
    let due = windows::Win32::Foundation::FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    unsafe { SetThreadpoolTimer(timer, Some(&due), 0, None) };
}

struct TimerJob {
    job: Job,
    timer: PTP_TIMER,
}

unsafe extern "system" fn run_timer_job(
    _instance: PTP_CALLBACK_INSTANCE,
    context: *mut c_void,
    _timer: PTP_TIMER,
) {
    let TimerJob { job, timer } = *unsafe { Box::from_raw(context.cast::<TimerJob>()) };
    // Closing from inside the callback is allowed: the timer is released
    // once this callback returns.
    unsafe { CloseThreadpoolTimer(timer) };
    let Job { run, _module } = job;
    run();
}

unsafe extern "system" fn run_job(_instance: PTP_CALLBACK_INSTANCE, context: *mut c_void) {
    let Job { run, _module } = *unsafe { Box::from_raw(context.cast::<Job>()) };
    run();
    // `_module` drops here, still inside this module's code: the count can
    // read zero while this frame has yet to return. See the module docs for
    // what `RaceDll` does and does not promise about that window.
}

#[cfg(all(test, feature = "dylib"))]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// A job waiting to run is code in this module that will run, so the
    /// module must refuse to unload until it has.
    ///
    /// Only the refusal is asserted, not an exact count: the count is
    /// process-wide and other tests in this binary move it concurrently, but
    /// they can only ever add reasons to stay loaded, never remove ours.
    #[test]
    fn a_queued_job_holds_the_module() {
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        submit(move || {
            go_rx.recv().expect("released");
            done_tx.send(()).expect("reported");
        });
        assert!(
            !unload::can_unload(),
            "a submitted job did not hold the module while waiting to run"
        );
        go_tx.send(()).expect("release the job");
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the job never ran");
    }
}
