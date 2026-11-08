//! Runtime-neutral primitives shared across the feature-gated runtimes
//! (`tp` / `dq` / `comio`). Nothing here depends on a specific runtime.
#![allow(dead_code)]

pub mod block_on;
pub mod check;
pub mod oneshot;
pub mod overlapped;

pub use block_on::block_on;
pub use check::check;
