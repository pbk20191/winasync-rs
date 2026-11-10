//! Windows async **engine concepts**, each behind its own cargo feature and
//! usable on its own.
//!
//! | feature | engine |
//! |---|---|
//! | `tp` | a Rust async runtime over the Win32 thread pool — TP_WORK/TIMER/WAIT/IO, with overlapped file and socket IO |
//! | `dq` | a single-threaded runtime whose executor is a WinRT `DispatcherQueue` |
//! | `comio` | WinRT-shaped COM streams over overlapped IO, for transports WinRT itself does not expose |
//! | `rio` | Registered I/O: registered buffers plus a drained completion queue (experimental) |
//! | `ioring` | I/O rings: batched file reads, writes and flushes through a submission and a completion queue |
//!
//! They share [`common`]: the overlapped core, Winsock plumbing, `SOCKADDR`
//! conversions, AF_HYPERV addressing, owned IO buffers, and the thread-pool
//! callback-context convention. Nothing in `common` depends on a particular
//! engine, which is what lets each feature build alone.
//!
//! # Unsafe surface
//!
//! These engines wrap completion-based Win32 IO, so several public items
//! carry real safety contracts rather than decorative ones — notably
//! [`common::buffer::IoBuf`] (a buffer whose address must survive being moved
//! into an operation) and [`common::tpctx::CtxHeader`] (a context layout a
//! finalization callback dereferences blind). Each states its rules; they are
//! not advisory.

pub mod common;

#[cfg(feature = "comio")]
pub mod comio;
#[cfg(feature = "dq")]
pub mod dq;
#[cfg(feature = "ioring")]
pub mod ioring;
#[cfg(feature = "rio")]
pub mod rio;
#[cfg(feature = "tp")]
pub mod tp;
