//! Winsock plumbing shared by every overlapped socket transport — `tp`'s
//! AF_INET/AF_UNIX/AF_HYPERV sockets and `comio`'s AF_UNIX/AF_HYPERV ones.
//!
//! These five functions were copied into each transport as it was written, on
//! the reasoning that the duplication was small. That held at two copies and
//! stopped holding at three: `classify_wsa_overlapped` in particular encodes
//! the overlapped-issue contract, and a divergent copy of *that* is a silent
//! hang, not a cosmetic difference. One copy now.
//!
//! # Winsock's lifetime is a reference, not a flag
//!
//! `WSAStartup` and `WSACleanup` are refcounted by `ws2_32` itself, so the API
//! already has the shape of an `Arc`: each user balances its own pair and
//! nobody needs to know who else is in the process. This module follows that
//! shape with [`WinsockRef`], a zero-sized token: the first one calls
//! `WSAStartup`, the last one dropped calls `WSACleanup`, and the ones in
//! between only count. Every socket this crate owns carries one, so Winsock
//! is up exactly while something of ours needs it and comes down with the
//! last socket -- in the server DLL, in the static library and in a plain
//! `tp` executable alike, with no entry point having to decide when.
//!
//! Until 2026-11-13 this was a `Mutex<bool>` raised by the first socket and
//! lowered by `DllCanUnloadNow`, which had to establish "nothing of ours is
//! live" from the outside and could not be reached at all without the DLL.
//! The token makes that precondition structural: a socket that exists holds a
//! reference, so the count cannot be zero while it does.

#[cfg(any(feature = "tp", feature = "comio"))]
use super::overlapped::Issue;
#[cfg(any(feature = "tp", feature = "comio"))]
use core::ffi::c_void;
use std::os::windows::io::{AsRawSocket, FromRawSocket, OwnedSocket};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::Networking::WinSock::{WSACleanup, WSADATA, WSAGetLastError, WSAStartup};
use windows::Win32::Networking::WinSock::SOCKET;
#[cfg(any(feature = "tp", feature = "comio", feature = "rio"))]
use windows::Win32::Networking::WinSock::{WSASocketW, WSA_FLAG_OVERLAPPED};
#[cfg(any(feature = "tp", feature = "comio"))]
use windows::Win32::Networking::WinSock::{
    SIO_GET_EXTENSION_FUNCTION_POINTER, WSAIoctl, WSA_IO_PENDING,
};
#[cfg(any(feature = "tp", feature = "comio"))]
use windows_core::GUID;

/// How many [`WinsockRef`]s are alive.
///
/// Under a lock rather than an atomic because the two transitions that
/// matter -- 0 to 1 and 1 to 0 -- each pair a count change with a Winsock
/// call, and the pair has to be indivisible: a second taker that saw the
/// count at 1 before the first taker's `WSAStartup` returned would create
/// its socket into `WSANOTINITIALISED`. Every other transition is a bare
/// increment or decrement under an uncontended lock, which is cheaper than
/// the `WSAStartup` call it replaces.
static REFS: Mutex<usize> = Mutex::new(0);

/// Takes the count's lock, tolerating a poisoned one: the count is updated
/// only after the Winsock call it pairs with succeeded, so it is never left
/// half-written.
fn refs() -> std::sync::MutexGuard<'static, usize> {
    REFS.lock().unwrap_or_else(|e| e.into_inner())
}

/// How many times our last Winsock reference has been released.
///
/// The stamp on every provider-derived cache -- see [`ProviderCache`]. Our
/// count reaching zero is the moment `WSACleanup` can reach *its* zero and
/// unload a provider, so anything cached before that is stale after it.
static TEARDOWNS: AtomicUsize = AtomicUsize::new(0);

/// One reason Winsock must stay up.
///
/// The first one calls `WSAStartup`, the last one out calls `WSACleanup`;
/// the ones in between only count. Zero-sized, and held as a field by
/// everything that owns a socket. Cloning takes a reference of its own
/// rather than sharing one, which is what a field in a second object needs.
///
/// The ordering argument that makes the provider caches sound rests on this
/// token: a cached pointer is only ever *called* by something that owns a
/// socket, that thing holds a token, and while any token exists the count is
/// nonzero and no release of ours can be the last. The caches are stamped so
/// that a pointer fetched before a last release is never reused after it.
pub struct WinsockRef(());

