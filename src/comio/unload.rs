//! Module lifetime: the one question `DllCanUnloadNow` has to answer.
//!
//! COM asks an in-process server whether it may be unloaded, and the server
//! has to say yes only when nothing it owns can still execute. Unloading a
//! module that still holds live vtables, in-flight overlapped operations or
//! queued thread-pool callbacks is not a leak — it is a jump into freed code.
//!
//! This module keeps the count that makes the answer sayable.
//!
//! # What has to be counted
//!
//! Three kinds of thing, and the third is the one that is easy to forget.
//!
//! **Objects.** Every type this crate builds with `#[implement]` carries a
//! `ModuleRef`. Its vtable is in this module, so a caller holding one can
//! call into freed code. The constructor increments and `Drop` decrements,
//! which is right no matter who did the constructing — a COM activation, a
//! Rust caller, or an accept completion handing back a stream.
//!
//! That last point is why this is sound at all. The count used to be dismissed
//! on the grounds that these types are also constructed from pure Rust paths
//! that have no reason to touch a module count. That was the wrong way round:
//! a Rust-constructed object *also* has a vtable in this module and *also*
//! has to be counted, and in the rlib build nobody reads the total. Counting
//! at construction is correct in both shapes the crate is built as.
//!
//! **Server locks.** `IClassFactory::LockServer(TRUE)` is a caller saying
//! "keep this module loaded even with no objects out". It holds a
//! `ModuleRef` until the matching `LockServer(FALSE)`.
//!
//! **In-flight I/O.** An overlapped operation can be pending with no COM
//! object alive to represent it, and its completion callback is a function
//! pointer into this module. [`crate::common::overlapped::LIVE_OPS`] already
//! counts exactly these — one per op block, incremented at submit and
//! decremented at the release tail — so this module reads it rather than
//! keeping a second one.
//!
//! # And the process-global state that is not counted here at all
//!
//! Live objects are not the only way a module becomes unremovable. Anything
//! it installed in **process-global state** has to be returned too, and
//! `WSAStartup` is the case here: unloading with the reference held leaks
//! it, and a later reload gets a fresh module image that calls `WSAStartup`
//! again, so the count climbs once per load cycle.
//!
//! Nothing in this module returns it, because nothing has to. Every socket
//! this crate owns carries a [`crate::common::winsock::WinsockRef`], whose
//! `Drop` is the `WSACleanup`; the last socket out takes Winsock down with
//! it. The two counts above reaching zero therefore *implies* the Winsock
//! count is zero -- every socket lives inside an object or an in-flight
//! operation -- and `DllCanUnloadNow` has nothing to release when it says
//! yes. The provider-owned caches (`ConnectEx` addresses, the RIO table) are
//! stamped with the number of times that last reference has gone, so a value
//! fetched under one Winsock is never reused under the next.
//!
//! Until 2026-11-13 the reference was a flag raised by the first socket and
//! lowered by a `release_process_state` that `DllCanUnloadNow` called just
//! before answering `S_OK` -- the only place that could establish "nothing of
//! ours is live" from outside, and a place that did not exist without the
//! DLL. The token puts the precondition where it is structural.
//!
//! # The residual race
//!
//! [`can_unload`] is a snapshot, and a snapshot is all COM ever gets. A
//! caller can release the last object on one thread while another asks
//! `DllCanUnloadNow`; the decrement happens inside `Drop`, which still has to
//! return through this module's code after the count reaches zero. The window
//! is small and it is inherent to the API — Microsoft's own guidance treats
//! `DllCanUnloadNow` as advisory for this reason, and `CoFreeUnusedLibraries`
//! defers the actual unload rather than acting on the answer immediately.
//!
//! What this module can promise is the half that is ours: a `S_OK` is never
//! returned while anything counted above is outstanding. Returning a *false*
//! `S_OK` is the crash; returning a false `S_FALSE` only costs a resident
//! module until the next sweep.

//! # Nothing here exists without `dylib`
//!
//! Every static below is gated on the feature, not merely left unread
//! without it. A count nobody can observe is still process-wide mutable
//! state shared by every consumer in the process, and `ModuleRef` is a
//! zero-sized no-op in that build — the field the twelve `#[implement]` types
//! carry compiles to nothing, so the gating costs no `cfg` at any of them.

#[cfg(feature = "dylib")]
use crate::common::overlapped::LIVE_OPS;
#[cfg(feature = "dylib")]
use std::sync::atomic::Ordering;
#[cfg(feature = "dylib")]
use std::sync::atomic::AtomicUsize;

