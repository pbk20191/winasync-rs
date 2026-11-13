//! Runtime-neutral primitives shared across the feature-gated runtimes
//! (`tp` / `dq` / `comio`). Nothing here depends on a specific runtime.
#![allow(dead_code)]

pub mod oneshot;
// A lock-free waker slot: what a pump that only notifies hands its wake to,
// and what a RIO request's completion wakes.
#[cfg(any(feature = "ioring", feature = "rio"))]
pub mod atomic_waker;

// Overlapped IO + its calling-thread driver are only needed by the overlapped
// runtimes (tp / comio / http), not by the DispatcherQueue runtime (dq).
#[cfg(any(feature = "tp", feature = "comio", feature = "rio", feature = "ioring", feature = "iocp", feature = "http"))]
pub mod block_on;
#[cfg(any(feature = "tp", feature = "comio", feature = "rio", feature = "ioring", feature = "iocp", feature = "http"))]
pub mod check;
// AF_HYPERV addressing: pure declarations, shared by both overlapped runtimes'
// Hyper-V socket transports.
#[cfg(any(feature = "tp", feature = "comio"))]
pub mod hvaddr;
// Which pool a TP object lives in, with what keeps it valid.
#[cfg(any(feature = "tp", feature = "comio", feature = "iocp", feature = "http", feature = "rio", feature = "ioring"))]
pub mod pool;
// `iocp` for dispatching what the core queues on a caller's port.
#[cfg(any(feature = "tp", feature = "comio", feature = "iocp", feature = "http"))]
pub mod overlapped;
#[cfg(any(feature = "tp", feature = "http"))]
pub mod opfuture;
// Winsock error mapping + the overlapped-issue classifier, shared by every
// socket transport in both overlapped runtimes.
// `iocp` because the overlapped core's socket registrations carry a
// `WinsockRef`.
#[cfg(any(feature = "tp", feature = "comio", feature = "rio", feature = "http", feature = "iocp"))]
pub mod winsock;
// Owned buffers for completion-based IO, plus a slab pool. Not for `comio`,
// whose streams speak WinRT `IBuffer` instead.
#[cfg(any(feature = "tp", feature = "rio", feature = "ioring", feature = "http"))]
pub mod buffer;
// The thread-pool callback-context convention.
#[cfg(any(feature = "tp", feature = "rio", feature = "ioring"))]
pub mod tpctx;
// Access control and peer identity for the local transports (named pipes,
// AF_UNIX), and the named-pipe instance policy built on it.
#[cfg(any(feature = "tp", feature = "comio"))]
pub mod security;
#[cfg(any(feature = "tp", feature = "comio"))]
pub mod pipe;
// SOCKADDR conversions: neutral, used by every socket engine.
#[cfg(any(feature = "tp", feature = "comio", feature = "rio", feature = "http"))]
pub mod sockaddr;

// Short paths for the two most-used items. Same note as elsewhere: unused
// *here* only because this is a bin crate with no external consumer.
#[cfg(any(feature = "tp", feature = "comio", feature = "rio", feature = "ioring", feature = "iocp", feature = "http"))]
#[allow(unused_imports)]
pub use block_on::block_on;
#[cfg(any(feature = "tp", feature = "comio", feature = "rio", feature = "ioring", feature = "iocp", feature = "http"))]
#[allow(unused_imports)]
pub use check::check;

