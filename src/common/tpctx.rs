//! The thread-pool callback-context convention shared by this crate's
//! engines.
//!
//! A TP object (`TP_WORK`, `TP_TIMER`, `TP_WAIT`, ...) carries one opaque
//! context pointer, and something has to free whatever it points at exactly
//! once. Who does the freeing depends on the callback *environment*:
//!
//! * An environment with a `FinalizationCallback` — `tp`'s private pool —
//!   calls [`CtxHeader::release`] when the object is finalized, whichever way
//!   the object was closed. That is the single release path there, and it is
//!   why every such context must begin with this header: the callback reads
//!   the first field as a function pointer and calls it.
//! * A **null environment** — the process default pool, which `rio`'s pump
//!   can use — has no finalization callback. There the owner releases the
//!   context itself after closing the object.
//!
//! The header lives here, rather than in `tp`, so a context can be built
//! without knowing which of the two will free it.

use core::ffi::c_void;

/// Common header of every context pointer handed to a TP object.
///
/// Placing this first in a `#[repr(C)]` struct is what lets a finalization
/// callback free a context whose type it does not know: it casts the context
/// pointer to `*const CtxHeader` and calls `release` with the original
/// pointer. Nothing checks that the cast is correct, so a context built
/// wrongly is a wild indirect call through whatever its first field happens
/// to hold.
pub struct CtxHeader {
    /// Frees the context this header begins. Private so it can only be set
    /// through [`CtxHeader::new`], whose safety contract names the rules.
    release: unsafe fn(*mut c_void),
}

impl CtxHeader {
    /// Builds a header for a context freed by `release`.
    ///
    /// # Safety
    /// All of these must hold, or a finalization callback will make a wild
    /// call or a double free:
    ///
    /// * The struct containing this header must be `#[repr(C)]` with the
    ///   header as its **first** field, so that a `*const CtxHeader` cast of
    ///   the context pointer is valid.
    /// * `release` must be sound to call exactly once with that same context
    ///   pointer, and must free it.
    /// * The context must be handed to **at most one** TP object, since each
    ///   finalization calls `release` once.
    /// * `release` must not itself close or wait on the TP object being
    ///   finalized.
    pub const unsafe fn new(release: unsafe fn(*mut c_void)) -> Self {
        Self { release }
    }

    /// Frees the context starting at `ctx`.
    ///
    /// # Safety
    /// `ctx` must be the pointer this header's own context begins at, and
    /// must not have been released already.
    pub unsafe fn release(&self, ctx: *mut c_void) {
        unsafe { (self.release)(ctx) }
    }
}
