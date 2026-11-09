//! Winsock plumbing shared by every overlapped socket transport — `tp`'s
//! AF_INET/AF_UNIX/AF_HYPERV sockets and `comio`'s AF_UNIX/AF_HYPERV ones.
//!
//! These five functions were copied into each transport as it was written, on
//! the reasoning that the duplication was small. That held at two copies and
//! stopped holding at three: `classify_wsa_overlapped` in particular encodes
//! the overlapped-issue contract, and a divergent copy of *that* is a silent
//! hang, not a cosmetic difference. One copy now.

#[cfg(any(feature = "tp", feature = "comio"))]
use super::overlapped::Issue;
#[cfg(any(feature = "tp", feature = "comio"))]
use core::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::Networking::WinSock::{WSACleanup, WSADATA, WSAGetLastError, WSAStartup};
#[cfg(any(feature = "tp", feature = "comio"))]
use windows::Win32::Networking::WinSock::{WSASocketW, WSA_FLAG_OVERLAPPED};
#[cfg(any(feature = "tp", feature = "comio"))]
use windows::Win32::Networking::WinSock::{
    SIO_GET_EXTENSION_FUNCTION_POINTER, SOCKET, WSAIoctl, WSA_IO_PENDING,
};
#[cfg(any(feature = "tp", feature = "comio"))]
use windows_core::GUID;

/// Whether `WSAStartup` is currently in effect, and nothing else.
struct Startup {
    up: bool,
}

/// Serializes startup, teardown, and socket creation against each other.
///
/// Socket creation is in that list deliberately. `wsa_init()` followed by a
/// separate `WSASocketW` leaves a window in which a teardown can land between
/// them, and the socket then fails with `WSANOTINITIALISED` for no reason the
/// caller can act on. [`socket`] holds this across both.
static STARTUP: Mutex<Startup> = Mutex::new(Startup { up: false });

/// How many times Winsock has been torn down.
///
/// Readable without the lock, and the reason provider function pointers can
/// be cached at all now that teardown exists — see [`ExtFnCache`]. Anything
/// cached before a teardown is stale afterwards, because the provider that
/// owned it may have been unloaded with it.
static TEARDOWNS: AtomicUsize = AtomicUsize::new(0);

/// Set by anything holding provider-derived state it cannot invalidate.
static PINNED: AtomicBool = AtomicBool::new(false);

/// Takes the startup lock, tolerating a poisoned one.
///
/// Poisoning here means a previous caller panicked between `WSAStartup` and
/// releasing the lock. The `up` flag is a single `bool` written after the
/// call succeeded, so it is never left half-updated, and failing every
/// subsequent socket because of one unrelated panic is the worse outcome.
fn lock() -> std::sync::MutexGuard<'static, Startup> {
    STARTUP.lock().unwrap_or_else(|e| e.into_inner())
}

fn ensure_started(state: &mut Startup) {
    if state.up {
        return;
    }
    let mut data = WSADATA::default();
    let r = unsafe { WSAStartup(0x0202, &mut data) };
    assert_eq!(r, 0, "WSAStartup failed: {r}");
    state.up = true;
}

/// Process-wide Winsock initialization. Idempotent and safe to call from
/// every socket constructor.
///
/// Balanced by [`wsa_shutdown`] since 2026-11-09. It was a `Once` that never
/// ran down, on the reasoning that "a library has no way to know it is the
/// last user" — which misreads the API. `WSAStartup` and `WSACleanup` are
/// refcounted precisely so each component balances its own call and nobody
/// has to know about the others.
///
/// What actually blocked teardown was the pointers: extension functions
/// fetched through `SIO_GET_EXTENSION_FUNCTION_POINTER` belong to the
/// *provider*, and they were cached in `OnceLock`s that cannot be reset, so a
/// `WSACleanup` reaching zero could unload the provider and leave those
/// caches aimed at freed code. [`ExtFnCache`] fixes that by stamping each
/// entry with `TEARDOWNS`, which is what made teardown safe to do at all.
///
/// Prefer [`socket`] where you are about to create one: it closes the window
/// between initializing and using.
pub fn wsa_init() {
    ensure_started(&mut lock());
}

/// Creates an overlapped socket, initializing Winsock if needed.
///
/// `flags` is the complete `dwFlags` for `WSASocketW`; `WSA_FLAG_OVERLAPPED`
/// is added for you, since every socket in this crate is overlapped.
#[cfg(any(feature = "tp", feature = "comio"))]
pub fn socket(af: i32, ty: i32, protocol: i32, flags: u32) -> windows_core::Result<SOCKET> {
    let mut state = lock();
    ensure_started(&mut state);
    // Still holding the lock: a teardown between the two would fail this
    // call with `WSANOTINITIALISED`.
    unsafe { WSASocketW(af, ty, protocol, None, 0, WSA_FLAG_OVERLAPPED | flags) }
}

