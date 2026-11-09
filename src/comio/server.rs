//! The out-of-process server: the six classes, served from a process of
//! their own.
//!
//! The in-process server is a DLL the client loads; this is the other shape
//! COM defines. A client calls `CoCreateInstance(CLSCTX_LOCAL_SERVER)`, COM
//! starts the executable registered under the class's `LocalServer32` key,
//! and that executable -- `winasync-server.exe`, package `server/` -- calls
//! [`run`]. Every call after that crosses the process boundary through the
//! proxy/stub in `winasync_ps.dll` (package `ps/`).
//!
//! # Lifetime
//!
//! The process lives exactly as long as something in it is in use, and the
//! count of "in use" is the one the DLL already keeps: every object, queued
//! pool job and in-flight operation holds a `ModuleRef`. Here a `ModuleRef`
//! takes a reference with `CoAddRefServerProcess` instead of bumping a
//! counter, which is the documented pattern for exactly this, and for one
//! reason above all: when `CoReleaseServerProcess` takes the count to zero,
//! COM itself suspends the class objects *atomically with* that decrement. An
//! activation arriving a moment later is refused with `CO_E_SERVER_STOPPING`
//! and COM starts a fresh server for it, rather than handing the request to a
//! process already on its way out. Keeping our own counter would reopen
//! exactly that window.
//!
//! In-flight operations count, which is the point: an operation's completion
//! runs code here, and the process must not exit underneath it. Class objects
//! do not -- they are registered for the life of the process, so counting
//! them would keep the count above zero forever (`ModuleRef::factory`).
//!
//! # Starting idle
//!
//! COM starts the server because a client asked for an object, so an object
//! normally follows within milliseconds. If that client dies in between, no
//! reference is ever taken and none is ever released, and nothing would end
//! the process. [`IDLE_TIMEOUT`] does: a server that has never been used by
//! then exits.
//!
//! # What it does not cover
//!
//! A client that dies while holding references. COM reclaims those through
//! its ping-based rundown, which takes minutes by design; until then the
//! server stays up, and that is COM working as specified, not a leak here.

use super::activation::{CLASSES, class_object};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, CoAddRefServerProcess, CoInitializeEx,
    CoRegisterClassObject, CoReleaseServerProcess, CoResumeClassObjects, CoRevokeClassObject,
    CoSuspendClassObjects, CoUninitialize, REGCLS_MULTIPLEUSE, REGCLS_SUSPENDED,
};
use windows_core::Result;

/// How long a server that has never been used waits before exiting.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Whether this process is serving. Only then does a `ModuleRef` take a
/// server-process reference; before [`run`] starts and after it finishes,
/// objects are ordinary objects.
static SERVING: AtomicBool = AtomicBool::new(false);

/// Whether any reference was ever taken -- the difference between a server
/// that is idle because it finished and one that was never used.
static USED: AtomicBool = AtomicBool::new(false);

/// Server locks taken through `IClassFactory::LockServer`, so a caller's
/// unbalanced unlock cannot release a reference some object holds.
static LOCKS: AtomicUsize = AtomicUsize::new(0);

/// Set when the last reference goes; [`run`] waits on it.
static FINISHED: Mutex<bool> = Mutex::new(false);
static WAKE: Condvar = Condvar::new();

/// Takes one server-process reference, if serving. Returns whether it did,
/// which the caller must remember to give the reference back.
pub(crate) fn add_ref() -> bool {
    if !SERVING.load(Ordering::Acquire) {
        return false;
    }
    unsafe { CoAddRefServerProcess() };
    USED.store(true, Ordering::Release);
    true
}

/// Gives one back. At zero COM has already suspended the class objects, and
/// all that is left is to wake [`run`] so it can tear down.
pub(crate) fn release() {
    if unsafe { CoReleaseServerProcess() } == 0 {
        *FINISHED.lock().unwrap_or_else(|e| e.into_inner()) = true;
        WAKE.notify_all();
    }
}

/// `IClassFactory::LockServer` in the out-of-process server: a lock is a
/// process reference like any other.
pub(crate) fn lock(lock: bool) {
    if lock {
        if add_ref() {
            LOCKS.fetch_add(1, Ordering::Relaxed);
        }
    } else if LOCKS
        .try_update(Ordering::AcqRel, Ordering::Relaxed, |n| n.checked_sub(1))
        .is_ok()
    {
        release();
    }
}

/// Serves the six classes until nothing is using them, then returns.
///
/// Called once, from the server executable's `main`, on a thread that has
/// not initialized COM. Returns when the last reference is released, or after
/// [`IDLE_TIMEOUT`] if none was ever taken.
pub fn run() -> Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
    SERVING.store(true, Ordering::Release);

    let served = serve();

    SERVING.store(false, Ordering::Release);
    // The process reference Winsock took for any socket transport. Every
    // object is gone by now, so no socket is.
    crate::common::winsock::wsa_shutdown();
    unsafe { CoUninitialize() };
    served
}

fn serve() -> Result<()> {
    // Registered suspended and resumed together, so no activation can reach
    // one class while another is still unregistered.
    let mut cookies = Vec::with_capacity(CLASSES.len());
    let registered = (|| {
        for class in CLASSES {
            let factory = class_object(class);
            cookies.push(unsafe {
                CoRegisterClassObject(
                    &class.clsid,
                    &factory,
                    CLSCTX_LOCAL_SERVER,
                    REGCLS_MULTIPLEUSE | REGCLS_SUSPENDED,
                )
            }?);
        }
        unsafe { CoResumeClassObjects() }
    })();

    if registered.is_ok() {
        wait();
    }

    for cookie in cookies {
        let _ = unsafe { CoRevokeClassObject(cookie) };
    }
    registered
}

fn wait() {
    let mut finished = FINISHED.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if *finished {
            return;
        }
        if USED.load(Ordering::Acquire) {
            finished = WAKE.wait(finished).unwrap_or_else(|e| e.into_inner());
            continue;
        }
        let (guard, timeout) = WAKE
            .wait_timeout(finished, IDLE_TIMEOUT)
            .unwrap_or_else(|e| e.into_inner());
        finished = guard;
        if timeout.timed_out() && !USED.load(Ordering::Acquire) {
            // Stop taking activations before the final look, so the answer
            // cannot change after it is read. One already inside
            // `CreateInstance` when we suspend may still complete and be
            // refused on its way out; COM answers that client by starting a
            // fresh server, which is what this race costs.
            let _ = unsafe { CoSuspendClassObjects() };
            if !USED.load(Ordering::Acquire) {
                return;
            }
            let _ = unsafe { CoResumeClassObjects() };
        }
    }
}
