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
//! # And one thing that is not counted but still forbids unloading
//!
//! Live objects are not the only way a module becomes unremovable. Anything
//! it installed in **process-global state** has to be given back too, and
//! `WSAStartup` is the case here.
//!
//! [`crate::common::winsock::wsa_init`] raises the process's Winsock
//! reference the first time any socket transport is constructed, and nothing
//! lowers it. Unloading in that state leaks the reference, and — worse — a
//! later reload gets a fresh `Once` in a fresh copy of the module's data and
//! calls `WSAStartup` *again*, so the count climbs once per load cycle.
//!
//! The obvious fix is a matching `WSACleanup`, and it is not safe here.
//! `unix` and `hvsock` cache `ConnectEx` / `AcceptEx` addresses obtained
//! through `SIO_GET_EXTENSION_FUNCTION_POINTER`, which belong to the Winsock
//! *provider*; a cleanup that takes the refcount to zero may unload that
//! provider and leave the caches — `OnceLock`s, which cannot be reset —
//! pointing at freed code. Trading a leak for a dangling call is the wrong
//! way round.
//!
//! So [`can_unload`] simply says no once Winsock is up. The counting is not
//! wasted: named pipes never touch Winsock, so a pipe-only session stays
//! unloadable, which is the common case. Lifting the restriction means doing
//! both halves — a balanced `WSACleanup` *and* provider-pointer caches that
//! can be re-queried after one — and that is a bigger change than the few
//! hundred resident kilobytes it would buy back.
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

use crate::common::overlapped::LIVE_OPS;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Everything live that is not an in-flight op.
static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);

/// Server locks held through `IClassFactory::LockServer`.
static LOCKS: AtomicUsize = AtomicUsize::new(0);

/// Takes or releases one server lock.
///
/// Kept apart from [`OUTSTANDING`] rather than folded into it because the two
/// fail differently. An object count can only go wrong if this crate has a
/// bug; a lock count can go wrong because a *caller* released a lock it never
/// took, and the subtraction has to survive that. `saturating_sub` is why: a
/// wrapped count would read `usize::MAX` and pin the module loaded for the
/// life of the process, turning somebody else's imbalance into our leak.
pub(crate) fn lock_server(lock: bool) {
    if lock {
        LOCKS.fetch_add(1, Ordering::Relaxed);
    } else {
        let _ = LOCKS.try_update(Ordering::Release, Ordering::Relaxed, |n| {
            Some(n.saturating_sub(1))
        });
    }
}

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
        OUTSTANDING.fetch_sub(1, Ordering::Release);
    }
}

/// Whether this module can be unloaded right now.
///
/// Read in the order that makes a false negative the only possible error: if
/// an op completes between the two loads the answer is a spurious "no", and a
/// spurious "no" costs nothing. The reverse ordering could see both counters
/// at zero in a state where neither was.
pub fn can_unload() -> bool {
    // The flag first: it is the cheapest, it is one-way, and once it is set
    // no amount of releasing objects will make the answer yes again.
    !crate::common::winsock::wsa_initialized()
        && LIVE_OPS.load(Ordering::Acquire) == 0
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
pub fn outstanding() -> usize {
    OUTSTANDING.load(Ordering::Acquire)
}

/// Server locks currently held. See [`outstanding`] for why this is public.
pub fn locks_held() -> usize {
    LOCKS.load(Ordering::Acquire)
}
