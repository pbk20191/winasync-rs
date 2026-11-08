//! Runtime-neutral primitives shared across the feature-gated runtimes
//! (`tp` / `dq` / `comio`). Nothing here depends on a specific runtime.
#![allow(dead_code)]

pub mod oneshot;

// Overlapped IO + its calling-thread driver are only needed by the overlapped
// runtimes (tp / comio), not by the DispatcherQueue runtime (dq).
#[cfg(any(feature = "tp", feature = "comio"))]
pub mod block_on;
#[cfg(any(feature = "tp", feature = "comio"))]
pub mod check;
#[cfg(any(feature = "tp", feature = "comio"))]
pub mod overlapped;

#[cfg(any(feature = "tp", feature = "comio"))]
pub use block_on::block_on;
#[cfg(any(feature = "tp", feature = "comio"))]
pub use check::check;
