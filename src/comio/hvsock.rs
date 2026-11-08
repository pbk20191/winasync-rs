//! Hyper-V socket (AF_HYPERV) transport: `HvStreamListener` (`AcceptEx`) and
//! `HvStreamSocket` (`ConnectEx` fetched via `WSAIoctl`), producing
//! `OverlappedDuplexStream`s (via [`stream::from_reg`]) registered as sockets
//! (`is_socket = true`), so `ReadAsync`/`WriteAsync` drive `WSARecv`/`WSASend`.
//!
//! This is a **pillar-3 gap filler**, not a WinRT duplicate: `Windows.Networking.Sockets`
//! has no Hyper-V socket type, and `HostName`/`HostNameType`
//! (`DomainName | Ipv4 | Ipv6 | Bluetooth`) cannot express a VM-GUID +
//! service-GUID endpoint at all, so `StreamSocket`/`StreamSocketListener`
//! could not reach this transport even in principle.
//!
//! Structurally this mirrors [`super::unix`] exactly — same `AcceptEx` /
//! `ConnectEx` shape, same `await_io_owning` discipline for the kernel-owned
//! address memory, same `arm_accept` callback form for `listener.rs`'s
//! executor-free accept chain. Only the address family and the sockaddr
//! differ; see [`crate::common::hvaddr`] for the addressing, shared with
//! `tp`'s Hyper-V transport.

use super::stream;
use super::tpio::{self, IoReg, Issue, OpHandle};
use super::waitop::await_io_owning;
use crate::common::hvaddr::{HvSocketAddr, SockaddrHv, HV_GUID_ZERO, HV_PROTOCOL_RAW};
use core::ffi::c_void;
use std::mem;
use std::sync::{Arc, Once, OnceLock};
use windows::Win32::Foundation::{HANDLE, WIN32_ERROR};
use windows::Win32::Networking::WinSock::{
    bind, closesocket, listen, setsockopt, AcceptEx, AF_HYPERV, INVALID_SOCKET,
    SIO_GET_EXTENSION_FUNCTION_POINTER, SOCKADDR, SOCKET, SOCK_STREAM, SOL_SOCKET, SOMAXCONN,
    SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT, WSADATA, WSAGetLastError, WSAID_CONNECTEX,
    WSAIoctl, WSASocketW, WSAStartup, WSA_FLAG_OVERLAPPED, WSA_IO_PENDING,
};
use windows::Win32::System::IO::OVERLAPPED;
use windows_core::{Error, IInspectable, Result, HRESULT};

/// Process-wide Winsock initialization; deliberately never torn down. Same
/// rationale as `super::unix::wsa_init` — `Once` is per-item, but `WSAStartup`
/// is refcounted by Winsock, so a second call from the other transport is
/// harmless.
fn wsa_init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        let mut data = WSADATA::default();
        let r = WSAStartup(0x0202, &mut data);
        assert_eq!(r, 0, "WSAStartup failed: {r}");
    });
}

fn wsa_last_hr() -> HRESULT {
    WIN32_ERROR(unsafe { WSAGetLastError() }.0 as u32).to_hresult()
}

fn wsa_last_err() -> Error {
    Error::from_hresult(wsa_last_hr())
}

fn sock_check(r: i32) -> Result<()> {
    if r == 0 {
        Ok(())
    } else {
        Err(wsa_last_err())
    }
}

/// Creates an AF_HYPERV stream socket. Fails with `WSAEAFNOSUPPORT` (10047)
/// where the Hyper-V socket provider is not present.
fn new_socket() -> Result<SOCKET> {
    wsa_init();
    unsafe {
        WSASocketW(
            AF_HYPERV as i32,
            SOCK_STREAM.0,
            HV_PROTOCOL_RAW,
            None,
            0,
            WSA_FLAG_OVERLAPPED,
        )
    }
}

fn classify_wsa_overlapped(succeeded: bool) -> Issue {
    if succeeded {
        // Synchronous success still queues a completion (we never set
        // FILE_SKIP_COMPLETION_PORT_ON_SUCCESS on these handles).
        Issue::Pending
    } else if unsafe { WSAGetLastError() } == WSA_IO_PENDING {
        Issue::Pending
    } else {
        Issue::Failed(wsa_last_hr())
    }
}

type ConnectExFn = unsafe extern "system" fn(
    s: SOCKET,
    name: *const SOCKADDR,
    namelen: i32,
    lpsendbuffer: *const c_void,
    dwsenddatalength: u32,
    lpdwbytessent: *mut u32,
    lpoverlapped: *mut OVERLAPPED,
) -> windows_core::BOOL;