impl WinsockRef {
    /// Takes a Winsock reference. Panics if `WSAStartup` fails, which on a
    /// supported Windows means the process is already in trouble.
    pub fn new() -> Self {
        let mut refs = refs();
        if *refs == 0 {
            let mut data = WSADATA::default();
            let r = unsafe { WSAStartup(0x0202, &mut data) };
            assert_eq!(r, 0, "WSAStartup failed: {r}");
        }
        *refs += 1;
        Self(())
    }
}

impl Default for WinsockRef {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for WinsockRef {
    fn clone(&self) -> Self {
        Self::new()
    }
}

impl Drop for WinsockRef {
    fn drop(&mut self) {
        let mut refs = refs();
        *refs -= 1;
        if *refs == 0 {
            // The last of ours: bump the stamp *before* the call, so a
            // lookup racing this already sees its cache as stale by the time
            // the provider can go away. A lookup that holds its own token
            // cannot be racing a last release at all -- see the type's doc.
            TEARDOWNS.fetch_add(1, Ordering::Release);
            // Held across the call on purpose: a taker arriving now waits
            // for the cleanup to finish and then starts Winsock afresh,
            // rather than racing its `WSAStartup` against this.
            unsafe {
                let _ = WSACleanup();
            }
        }
    }
}

/// Whether any [`WinsockRef`] is alive, i.e. whether `WSAStartup` is in
/// effect on this crate's account.
pub fn wsa_initialized() -> bool {
    *refs() > 0
}

/// An overlapped socket together with the Winsock reference that keeps it
/// valid.
///
/// What [`socket`] returns. The reference is taken *before* `WSASocketW`
/// and lives as long as the socket, so there is no moment at which the
/// socket exists and a last release of ours could `WSACleanup` it away. The
/// socket closes on drop, which is also what makes early returns in the
/// constructors clean without a guard.
///
/// Field order is the drop order: the socket first, the reference after.
pub struct WsaSocket {
    sock: OwnedSocket,
    winsock: WinsockRef,
}

impl WsaSocket {
    /// Adopts a socket somebody else created, taking a reference for it.
    ///
    /// # Safety
    /// `sock` is open, owned by the caller, and closed by nothing else.
    pub unsafe fn from_raw(sock: SOCKET) -> Self {
        // The reference first, so the count is nonzero before the socket is
        // ours.
        let winsock = WinsockRef::new();
        Self {
            sock: unsafe { OwnedSocket::from_raw_socket(sock.0 as u64) },
            winsock,
        }
    }

    /// Adopts an owned socket, taking a reference for it.
    pub fn from_owned(sock: OwnedSocket) -> Self {
        let winsock = WinsockRef::new();
        Self { sock, winsock }
    }

    /// Reassembles what [`into_parts`](Self::into_parts) split.
    pub fn from_parts(sock: OwnedSocket, winsock: WinsockRef) -> Self {
        Self { sock, winsock }
    }

    /// The socket, for the Winsock calls that want the raw value.
    pub fn raw(&self) -> SOCKET {
        SOCKET(self.sock.as_raw_socket() as usize)
    }