/// Everything live that is not an in-flight op.
#[cfg(feature = "dylib")]
static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);

/// Server locks held through `IClassFactory::LockServer`.
#[cfg(feature = "dylib")]
static LOCKS: AtomicUsize = AtomicUsize::new(0);

/// Takes or releases one server lock.
///
/// Kept apart from [`OUTSTANDING`] rather than folded into it because the two
/// fail differently. An object count can only go wrong if this crate has a
/// bug; a lock count can go wrong because a *caller* released a lock it never
/// took, and the subtraction has to survive that. `saturating_sub` is why: a
/// wrapped count would read `usize::MAX` and pin the module loaded for the
/// life of the process, turning somebody else's imbalance into our leak.
#[cfg(feature = "dylib")]
pub(crate) fn lock_server(lock: bool) {
    if lock {
        LOCKS.fetch_add(1, Ordering::Relaxed);
    } else {
        let _ = LOCKS.try_update(Ordering::Release, Ordering::Relaxed, |n| {
            Some(n.saturating_sub(1))
        });
    }
}

/// `LockServer` still succeeds without the feature: a caller locking a server
/// that never offers to unload has lost nothing.
#[cfg(not(feature = "dylib"))]
pub(crate) fn lock_server(_lock: bool) {}

/// One reason this module must stay loaded.
///
/// Held as a field by every `#[implement]` type in `comio`, and by the guard
/// `LockServer` keeps. Cloning makes a second reason rather than sharing one,
/// which is what a field in a second object needs.
pub(crate) struct ModuleRef(());

impl ModuleRef {
    pub(crate) fn new() -> Self {
        // Relaxed is enough on the way up: the object being constructed is
        // not reachable by another thread until after this returns, so there
        // is nothing for this increment to be ordered against.
        #[cfg(feature = "dylib")]
        OUTSTANDING.fetch_add(1, Ordering::Relaxed);
        Self(())
    }
}

