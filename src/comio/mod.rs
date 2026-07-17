//! COM components exposing named pipe and AF_UNIX transports through the same
//! WinRT stream interfaces (`IInputStream`/`IOutputStream`/
//! `IAsyncOperationWithProgress`) as `StreamSocket`, backed by Win32 overlapped
//! IO on the default thread-pool callback environment (`pcbe = NULL`).
#![allow(dead_code)]

pub mod asyncop;
pub mod buffer;
pub mod pipe;
pub mod stream;
pub mod tpio;
pub mod waitop;

/// Thread pool / overlapped calls only fail on resource exhaustion; map those
/// to a panic, matching `tp`'s policy.
pub(crate) fn check<T>(r: windows_core::Result<T>) -> T {
    r.expect("comio overlapped allocation failed")
}
