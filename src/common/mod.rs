//! Runtime-neutral primitives shared across the feature-gated runtimes
//! (`tp` / `dq` / `comio`). Nothing here depends on a specific runtime.
#![allow(dead_code)]

pub mod oneshot;

// Overlapped IO + its calling-thread driver are only needed by the overlapped
// runtimes (tp / comio), not by the DispatcherQueue runtime (dq).
#[cfg(any(feature = "tp", feature = "comio", feature = "rio"))]
pub mod block_on;
#[cfg(any(feature = "tp", feature = "comio", feature = "rio"))]
pub mod check;
// AF_HYPERV addressing: pure declarations, shared by both overlapped runtimes'
// Hyper-V socket transports.
#[cfg(any(feature = "tp", feature = "comio"))]
pub mod hvaddr;
#[cfg(any(feature = "tp", feature = "comio"))]
pub mod overlapped;
// Winsock error mapping + the overlapped-issue classifier, shared by every
// socket transport in both overlapped runtimes.
#[cfg(any(feature = "tp", feature = "comio", feature = "rio"))]
pub mod winsock;
// SOCKADDR conversions: neutral, used by every socket engine.
#[cfg(any(feature = "tp", feature = "comio", feature = "rio"))]
pub mod sockaddr;

// Short paths for the two most-used items. Same note as elsewhere: unused
// *here* only because this is a bin crate with no external consumer.
#[cfg(any(feature = "tp", feature = "comio", feature = "rio"))]
#[allow(unused_imports)]
pub use block_on::block_on;
#[cfg(any(feature = "tp", feature = "comio", feature = "rio"))]
#[allow(unused_imports)]
pub use check::check;

