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
//!   the first field as a function pointer.
//! * A **null environment** — the process default pool, which `comio` uses
//!   for its IO and `rio` can use for its pump — has no finalization
//!   callback. There the owner releases the context itself after closing the
//!   object.
//!
//! The header lives here, rather than in `tp`, so a context can be built
//! without knowing which of the two will free it.

use core::ffi::c_void;

/// Common header of every context pointer handed to a TP object.
#[repr(C)]
pub struct CtxHeader {
    pub release: unsafe fn(*mut c_void),
}