/// Releases this module's Winsock reference.
///
/// Returns whether it actually called `WSACleanup`, so a caller can tell
/// "there was nothing to release" from "something is holding it".
///
/// **Only safe to call when nothing of ours is live.** Every socket this
/// crate owns lives inside an object or an in-flight operation that
/// `comio::unload::can_unload` counts, so that function returning true is the
/// precondition. It is called from `DllCanUnloadNow`, which is not under the
/// loader lock — `DllMain` would be, and `WSACleanup` can unload provider
/// DLLs, which is exactly the deadlock the loader lock exists to prevent.
///
/// Refuses while [`pin`] has been called. Teardown is only sound if every
/// cached provider pointer can be invalidated, and a pin is how a module
/// says it holds one that cannot be.
pub fn wsa_shutdown() -> bool {
    if PINNED.load(Ordering::Acquire) {
        return false;
    }
    let mut state = lock();
    if !state.up {
        return false;
    }
    // Bump before the call, not after: a lookup racing this must already see
    // its cache as stale by the time the provider can go away.
    TEARDOWNS.fetch_add(1, Ordering::Release);
    unsafe {
        let _ = WSACleanup();
    }
    state.up = false;
    true
}

/// Declares that the caller holds provider-derived state it cannot
/// invalidate, which vetoes [`wsa_shutdown`] for the life of the process.
///
/// `rio::table` is the one caller: its `RIO_EXTENSION_FUNCTION_TABLE` is
/// cached behind a `OnceLock` and handed out as `&'static`, so there is no
/// way to retire it. Pinning is the honest answer — a module that cannot give
/// its pointers back should say so rather than let somebody else free the
/// memory under them.
pub fn pin() {
    PINNED.store(true, Ordering::Release);
}

/// Whether `WSAStartup` is currently in effect.
pub fn wsa_initialized() -> bool {
    lock().up
}

/// A cached Winsock extension function pointer.
///
/// Replaces the `OnceLock<usize>` each transport used to declare. The
/// difference is the stamp: a pointer fetched before a `WSACleanup` is not
/// valid after one, because the provider that owned it may have been unloaded
/// in between, and a `OnceLock` has no way to say so.
///
/// A `Mutex` rather than a pair of atomics on purpose. The reader has to see
/// the pointer and its stamp as one value — torn across two atomic loads, a
/// reader could pair a stale pointer with a fresh stamp and call freed code.
/// A seqlock would avoid the lock; it is not worth the subtlety for something
/// consulted once per connect, beside a syscall.
#[cfg(any(feature = "tp", feature = "comio"))]
pub struct ExtFnCache(Mutex<Option<(usize, usize)>>);

#[cfg(any(feature = "tp", feature = "comio"))]
impl ExtFnCache {
    pub const fn new() -> Self {
        Self(Mutex::new(None))
    }
}

#[cfg(any(feature = "tp", feature = "comio"))]
impl Default for ExtFnCache {
    fn default() -> Self {
        Self::new()
    }
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
#[cfg(any(feature = "tp", feature = "comio"))]
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
#[cfg(any(feature = "tp", feature = "comio"))]
pub fn classify_wsa_ret(r: i32) -> Issue {
    classify_wsa_overlapped(r == 0)
}

/// `ConnectEx`'s ABI.
///
/// One declaration for every transport. The *cache* stays per provider (see
/// [`connectex`]), but the signature does not vary, and a second hand-written
/// copy of it is a silent ABI mismatch waiting to happen rather than a
/// stylistic duplication.
#[cfg(any(feature = "tp", feature = "comio"))]
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
#[cfg(any(feature = "tp", feature = "comio"))]
pub fn connectex(sock: SOCKET, cache: &ExtFnCache) -> windows_core::Result<ConnectExFn> {
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
/// `ExtFnCache` keeps the cache per provider at the cost of one extra ioctl
/// per family per process, which is the conservative reading of the contract.
///
/// The cache is also stamped with `TEARDOWNS`, so a `WSACleanup` retires
/// every entry: the pointer belongs to a provider that teardown may have
/// unloaded.
///
/// # Safety
/// `F` must be the exact `extern "system"` signature the extension `id`
/// names, and must be pointer-sized. `cache` must be dedicated to this one
/// `id` *and* to sockets of one provider.
#[cfg(any(feature = "tp", feature = "comio"))]
pub unsafe fn extension_fn<F: Copy>(
    sock: SOCKET,
    id: &GUID,
    cache: &ExtFnCache,
) -> windows_core::Result<F> {
    debug_assert_eq!(
        size_of::<F>(),
        size_of::<*mut c_void>(),
        "extension_fn::<F> requires a bare function pointer"
    );
    // Read the stamp before the ioctl, not after: if a teardown lands in
    // between, the entry we are about to write is already stale and the next
    // caller re-fetches. The reverse order could stamp a stale pointer as
    // fresh.
    let generation = TEARDOWNS.load(Ordering::Acquire);
    if let Some((cached_generation, p)) = *cache.0.lock().unwrap_or_else(|e| e.into_inner())
        && cached_generation == generation
    {
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
    *cache.0.lock().unwrap_or_else(|e| e.into_inner()) = Some((generation, p));
    Ok(unsafe { core::mem::transmute_copy::<usize, F>(&p) })
}