/// `ConnectEx` is not a direct export: fetch its provider function pointer via
/// `WSAIoctl(SIO_GET_EXTENSION_FUNCTION_POINTER)` once and cache it.
///
/// Cached separately from `super::unix`'s: the pointer is a property of the
/// *provider*, and AF_HYPERV is served by a different one (hvsocket) than
/// AF_UNIX (afunix), so the two caches must not be shared.
fn get_connectex(sock: SOCKET) -> Result<ConnectExFn> {
    static PTR: OnceLock<usize> = OnceLock::new();
    if let Some(&p) = PTR.get() {
        return Ok(unsafe { mem::transmute::<usize, ConnectExFn>(p) });
    }
    let mut fptr: Option<ConnectExFn> = None;
    let mut bytes = 0u32;
    let r = unsafe {
        WSAIoctl(
            sock,
            SIO_GET_EXTENSION_FUNCTION_POINTER,
            Some(&WSAID_CONNECTEX as *const _ as *const c_void),
            size_of::<windows_core::GUID>() as u32,
            Some(&mut fptr as *mut _ as *mut c_void),
            size_of::<Option<ConnectExFn>>() as u32,
            &mut bytes,
            None,
            None,
        )
    };
    if r != 0 {
        return Err(wsa_last_err());
    }
    let f = fptr.expect("SIO_GET_EXTENSION_FUNCTION_POINTER returned null ConnectEx");
    let _ = PTR.set(f as usize);
    Ok(f)
}

/// AcceptEx address slot: sockaddr size + 16 bytes, per the API contract.
const ADDR_SLOT: usize = size_of::<SockaddrHv>() + 16;

/// Owns the pre-created accept socket while `AcceptEx` is in flight, closing it
/// if the `accept()` future is dropped before the op resolves. Same reasoning
/// as `super::unix::AcceptSockGuard`: closing a socket with a pending op just
/// aborts the op, with no kernel-owned memory left behind.
struct AcceptSockGuard(SOCKET);

impl Drop for AcceptSockGuard {
    fn drop(&mut self) {
        if self.0 != INVALID_SOCKET {
            unsafe { closesocket(self.0) };
        }
    }
}

// SOCKET is a plain kernel handle value.
unsafe impl Send for AcceptSockGuard {}

/// A listening Hyper-V socket, registered with the thread pool's TP_IO so
/// `AcceptEx` can be driven through [`await_io_owning`].
pub struct HvStreamListener {
    reg: Arc<IoReg>,
}