    /// Splits into the socket and its reference. Drop the reference after
    /// the socket, never before.
    pub fn into_parts(self) -> (OwnedSocket, WinsockRef) {
        (self.sock, self.winsock)
    }
}

/// Creates an overlapped socket with a Winsock reference of its own.
///
/// `flags` is the complete `dwFlags` for `WSASocketW`; `WSA_FLAG_OVERLAPPED`
/// is added for you, since every socket in this crate is overlapped.
#[cfg(any(feature = "tp", feature = "comio", feature = "rio"))]
pub fn socket(af: i32, ty: i32, protocol: i32, flags: u32) -> windows_core::Result<WsaSocket> {
    let winsock = WinsockRef::new();
    let sock = unsafe { WSASocketW(af, ty, protocol, None, 0, WSA_FLAG_OVERLAPPED | flags) }?;
    Ok(WsaSocket {
        sock: unsafe { OwnedSocket::from_raw_socket(sock.0 as u64) },
        winsock,
    })
}

/// A cached value that belongs to a Winsock provider and is only valid while
/// that provider is loaded: an extension function pointer, or RIO's whole
/// function table.
///
/// A provider can be unloaded by the `WSACleanup` that takes the process's
/// count to zero, and ours reaching zero is the only way that can happen on
/// our account. So every entry carries the teardown stamp it was fetched
/// under -- how many times a last [`WinsockRef`] has been released -- and a
/// mismatch means "fetch again". That is the whole difference
/// from a `OnceLock`, which has no way to say its value has expired.
///
/// A `Mutex` rather than a pair of atomics on purpose. The reader has to see
/// the value and its stamp as one thing -- torn across two atomic loads, a
/// reader could pair a stale pointer with a fresh stamp and call freed code.
/// A seqlock would avoid the lock; it is not worth the subtlety for something
/// consulted once per connect, beside a syscall.
pub struct ProviderCache<T>(Mutex<Option<(usize, T)>>);

impl<T: Copy> ProviderCache<T> {
    pub const fn new() -> Self {
        Self(Mutex::new(None))
    }

    /// The cached value if its stamp is current, else whatever `fetch`
    /// produces, cached under the stamp read *before* the fetch.
    ///
    /// Reading the stamp first matters: if a last release lands between the
    /// read and the fetch, the entry is written already-stale and the next
    /// caller re-fetches. The reverse order could stamp a stale value as
    /// fresh. (A caller holding a token makes that race impossible, but the
    /// cache does not know who holds what and is correct either way.)
    pub fn get_or_fetch(
        &self,
        fetch: impl FnOnce() -> windows_core::Result<T>,
    ) -> windows_core::Result<T> {
        let generation = TEARDOWNS.load(Ordering::Acquire);
        if let Some((cached_generation, v)) = *self.0.lock().unwrap_or_else(|e| e.into_inner())
            && cached_generation == generation
        {
            return Ok(v);
        }
        let v = fetch()?;
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some((generation, v));
        Ok(v)
    }
}

impl<T: Copy> Default for ProviderCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// A cached Winsock extension function pointer, stored as its address.
pub type ExtFnCache = ProviderCache<usize>;

/// The calling thread's last Winsock error as an `HRESULT`.
pub fn wsa_last_hr() -> windows_core::HRESULT {
    WIN32_ERROR(unsafe { WSAGetLastError() }.0 as u32).to_hresult()
}

/// The calling thread's last Winsock error as an `Error`.
pub fn wsa_last_err() -> windows_core::Error {
    windows_core::Error::from_hresult(wsa_last_hr())
}

/// The `backlog` every listener in this crate passes to `listen`.
///
/// Not the `windows` crate's `SOMAXCONN`: that binding comes from the old
/// `winsock.h` and is **5**. `winsock2.h` redefines it as `0x7fffffff`,
/// which the provider reads as "the maximum reasonable value" (200 on
/// desktop Windows); the benchmark harness found the difference as 500 ms
/// SYN retransmits on the sixth concurrent connect. This is the
/// `winsock2.h` value.
pub const LISTEN_BACKLOG: i32 = 0x7fff_ffff;

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
/// cache.
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
/// The cache is stamped (see [`ProviderCache`]), so a last release of our
/// Winsock reference retires every entry: the pointer belongs to a provider
/// that the `WSACleanup` may have unloaded.
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
    let p = cache.get_or_fetch(|| {
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
            // A provider reporting success with a null pointer is broken.
            // Return an error rather than panic: this crate keeps panics
            // away from the FFI paths.
            return Err(windows_core::Error::from_hresult(windows_core::HRESULT(
                0x8000_4003u32 as i32, // E_POINTER
            )));
        }
        Ok(raw as usize)
    })?;
    Ok(unsafe { core::mem::transmute_copy::<usize, F>(&p) })
}
