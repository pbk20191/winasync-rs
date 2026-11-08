//! Winsock plumbing shared by every overlapped socket transport — `tp`'s
//! AF_INET/AF_UNIX/AF_HYPERV sockets and `comio`'s AF_UNIX/AF_HYPERV ones.
//!
//! These five functions were copied into each transport as it was written, on
//! the reasoning that the duplication was small. That held at two copies and
//! stopped holding at three: `classify_wsa_overlapped` in particular encodes
//! the overlapped-issue contract, and a divergent copy of *that* is a silent
//! hang, not a cosmetic difference. One copy now.

use super::overlapped::Issue;
use core::ffi::c_void;
use std::sync::OnceLock;
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::Networking::WinSock::{
    SIO_GET_EXTENSION_FUNCTION_POINTER, SOCKET, WSADATA, WSAGetLastError, WSAIoctl, WSAStartup,
    WSA_IO_PENDING,
};
use windows_core::GUID;

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

/// `ConnectEx`'s ABI.
///
/// One declaration for every transport. The *cache* stays per provider (see
/// [`connectex`]), but the signature does not vary, and a second hand-written
/// copy of it is a silent ABI mismatch waiting to happen rather than a
/// stylistic duplication.
pub type ConnectExFn = unsafe extern "system" fn(
    s: SOCKET,
    name: *const windows::Win32::Networking::WinSock::SOCKADDR,
    namelen: i32,
    lpsendbuffer: *const c_void,
    dwsenddatalength: u32,
    lpdwbytessent: *mut u32,
    lpoverlapped: *mut windows::Win32::System::IO::OVERLAPPED,
) -> windows_core::BOOL;

/// Fetches `ConnectEx` for `sock`'s provider, caching into the caller's
/// `OnceLock`.
///
/// Each transport passes its own cache: the pointer belongs to the socket's
/// service provider, and AF_INET, AF_UNIX and AF_HYPERV are different
/// providers even though stock Windows serves them all from one
/// `mswsock.dll`.
pub fn connectex(sock: SOCKET, cache: &OnceLock<usize>) -> windows_core::Result<ConnectExFn> {
    unsafe {
        extension_fn(
            sock,
            &windows::Win32::Networking::WinSock::WSAID_CONNECTEX,
            cache,
        )
    }
}

/// Fetches a Winsock extension function pointer — `ConnectEx`, `AcceptEx`,
/// `DisconnectEx`, `TransmitPackets`, `WSARecvMsg` — which are reachable only
/// through `WSAIoctl(SIO_GET_EXTENSION_FUNCTION_POINTER)`, never as exports.
///
/// # Why `cache` belongs to the caller
///
/// One GUID per call; there is deliberately no batched variant here.
/// `SIO_GET_MULTIPLE_EXTENSION_FUNCTION_POINTER` looks like one but is not:
/// its input is a single GUID naming a function *table*, and the only table
/// GUID Windows providers accept is `WSAID_MULTIPLE_RIO` (Registered I/O).
/// Measured on Windows 11: asking it for `WSAID_CONNECTEX` — alone or
/// alongside the RIO GUID — fails with `WSAEOPNOTSUPP`, and a second GUID in
/// the input buffer is ignored.
///
/// The pointer is a property of the socket's **service provider**, not of the
/// process: Winsock forwards this ioctl to the provider's `WSPIoctl`, so
/// nothing promises one process-wide value. In practice every Microsoft base
/// provider (TCP/IP, AFUNIX, HvSocket) is served by the same `mswsock.dll` and
/// returns the same address — but a layered provider (LSP) inserted under some
/// protocols and not others can break that. Giving each transport its own
/// `OnceLock` keeps the cache per provider at the cost of one extra ioctl per
/// family per process, which is the conservative reading of the contract.
///
/// # Safety
/// `F` must be the exact `extern "system"` signature the extension `id`
/// names, and must be pointer-sized. `cache` must be dedicated to this one
/// `id` *and* to sockets of one provider.
pub unsafe fn extension_fn<F: Copy>(
    sock: SOCKET,
    id: &GUID,
    cache: &OnceLock<usize>,
) -> windows_core::Result<F> {
    debug_assert_eq!(
        size_of::<F>(),
        size_of::<*mut c_void>(),
        "extension_fn::<F> requires a bare function pointer"
    );
    if let Some(&p) = cache.get() {
        return Ok(unsafe { core::mem::transmute_copy::<usize, F>(&p) });
    }
    let mut raw: *mut c_void = core::ptr::null_mut();
    let mut bytes = 0u32;
    let r = unsafe {
        WSAIoctl(
            sock,
            SIO_GET_EXTENSION_FUNCTION_POINTER,
            Some(id as *const GUID as *const c_void),
            size_of::<GUID>() as u32,
            Some(&mut raw as *mut _ as *mut c_void),
            size_of::<*mut c_void>() as u32,
            &mut bytes,
            None,
            None,
        )
    };
    if r != 0 {
        return Err(wsa_last_err());
    }
    if raw.is_null() {
        // A provider reporting success with a null pointer is broken. Return
        // an error rather than panic: this crate keeps panics away from the
        // FFI paths.
        return Err(windows_core::Error::from_hresult(windows_core::HRESULT(
            0x8000_4003u32 as i32, // E_POINTER
        )));
    }
    let p = raw as usize;
    let _ = cache.set(p);
    Ok(unsafe { core::mem::transmute_copy::<usize, F>(&p) })
}