impl HvStreamListener {
    /// Binds a new AF_HYPERV socket to `addr` and starts listening.
    ///
    /// Bind the VM id to `HV_GUID_WILDCARD` to accept from any partition; the
    /// service id must be registered under `GuestCommunicationServices` for
    /// guests to reach it (host-to-host loopback does not need that).
    pub fn bind(addr: HvSocketAddr) -> Result<Self> {
        let sock = new_socket()?;
        let register = || -> Result<Arc<IoReg>> {
            let (sa, addrlen) = addr.to_raw();
            sock_check(unsafe { bind(sock, &sa as *const _ as *const SOCKADDR, addrlen) })?;
            sock_check(unsafe { listen(sock, SOMAXCONN as i32) })?;
            IoReg::register(HANDLE(sock.0 as *mut c_void), true)
        };
        match register() {
            Ok(reg) => Ok(HvStreamListener { reg }),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }

    /// Accepts one connection, returning it as a duplex WinRT stream.
    pub async fn accept(&self) -> Result<IInspectable> {
        let accept_sock = new_socket()?;
        let listen_sock = SOCKET(self.reg.handle.0 as usize);
        let mut guard = AcceptSockGuard(accept_sock);

        // The address buffer is kernel-owned memory for the op's duration (see
        // `super::unix::accept`'s long-form note): it must outlive not just
        // this `.await` but the op's real completion, hence `await_io_owning`
        // and the `usize`-smuggled pointer.
        let mut addr_buf: Box<[u8; 2 * ADDR_SLOT]> = Box::new([0u8; 2 * ADDR_SLOT]);
        let addr_ptr = addr_buf.as_mut_ptr() as usize;

        let result = await_io_owning(&self.reg, addr_buf, move |ovl| unsafe {
            let mut bytes = 0u32;
            let ok = AcceptEx(
                listen_sock,
                accept_sock,
                addr_ptr as *mut c_void,
                0, // complete on connect; do not wait for first data
                ADDR_SLOT as u32,
                ADDR_SLOT as u32,
                &mut bytes,
                ovl,
            );
            classify_wsa_overlapped(ok.as_bool())
        })
        .await;
        result?;

        let sock = mem::replace(&mut guard.0, INVALID_SOCKET);
        let finish = || -> Result<IInspectable> {
            let listen_val = listen_sock.0.to_ne_bytes();
            sock_check(unsafe {
                setsockopt(sock, SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, Some(&listen_val))
            })?;
            let reg = IoReg::register(HANDLE(sock.0 as *mut c_void), true)?;
            Ok(stream::from_reg(reg))
        };
        match finish() {
            Ok(s) => Ok(s),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }

    /// Callback-based counterpart to [`accept`](Self::accept), used by
    /// `listener.rs`'s self-re-arming, executor-free accept chain. Identical in
    /// structure to `super::unix::UnixStreamListener::arm_accept`, including
    /// the hand-rolled move of the address buffer and accept socket into the
    /// completion closure so both live until the real kernel completion.
    pub(crate) fn arm_accept(
        &self,
        on_result: Box<dyn FnOnce(Result<IInspectable>) + Send>,
    ) -> Result<OpHandle> {
        let accept_sock = new_socket()?;
        let listen_sock = SOCKET(self.reg.handle.0 as usize);

        let mut addr_buf: Box<[u8; 2 * ADDR_SLOT]> = Box::new([0u8; 2 * ADDR_SLOT]);
        let addr_ptr = addr_buf.as_mut_ptr() as usize;

        let op = unsafe {
            tpio::submit(
                &self.reg,
                Box::new(move |ioresult: u32, _bytes: usize| {
                    // Keep the address buffer alive until this real completion.
                    drop(addr_buf);
                    let res = if ioresult == 0 {
                        let listen_val = listen_sock.0.to_ne_bytes();
                        let finish = || -> Result<IInspectable> {
                            sock_check(setsockopt(
                                accept_sock,
                                SOL_SOCKET,
                                SO_UPDATE_ACCEPT_CONTEXT,
                                Some(&listen_val),
                            ))?;
                            let reg = IoReg::register(HANDLE(accept_sock.0 as *mut c_void), true)?;
                            Ok(stream::from_reg(reg))
                        };
                        match finish() {
                            Ok(s) => Ok(s),
                            Err(e) => {
                                closesocket(accept_sock);
                                Err(e)
                            }
                        }
                    } else {
                        closesocket(accept_sock);
                        Err(Error::from_hresult(HRESULT::from_win32(ioresult)))
                    };
                    on_result(res);
                }),
                move |ovl| {
                    let mut bytes = 0u32;
                    let ok = AcceptEx(
                        listen_sock,
                        accept_sock,
                        addr_ptr as *mut c_void,
                        0,
                        ADDR_SLOT as u32,
                        ADDR_SLOT as u32,
                        &mut bytes,
                        ovl,
                    );
                    classify_wsa_overlapped(ok.as_bool())
                },
            )
        };
        Ok(op)
    }
}

/// Client side of a Hyper-V socket connection.
pub struct HvStreamSocket;

impl HvStreamSocket {
    /// Connects to `addr`, returning the connection as a duplex WinRT stream.
    pub async fn connect(addr: HvSocketAddr) -> Result<IInspectable> {
        let sock = new_socket()?;
        let prepared = (|| -> Result<(ConnectExFn, Arc<IoReg>)> {
            // ConnectEx fails with WSAEINVAL on an unbound socket; the
            // AF_HYPERV equivalent of an autobind is the all-zero endpoint.
            let (local, local_len) = HvSocketAddr::new(HV_GUID_ZERO, HV_GUID_ZERO).to_raw();
            sock_check(unsafe { bind(sock, &local as *const _ as *const SOCKADDR, local_len) })?;
            let connect_ex = get_connectex(sock)?;
            let reg = IoReg::register(HANDLE(sock.0 as *mut c_void), true)?;
            Ok((connect_ex, reg))
        })();
        let (connect_ex, reg) = match prepared {
            Ok(v) => v,
            Err(e) => {
                unsafe { closesocket(sock) };
                return Err(e);
            }
        };

        let (remote, remote_len) = addr.to_raw();
        // `remote` is memory ConnectEx reads for the op's duration — boxed and
        // gated through `await_io_owning` exactly as in `super::unix::connect`,
        // so a dropped future (e.g. under a timeout) cannot free it while the
        // op is still live.
        let remote_box = Box::new(remote);
        let remote_ptr = remote_box.as_ref() as *const SockaddrHv as usize;
        let result = await_io_owning(&reg, remote_box, move |ovl| unsafe {
            let mut sent = 0u32;
            let ok = connect_ex(
                sock,
                remote_ptr as *const SOCKADDR,
                remote_len,
                core::ptr::null(),
                0,
                &mut sent,
                ovl,
            );
            classify_wsa_overlapped(ok.as_bool())
        })
        .await;
        result?;
        sock_check(unsafe { setsockopt(sock, SOL_SOCKET, SO_UPDATE_CONNECT_CONTEXT, None) })?;
        Ok(stream::from_reg(reg))
    }
}