impl Default for ModuleRef {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ModuleRef {
    fn clone(&self) -> Self {
        Self::new()
    }
}

impl Drop for ModuleRef {
    fn drop(&mut self) {
        // Release, paired with the Acquire in `can_unload`: everything this
        // object did must be visible to a thread that goes on to observe the
        // count at zero and unload the module.
        #[cfg(feature = "dylib")]
        OUTSTANDING.fetch_sub(1, Ordering::Release);
    }
}

/// Whether this module can be unloaded right now.
///
/// Read in the order that makes a false negative the only possible error: if
/// an op completes between the two loads the answer is a spurious "no", and a
/// spurious "no" costs nothing. The reverse ordering could see both counters
/// at zero in a state where neither was.
#[cfg(feature = "dylib")]
pub fn can_unload() -> bool {
    LIVE_OPS.load(Ordering::Acquire) == 0
        && OUTSTANDING.load(Ordering::Acquire) == 0
        && LOCKS.load(Ordering::Acquire) == 0
}

/// The live object count.
///
/// Not the same thing as [`can_unload`]: this is the object half only.
///
/// Public because the counter is process-wide, and a test that measures a
/// process-wide counter needs a process of its own — `tests/unload.rs` is an
/// integration test for exactly that reason. As a unit test it would race
/// every other test in the crate that builds an object.
#[cfg(feature = "dylib")]
pub fn outstanding() -> usize {
    OUTSTANDING.load(Ordering::Acquire)
}

/// Server locks currently held. See [`outstanding`] for why this is public.
#[cfg(feature = "dylib")]
pub fn locks_held() -> usize {
    LOCKS.load(Ordering::Acquire)
}

/// The body of the server DLL's `DllMain`.
///
/// Lives here, not in the DLL crate, because it reads this module's counters.
/// The `winasync-dll` package forwards to it from the one `DllMain` there is;
/// this library defines no `DllMain` of its own and no exported symbol of any
/// kind, which is what makes "only the cdylib has these" a fact about the
/// build rather than a convention.
///
/// It exists for one job, and only in debug builds: **catching a module that
/// was unloaded while it still owned something.** `DllCanUnloadNow` is
/// advisory, so a host is free to `FreeLibrary` the module without asking,
/// and if it does so while an object or an operation is live the failure is a
/// jump into freed code somewhere else entirely, minutes later, with no trace
/// back to here. Checking at the moment of detach turns that into a message
/// naming the counts.
///
/// There is deliberately no cleanup here. `DllMain` runs under the loader
/// lock, where the useful things -- `WSACleanup`, waiting on thread-pool
/// callbacks, anything that could load or unload a module -- are exactly what
/// must not be called. Observing is safe; acting is not. There is also
/// nothing left to clean: a socket's `Drop` releases its own Winsock
/// reference, and a module with no sockets holds none.
///
/// Returns the `BOOL` `DllMain` must return; anything but TRUE fails the
/// load.
///
/// # Safety
///
/// Only the server DLL's `DllMain` may call this, with the arguments the
/// loader passed it.
///
/// That is a contract, not the whole defence. `DllMain` stays exported -- and
/// in the import library, so it can be linked against -- which means anyone
/// can call the DLL's copy with arguments of their choosing. So the first
/// attach wins and every later one is ignored: the loader's call always comes
/// first, and a forged handle would otherwise become the `RaceDll` the thread
/// pool trusts. A detach is only believed for the handle that attached.
#[doc(hidden)]
#[cfg(feature = "dylib")]
pub unsafe fn dll_main(
    instance: *mut core::ffi::c_void,
    reason: u32,
    reserved: *mut core::ffi::c_void,
) -> i32 {
    const DLL_PROCESS_DETACH: u32 = 0;
    const DLL_PROCESS_ATTACH: u32 = 1;

    match reason {
        DLL_PROCESS_ATTACH => {
            // Before anything else in the module can run, so every reader of
            // `module()` -- `pool::callback_env` above all -- sees it set.
            let _ = MODULE.compare_exchange(
                core::ptr::null_mut(),
                instance,
                Ordering::Release,
                Ordering::Relaxed,
            );
        }
        // `reserved` is null when the module is being unloaded by
        // `FreeLibrary`, and non-null when the process is exiting. Only the
        // first is a mistake: at process exit nothing will run again, so
        // outstanding objects are not a dangling-call risk.
        DLL_PROCESS_DETACH if reserved.is_null() && instance == module() => on_forced_unload(),
        _ => {}
    }
    1
}

/// Reports a module that is being unloaded with work outstanding.
///
/// Split out of [`dll_main`] rather than written inline so the release build
/// is an empty call instead of a `cfg`-gated branch inside a match arm.
#[cfg(all(feature = "dylib", debug_assertions))]
fn on_forced_unload() {
    if can_unload() {
        return;
    }
    // Not a panic: this runs in an `extern "system"` frame under the loader
    // lock, where unwinding is the one thing worse than the bug being
    // reported.
    eprintln!(
        concat!(
            "FATAL: {} unloaded with work outstanding -- {} object(s), ",
            "{} server lock(s), {} operation(s) in flight{}. Any interface ",
            "the host still holds now points into freed code.",
        ),
        env!("CARGO_PKG_NAME"),
        OUTSTANDING.load(Ordering::Acquire),
        LOCKS.load(Ordering::Acquire),
        LIVE_OPS.load(Ordering::Acquire),
        if crate::common::winsock::wsa_initialized() {
            ", and Winsock was initialized"
        } else {
            ""
        },
    );
    std::process::abort();
}

/// Release builds make no claim about this; `DllCanUnloadNow` is advisory and
/// a host that ignores it is not doing anything the API forbids.
#[cfg(all(feature = "dylib", not(debug_assertions)))]
fn on_forced_unload() {}

/// This module's handle, recorded by [`dll_main`] on attach. Null in the
/// rlib, where no loader ever calls it.
///
/// A handle rather than a flag because `pool::callback_env` needs it: it is
/// the `RaceDll` the thread pool pins while our callbacks are outstanding.
/// The loader hands it over for free on attach, which beats asking for it
/// later with `GetModuleHandleExW(FROM_ADDRESS)`.
#[cfg(feature = "dylib")]
static MODULE: std::sync::atomic::AtomicPtr<core::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(core::ptr::null_mut());

/// This module's `HMODULE`, or null when the code is not running from the
/// server DLL.
#[cfg(feature = "dylib")]
pub(crate) fn module() -> *mut core::ffi::c_void {
    MODULE.load(Ordering::Acquire)
}

/// Whether this code is running from the server DLL rather than linked into
/// an executable. Diagnostic only; nothing branches on it.
#[cfg(feature = "dylib")]
pub fn in_dll() -> bool {
    !module().is_null()
}
