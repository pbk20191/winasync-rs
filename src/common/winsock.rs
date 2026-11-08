//! Winsock plumbing shared by every overlapped socket transport — `tp`'s
//! AF_INET/AF_UNIX/AF_HYPERV sockets and `comio`'s AF_UNIX/AF_HYPERV ones.
//!
//! These five functions were copied into each transport as it was written, on
//! the reasoning that the duplication was small. That held at two copies and
//! stopped holding at three: `classify_wsa_overlapped` in particular encodes
//! the overlapped-issue contract, and a divergent copy of *that* is a silent
//! hang, not a cosmetic difference. One copy now.

use super::overlapped::Issue;
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::Networking::WinSock::{
    WSADATA, WSAGetLastError, WSAStartup, WSA_IO_PENDING,
};

/// Process-wide Winsock initialization, deliberately never torn down
/// (`WSACleanup` is not called — a library has no way to know it is the last
/// user). Idempotent and safe to call from every socket constructor.
pub fn wsa_init() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        let mut data = WSADATA::default();
        let r = WSAStartup(0x0202, &mut data);
        assert_eq!(r, 0, "WSAStartup failed: {r}");
    });
}

/// The calling thread's last Winsock error as an `HRESULT`.
pub fn wsa_last_hr() -> windows_core::HRESULT {
    WIN32_ERROR(unsafe { WSAGetLastError() }.0 as u32).to_hresult()
}

/// The calling thread's last Winsock error as an `Error`.
pub fn wsa_last_err() -> windows_core::Error {
    windows_core::Error::from_hresult(wsa_last_hr())
}

/// Maps a Winsock "0 or SOCKET_ERROR" return to a `Result`.
pub fn sock_check(r: i32) -> windows_core::Result<()> {
    if r == 0 {
        Ok(())
    } else {
        Err(wsa_last_err())
    }
}

/// Classifies an overlapped Winsock call from whether it succeeded
/// synchronously.
///
/// Reports only what the call did; whether a completion packet follows is
/// decided by `submit` from the handle's notification mode (see [`Issue`]).
pub fn classify_wsa_overlapped(succeeded: bool) -> Issue {
    if succeeded {
        Issue::SyncSuccess
    } else if unsafe { WSAGetLastError() } == WSA_IO_PENDING {
        Issue::Pending
    } else {
        Issue::Failed(wsa_last_hr())
    }
}

/// [`classify_wsa_overlapped`] for the APIs that return `0` on success and
/// `SOCKET_ERROR` otherwise (`WSARecv`, `WSASend`, ...).
pub fn classify_wsa_ret(r: i32) -> Issue {
    classify_wsa_overlapped(r == 0)
}
