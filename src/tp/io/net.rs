//! Async sockets over TP_IO: `TcpListener` (AcceptEx), `TcpStream`
//! (ConnectEx / WSARecv / WSASend), and `UdpSocket` (WSARecvFrom / WSASendTo).
//! IPv4 and IPv6 are both supported through `std::net::SocketAddr`; the sockaddr
//! marshalling helpers ([`write_sockaddr`]/[`read_sockaddr`]) and socket
//! creation ([`new_socket`]) are shared across all three.
//!
//! # IPv6 caveats
//! An IPv6 socket is single-stack by default (Windows sets `IPV6_V6ONLY`), so an
//! IPv6 listener does not accept IPv4-mapped connections; bind a second IPv4
//! socket (or clear `IPV6_V6ONLY`) for dual-stack. `flowinfo`/`scope_id` are
//! passed through to/from the OS `sockaddr_in6` as-is (no byte-swap); only
//! non-zero flow labels crossing to a differing stack would be affected.

use super::file::File;
use super::{submit, IoRes, Issue, OpFuture};
use crate::tp::{Runtime, RuntimeInner};
use core::ffi::c_void;
use std::mem;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::{Arc, Once, OnceLock};
use windows::Win32::Foundation::{HANDLE, WIN32_ERROR};
use windows::Win32::Networking::WinSock::{
    bind, closesocket, connect, getpeername, getsockname, listen, setsockopt, shutdown, AcceptEx,
    WSAGetLastError, WSAIoctl, WSARecv, WSARecvFrom, WSASend, WSASendTo, WSASocketW, WSAStartup,
    ADDRESS_FAMILY, AF_INET, AF_INET6, IN6_ADDR, IN6_ADDR_0, IN_ADDR, IN_ADDR_0, INVALID_SOCKET,
    IPPROTO, IPPROTO_TCP, IPPROTO_UDP, SD_SEND, SIO_GET_EXTENSION_FUNCTION_POINTER, SOCKADDR,
    SOCKADDR_IN, SOCKADDR_IN6, SOCKADDR_IN6_0, SOCKADDR_STORAGE, SOCKET, SOCK_DGRAM, SOCK_STREAM,
    SOL_SOCKET, SOMAXCONN, SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT, TF_DISCONNECT,
    TF_REUSE_SOCKET, TP_ELEMENT_FILE, TP_ELEMENT_MEMORY, TRANSMIT_PACKETS_ELEMENT,
    TRANSMIT_PACKETS_ELEMENT_0, TRANSMIT_PACKETS_ELEMENT_0_0, TransmitFile, WINSOCK_SOCKET_TYPE,
    WSABUF, WSADATA, WSAID_CONNECTEX, WSAID_DISCONNECTEX, WSAID_TRANSMITPACKETS,
    WSA_FLAG_OVERLAPPED, WSA_IO_PENDING,
};

/// Process-wide Winsock initialization; never torn down (`WSACleanup` is
/// deliberately not called).
pub fn wsa_init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        let mut data = WSADATA::default();
        let r = WSAStartup(0x0202, &mut data);
        assert_eq!(r, 0, "WSAStartup failed: {r}");
    });
}

fn wsa_last_hr() -> windows_core::HRESULT {
    WIN32_ERROR(unsafe { WSAGetLastError() }.0 as u32).to_hresult()
}

fn wsa_last_err() -> windows_core::Error {
    windows_core::Error::from_hresult(wsa_last_hr())
}

pub(super) fn sock_check(r: i32) -> windows_core::Result<()> {
    if r == 0 {
        Ok(())
    } else {
        Err(wsa_last_err())
    }
}

/// Creates an overlapped socket of the given family/type/protocol. `WSASocketW`
/// is wrapped by the `windows` crate to map `INVALID_SOCKET` to an `Err`.
pub(super) fn new_socket(
    family: ADDRESS_FAMILY,
    socktype: WINSOCK_SOCKET_TYPE,
    proto: IPPROTO,
) -> windows_core::Result<SOCKET> {
    wsa_init();
    unsafe {
        WSASocketW(
            family.0 as i32,
            socktype.0,
            proto.0,
            None,
            0,
            WSA_FLAG_OVERLAPPED,
        )
    }
}

/// The address family a `SocketAddr` needs its socket created with.
fn family_of(addr: &SocketAddr) -> ADDRESS_FAMILY {
    match addr {
        SocketAddr::V4(_) => AF_INET,
        SocketAddr::V6(_) => AF_INET6,
    }
}

/// The `0.0.0.0:0` / `[::]:0` wildcard of a family, for the implicit local bind
/// `ConnectEx` requires.
fn wildcard(family: ADDRESS_FAMILY) -> SocketAddr {
    if family == AF_INET6 {
        SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0))
    } else {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
    }
}

/// Marshals a `SocketAddr` into a zeroed `SOCKADDR_STORAGE`, returning the
/// valid length (`sizeof(sockaddr_in|sockaddr_in6)`). Port is written in
/// network byte order; the IPv4 address follows the existing `to_be` idiom so
/// its in-memory bytes are the octets in order.
fn write_sockaddr(addr: &SocketAddr, storage: &mut SOCKADDR_STORAGE) -> i32 {
    unsafe {
        match addr {
            SocketAddr::V4(v4) => {
                *(storage as *mut _ as *mut SOCKADDR_IN) = SOCKADDR_IN {
                    sin_family: AF_INET,
                    sin_port: v4.port().to_be(),
                    sin_addr: IN_ADDR {
                        S_un: IN_ADDR_0 {
                            S_addr: u32::from(*v4.ip()).to_be(),
                        },
                    },
                    sin_zero: [0; 8],
                };
                size_of::<SOCKADDR_IN>() as i32
            }
            SocketAddr::V6(v6) => {
                *(storage as *mut _ as *mut SOCKADDR_IN6) = SOCKADDR_IN6 {
                    sin6_family: AF_INET6,
                    sin6_port: v6.port().to_be(),
                    sin6_flowinfo: v6.flowinfo(),
                    sin6_addr: IN6_ADDR {
                        u: IN6_ADDR_0 {
                            Byte: v6.ip().octets(),
                        },
                    },
                    Anonymous: SOCKADDR_IN6_0 {
                        sin6_scope_id: v6.scope_id(),
                    },
                };
                size_of::<SOCKADDR_IN6>() as i32
            }
        }
    }
}

/// Parses a `SOCKADDR_STORAGE` (as filled by `getsockname`/`getpeername`/a
/// datagram source) back into a `SocketAddr`, dispatching on `ss_family`.
fn read_sockaddr(storage: &SOCKADDR_STORAGE) -> windows_core::Result<SocketAddr> {
    unsafe {
        if storage.ss_family == AF_INET {
            let sa = &*(storage as *const _ as *const SOCKADDR_IN);
            Ok(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::from(u32::from_be(sa.sin_addr.S_un.S_addr)),
                u16::from_be(sa.sin_port),
            )))
        } else if storage.ss_family == AF_INET6 {
            let sa = &*(storage as *const _ as *const SOCKADDR_IN6);
            Ok(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(sa.sin6_addr.u.Byte),
                u16::from_be(sa.sin6_port),
                sa.sin6_flowinfo,
                sa.Anonymous.sin6_scope_id,
            )))
        } else {
            // Neither AF_INET nor AF_INET6 — WSAEAFNOSUPPORT.
            Err(windows_core::Error::from_hresult(
                WIN32_ERROR(10047).to_hresult(),
            ))
        }
    }
}

/// `getsockname` for a socket, as a `SocketAddr` (useful after binding port 0).
fn addr_of_socket(sock: SOCKET) -> windows_core::Result<SocketAddr> {
    let mut storage = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
    let mut len = size_of::<SOCKADDR_STORAGE>() as i32;
    sock_check(unsafe { getsockname(sock, &mut storage as *mut _ as *mut SOCKADDR, &mut len) })?;
    read_sockaddr(&storage)
}

/// `getpeername` for a connected socket, as a `SocketAddr`.
fn peer_of_socket(sock: SOCKET) -> windows_core::Result<SocketAddr> {
    let mut storage = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
    let mut len = size_of::<SOCKADDR_STORAGE>() as i32;
    sock_check(unsafe { getpeername(sock, &mut storage as *mut _ as *mut SOCKADDR, &mut len) })?;
    read_sockaddr(&storage)
}

type ConnectExFn = unsafe extern "system" fn(
    s: SOCKET,
    name: *const SOCKADDR,
    namelen: i32,
    lpsendbuffer: *const c_void,
    dwsenddatalength: u32,
    lpdwbytessent: *mut u32,
    lpoverlapped: *mut windows::Win32::System::IO::OVERLAPPED,
) -> windows_core::BOOL;

/// `ConnectEx` is not a direct export: fetch its provider function pointer via
/// `WSAIoctl(SIO_GET_EXTENSION_FUNCTION_POINTER)` once and cache it (the base
/// TCP provider's pointer is process-stable).
pub(super) fn get_connectex(sock: SOCKET) -> windows_core::Result<ConnectExFn> {
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

type TransmitPacketsFn = unsafe extern "system" fn(
    s: SOCKET,
    lppacketarray: *mut TRANSMIT_PACKETS_ELEMENT,
    nelementcount: u32,
    nsendsize: u32,
    lpoverlapped: *mut windows::Win32::System::IO::OVERLAPPED,
    dwflags: u32,
) -> windows_core::BOOL;

/// `TransmitPackets` is a provider extension (no direct export): fetch its
/// function pointer via `WSAIoctl(SIO_GET_EXTENSION_FUNCTION_POINTER)` once and
/// cache it, like [`get_connectex`].
fn get_transmitpackets(sock: SOCKET) -> windows_core::Result<TransmitPacketsFn> {
    static PTR: OnceLock<usize> = OnceLock::new();
    if let Some(&p) = PTR.get() {
        return Ok(unsafe { mem::transmute::<usize, TransmitPacketsFn>(p) });
    }
    let mut fptr: Option<TransmitPacketsFn> = None;
    let mut bytes = 0u32;
    let r = unsafe {
        WSAIoctl(
            sock,
            SIO_GET_EXTENSION_FUNCTION_POINTER,
            Some(&WSAID_TRANSMITPACKETS as *const _ as *const c_void),
            size_of::<windows_core::GUID>() as u32,
            Some(&mut fptr as *mut _ as *mut c_void),
            size_of::<Option<TransmitPacketsFn>>() as u32,
            &mut bytes,
            None,
            None,
        )
    };
    if r != 0 {
        return Err(wsa_last_err());
    }
    let f = fptr.expect("SIO_GET_EXTENSION_FUNCTION_POINTER returned null TransmitPackets");
    let _ = PTR.set(f as usize);
    Ok(f)
}

/// Post-transfer disposition for [`TcpStream::transmit_file`] /
/// [`TcpStream::transmit_packets`]: optionally disconnect after sending, and
/// prepare the handle for reuse (see [`TcpStream::reconnect`] /
/// [`TcpStream::into_reusable`]).
#[derive(Clone, Copy, Default)]
pub struct TransmitFlags {
    /// Issue a transport disconnect after the transfer (`TF_DISCONNECT`).
    pub disconnect: bool,
    /// Prepare the handle to be reused by a later `AcceptEx`/`ConnectEx`
    /// (`TF_REUSE_SOCKET`). Implies `disconnect`.
    pub reuse_socket: bool,
}

impl TransmitFlags {
    fn to_bits(self) -> u32 {
        let mut f = 0;
        if self.disconnect || self.reuse_socket {
            f |= TF_DISCONNECT;
        }
        if self.reuse_socket {
            f |= TF_REUSE_SOCKET;
        }
        f
    }
}

type DisconnectExFn = unsafe extern "system" fn(
    s: SOCKET,
    lpoverlapped: *mut windows::Win32::System::IO::OVERLAPPED,
    dwflags: u32,
    dwreserved: u32,
) -> windows_core::BOOL;

/// `DisconnectEx` is a provider extension (no direct export): fetch and cache
/// its function pointer via `WSAIoctl`, like [`get_connectex`].
fn get_disconnectex(sock: SOCKET) -> windows_core::Result<DisconnectExFn> {
    static PTR: OnceLock<usize> = OnceLock::new();
    if let Some(&p) = PTR.get() {
        return Ok(unsafe { mem::transmute::<usize, DisconnectExFn>(p) });
    }
    let mut fptr: Option<DisconnectExFn> = None;
    let mut bytes = 0u32;
    let r = unsafe {
        WSAIoctl(
            sock,
            SIO_GET_EXTENSION_FUNCTION_POINTER,
            Some(&WSAID_DISCONNECTEX as *const _ as *const c_void),
            size_of::<windows_core::GUID>() as u32,
            Some(&mut fptr as *mut _ as *mut c_void),
            size_of::<Option<DisconnectExFn>>() as u32,
            &mut bytes,
            None,
            None,
        )
    };
    if r != 0 {
        return Err(wsa_last_err());
    }
    let f = fptr.expect("SIO_GET_EXTENSION_FUNCTION_POINTER returned null DisconnectEx");
    let _ = PTR.set(f as usize);
    Ok(f)
}

pub(super) fn classify_wsa_overlapped(succeeded: bool) -> Issue {
    if succeeded {
        // Synchronous success still queues a completion (we never set
        // FILE_SKIP_COMPLETION_PORT_ON_SUCCESS).
        Issue::Pending
    } else if unsafe { WSAGetLastError() } == WSA_IO_PENDING {
        Issue::Pending
    } else {
        Issue::Failed(wsa_last_hr())
    }
}

/// AcceptEx address slot: sockaddr size + 16 bytes, per the API contract.
pub(super) const ADDR_SLOT: usize = size_of::<SOCKADDR_STORAGE>() + 16;

/// Owns the pre-created accept socket and address buffer while AcceptEx is in
/// flight. If the op is cancelled the completion callback frees the block and
/// this Drop closes the socket.
struct AcceptPayload {
    accept_sock: SOCKET,
    addr_buf: Box<[u8; 2 * ADDR_SLOT]>,
}

impl Drop for AcceptPayload {
    fn drop(&mut self) {
        if self.accept_sock != INVALID_SOCKET {
            unsafe { closesocket(self.accept_sock) };
        }
    }
}

// SOCKET is a plain kernel handle value.
unsafe impl Send for AcceptPayload {}

/// Payload for [`TcpListener::accept_into`]: the recycled socket's `IoRes` (kept
/// alive for the op — its socket is the AcceptEx target, and its TP_IO is reused
/// by the returned stream) plus the AcceptEx address buffer. `Arc<IoRes>` is
/// `Send`, so this needs no manual `unsafe impl` (unlike the raw-`SOCKET`
/// [`AcceptPayload`]).
struct AcceptIntoPayload {
    res: Arc<IoRes>,
    addr_buf: Box<[u8; 2 * ADDR_SLOT]>,
}

/// A listening TCP socket bound to a [`Runtime`]'s pool.
pub struct TcpListener {
    res: Arc<IoRes>,
    /// The address family the listen socket was created with; accepted sockets
    /// must be created with the same family for `AcceptEx`.
    family: ADDRESS_FAMILY,
}

impl TcpListener {
    /// Binds and listens on `addr` (IPv4 or IPv6).
    pub fn bind(rt: &Runtime, addr: SocketAddr) -> windows_core::Result<TcpListener> {
        let family = family_of(&addr);
        let sock = new_socket(family, SOCK_STREAM, IPPROTO_TCP)?;
        let register = || -> windows_core::Result<Arc<IoRes>> {
            let mut storage = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
            let len = write_sockaddr(&addr, &mut storage);
            sock_check(unsafe { bind(sock, &storage as *const _ as *const SOCKADDR, len) })?;
            sock_check(unsafe { listen(sock, SOMAXCONN as i32) })?;
            IoRes::register(&rt.inner, HANDLE(sock.0 as *mut c_void), true)
        };
        match register() {
            Ok(res) => Ok(TcpListener { res, family }),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }

    /// The locally bound address (useful after binding port 0).
    pub fn local_addr(&self) -> windows_core::Result<SocketAddr> {
        addr_of_socket(self.res.socket())
    }

    /// Accepts one connection.
    pub async fn accept(&self) -> windows_core::Result<TcpStream> {
        let accept_sock = new_socket(self.family, SOCK_STREAM, IPPROTO_TCP)?;
        let listen_sock = self.res.socket();
        let fut = unsafe {
            submit(
                &self.res,
                AcceptPayload {
                    accept_sock,
                    addr_buf: Box::new([0; 2 * ADDR_SLOT]),
                },
                |ovl, p| {
                    let mut bytes = 0u32;
                    let ok = AcceptEx(
                        listen_sock,
                        p.accept_sock,
                        p.addr_buf.as_mut_ptr() as *mut c_void,
                        0, // complete on connect; do not wait for first data
                        ADDR_SLOT as u32,
                        ADDR_SLOT as u32,
                        &mut bytes,
                        ovl,
                    );
                    classify_wsa_overlapped(ok.as_bool())
                },
            )
        };
        let (result, mut payload) = fut.await;
        result?;
        let sock = mem::replace(&mut payload.accept_sock, INVALID_SOCKET);
        let finish = || -> windows_core::Result<TcpStream> {
            // Until SO_UPDATE_ACCEPT_CONTEXT (value = the listen socket) the
            // accepted socket is half-configured (shutdown/getsockname/
            // getpeername fail).
            let listen_val = listen_sock.0.to_ne_bytes();
            sock_check(unsafe {
                setsockopt(sock, SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, Some(&listen_val))
            })?;
            TcpStream::from_socket(&self.res.rt, sock)
        };
        match finish() {
            Ok(stream) => Ok(stream),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }

    /// Accepts the next connection into a socket recycled from
    /// [`TcpStream::into_reusable`], skipping the per-connection `socket()` and —
    /// crucially — the completion-port re-association, which Windows forbids (a
    /// socket is bound to one IOCP for life). The reused socket keeps its original
    /// `IoRes`/TP_IO; only `AcceptEx` is re-run on it, driven by this listener,
    /// and the returned stream reuses that same `IoRes`. The reused socket must
    /// match this listener's address family.
    pub async fn accept_into(&self, reusable: ReusableSocket) -> windows_core::Result<TcpStream> {
        let accept_sock = reusable.res.socket();
        let listen_sock = self.res.socket();
        let fut = unsafe {
            submit(
                &self.res,
                AcceptIntoPayload {
                    // Keep the recycled socket's IoRes alive for the op (its
                    // socket is the AcceptEx target); the IoRes owns the socket,
                    // so the payload never closes it.
                    res: reusable.res,
                    addr_buf: Box::new([0; 2 * ADDR_SLOT]),
                },
                |ovl, p| {
                    let mut bytes = 0u32;
                    let ok = AcceptEx(
                        listen_sock,
                        p.res.socket(),
                        p.addr_buf.as_mut_ptr() as *mut c_void,
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
        let (result, payload) = fut.await;
        result?;
        let listen_val = listen_sock.0.to_ne_bytes();
        sock_check(unsafe {
            setsockopt(accept_sock, SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, Some(&listen_val))
        })?;
        // Reuse the recycled socket's existing TP_IO registration directly — no
        // re-register (one IOCP per socket for life).
        Ok(TcpStream { res: payload.res })
    }
}

/// Payload of a WSARecv: the buffer plus the flags word, which the provider
/// may write at completion time and therefore must live in the op block.
struct RecvPayload {
    buf: Vec<u8>,
    flags: u32,
}

/// A connected TCP socket bound to a [`Runtime`]'s pool.
pub struct TcpStream {
    res: Arc<IoRes>,
}

impl TcpStream {
    fn from_socket(rt: &Arc<RuntimeInner>, sock: SOCKET) -> windows_core::Result<TcpStream> {
        match IoRes::register(rt, HANDLE(sock.0 as *mut c_void), true) {
            Ok(res) => Ok(TcpStream { res }),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }

    /// Connects to `addr` (IPv4 or IPv6).
    pub async fn connect(rt: &Runtime, addr: SocketAddr) -> windows_core::Result<TcpStream> {
        let family = family_of(&addr);
        let sock = new_socket(family, SOCK_STREAM, IPPROTO_TCP)?;
        let prepared = (|| -> windows_core::Result<(ConnectExFn, TcpStream)> {
            // ConnectEx fails with WSAEINVAL on an unbound socket.
            let mut local = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
            let local_len = write_sockaddr(&wildcard(family), &mut local);
            sock_check(unsafe {
                bind(sock, &local as *const _ as *const SOCKADDR, local_len)
            })?;
            let connect_ex = get_connectex(sock)?;
            Ok((connect_ex, TcpStream::from_socket(&rt.inner, sock)?))
        })();
        let (connect_ex, stream) = match prepared {
            Ok(v) => v,
            Err(e) => {
                unsafe { closesocket(sock) };
                return Err(e);
            }
        };

        let mut remote = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
        let remote_len = write_sockaddr(&addr, &mut remote);
        let fut = unsafe {
            submit(&stream.res, (remote, remote_len), move |ovl, p| {
                let mut sent = 0u32;
                let ok = connect_ex(
                    sock,
                    &p.0 as *const _ as *const SOCKADDR,
                    p.1,
                    core::ptr::null(),
                    0,
                    &mut sent,
                    ovl,
                );
                classify_wsa_overlapped(ok.as_bool())
            })
        };
        let (result, _) = fut.await;
        result?;
        sock_check(unsafe { setsockopt(sock, SOL_SOCKET, SO_UPDATE_CONNECT_CONTEXT, None) })?;
        Ok(stream)
    }

    /// Receives into `buf`; resolves to the byte count (0 = peer closed) and
    /// hands the buffer back.
    pub fn read(
        &self,
        buf: Vec<u8>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<u8>)> + Send + use<> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, RecvPayload { buf, flags: 0 }, move |ovl, p| {
                let wsabuf = WSABUF {
                    len: p.buf.len() as u32,
                    buf: windows_core::PSTR(p.buf.as_mut_ptr()),
                };
                let r = WSARecv(sock, &[wsabuf], None, &mut p.flags, Some(ovl), None);
                classify_wsa_overlapped(r == 0)
            })
        };
        async move {
            let (result, payload) = fut.await;
            (result, payload.buf)
        }
    }

    /// Sends `buf`; resolves to the byte count and hands the buffer back.
    pub fn write(
        &self,
        buf: Vec<u8>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<u8>)> + Send + use<> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                let wsabuf = WSABUF {
                    len: buf.len() as u32,
                    buf: windows_core::PSTR(buf.as_mut_ptr()),
                };
                let r = WSASend(sock, &[wsabuf], None, 0, Some(ovl), None);
                classify_wsa_overlapped(r == 0)
            })
        };
        async move {
            let (result, buf) = fut.await;
            (result, buf)
        }
    }

    /// Half-closes the send direction; the peer's next read returns 0.
    pub fn shutdown_send(&self) -> windows_core::Result<()> {
        sock_check(unsafe { shutdown(self.res.socket(), SD_SEND) })
    }

    /// A second handle onto the same socket (shared via the refcounted
    /// `IoRes`); the socket closes only once all handles drop. Used by
    /// `TcpStreamIo::split` to drive the read and write halves independently —
    /// concurrent WSARecv (read) and WSASend (write) on one socket is supported.
    pub(super) fn share(&self) -> TcpStream {
        TcpStream {
            res: self.res.clone(),
        }
    }

    /// The locally bound address of this connected socket.
    pub fn local_addr(&self) -> windows_core::Result<SocketAddr> {
        addr_of_socket(self.res.socket())
    }

    /// The remote peer's address.
    pub fn peer_addr(&self) -> windows_core::Result<SocketAddr> {
        peer_of_socket(self.res.socket())
    }

    /// Sends (part of) an open overlapped `file`'s contents over the socket
    /// using the kernel's `TransmitFile` — a zero-copy path straight from the
    /// file-system cache, without staging the bytes through user space.
    ///
    /// `offset` is the starting position in the file; `count == 0` sends from
    /// `offset` to end of file. Resolves to the number of bytes transmitted.
    /// The op completes on this socket's TP_IO like any other overlapped send.
    pub fn transmit_file(
        &self,
        file: &File,
        offset: u64,
        count: u32,
        flags: TransmitFlags,
    ) -> impl Future<Output = windows_core::Result<usize>> + Send + use<> {
        let sock = self.res.socket();
        let file_handle = file.handle();
        let tf = flags.to_bits();
        // Keep the file's IoRes (hence its HANDLE) alive for the op's lifetime.
        let keepalive = file.res_arc();
        let fut = unsafe {
            submit(&self.res, keepalive, move |ovl, _keepalive| {
                (*ovl).Anonymous.Anonymous.Offset = offset as u32;
                (*ovl).Anonymous.Anonymous.OffsetHigh = (offset >> 32) as u32;
                let ok = TransmitFile(sock, file_handle, count, 0, Some(ovl), None, tf);
                classify_wsa_overlapped(ok.as_bool())
            })
        };
        async move {
            let (result, _keepalive) = fut.await;
            result
        }
    }

    /// Sends a sequence of in-memory buffers and/or file ranges as one
    /// `TransmitPackets` call — a single-syscall scatter of mixed memory and
    /// file sources, transmitted in insertion order. Resolves to the total
    /// number of bytes sent. See [`TransmitPackets`] for building the sequence.
    pub fn transmit_packets(
        &self,
        packets: TransmitPackets,
        flags: TransmitFlags,
    ) -> impl Future<Output = windows_core::Result<usize>> + Send + use<> {
        let sock = self.res.socket();
        let tf = flags.to_bits();
        // Fetch the extension pointer and issue the op *synchronously*, so the
        // returned async block captures only the (Send) OpFuture — never the
        // non-Send HANDLEs, which are consumed here into the op block.
        let started: windows_core::Result<OpFuture<TransmitPacketsPayload>> = (|| {
            let transmit = get_transmitpackets(sock)?;
            let TransmitPackets { elems } = packets;
            let count = elems.len();
            let payload = TransmitPacketsPayload {
                elems,
                array: Vec::with_capacity(count),
            };
            Ok(unsafe {
                submit(&self.res, payload, move |ovl, p| {
                    // Build the element array in place (it and the memory
                    // buffers it points at all live in the op block for the
                    // op's duration). Pre-reserved, so no realloc moves it.
                    p.array.clear();
                    for e in p.elems.iter_mut() {
                        let el = match e {
                            Elem::Mem(buf) => TRANSMIT_PACKETS_ELEMENT {
                                dwElFlags: TP_ELEMENT_MEMORY,
                                cLength: buf.len() as u32,
                                Anonymous: TRANSMIT_PACKETS_ELEMENT_0 {
                                    pBuffer: buf.as_mut_ptr() as *mut c_void,
                                },
                            },
                            Elem::File {
                                handle,
                                offset,
                                length,
                                ..
                            } => TRANSMIT_PACKETS_ELEMENT {
                                dwElFlags: TP_ELEMENT_FILE,
                                cLength: *length,
                                Anonymous: TRANSMIT_PACKETS_ELEMENT_0 {
                                    Anonymous: TRANSMIT_PACKETS_ELEMENT_0_0 {
                                        nFileOffset: *offset,
                                        hFile: *handle,
                                    },
                                },
                            },
                        };
                        p.array.push(el);
                    }
                    let ok = transmit(
                        sock,
                        p.array.as_mut_ptr(),
                        p.array.len() as u32,
                        0,
                        ovl,
                        tf,
                    );
                    classify_wsa_overlapped(ok.as_bool())
                })
            })
        })();
        async move {
            match started {
                Ok(fut) => fut.await.0,
                Err(e) => Err(e),
            }
        }
    }

    /// Transport-level disconnect via `DisconnectEx`. With `reuse = true`
    /// (`TF_REUSE_SOCKET`) the handle is left ready for reuse without a fresh
    /// `socket()` — by [`reconnect`](Self::reconnect) (client) or
    /// [`into_reusable`](Self::into_reusable) + [`TcpListener::accept_into`]
    /// (server).
    ///
    /// This is a **graceful** disconnect: the returned future resolves once the
    /// close handshake completes — i.e. after the peer has also closed its side.
    /// Use it on a *finished* connection (both ends done); if the peer keeps its
    /// side open it will not complete promptly.
    pub fn disconnect(
        &self,
        reuse: bool,
    ) -> impl Future<Output = windows_core::Result<()>> + Send + use<> {
        let sock = self.res.socket();
        let started: windows_core::Result<OpFuture<()>> =
            get_disconnectex(sock).map(|disconnect_ex| {
                let flags = if reuse { TF_REUSE_SOCKET } else { 0 };
                unsafe {
                    submit(&self.res, (), move |ovl, _| {
                        let ok = disconnect_ex(sock, ovl, flags, 0);
                        classify_wsa_overlapped(ok.as_bool())
                    })
                }
            });
        async move {
            match started {
                Ok(fut) => fut.await.0.map(|_| ()),
                Err(e) => Err(e),
            }
        }
    }

    /// Reuses this (disconnected, `TF_REUSE_SOCKET`-prepared) socket for a new
    /// outbound connection to `addr` — a `ConnectEx` on the *same handle*, with
    /// no fresh `socket()`/`bind()`/pool registration. Call after
    /// `disconnect(true).await`.
    pub async fn reconnect(&self, addr: SocketAddr) -> windows_core::Result<()> {
        let sock = self.res.socket();
        let connect_ex = get_connectex(sock)?;
        let mut remote = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
        let remote_len = write_sockaddr(&addr, &mut remote);
        let fut = unsafe {
            submit(&self.res, (remote, remote_len), move |ovl, p| {
                let mut sent = 0u32;
                let ok = connect_ex(
                    sock,
                    &p.0 as *const _ as *const SOCKADDR,
                    p.1,
                    core::ptr::null(),
                    0,
                    &mut sent,
                    ovl,
                );
                classify_wsa_overlapped(ok.as_bool())
            })
        };
        fut.await.0?;
        sock_check(unsafe { setsockopt(sock, SOL_SOCKET, SO_UPDATE_CONNECT_CONTEXT, None) })?;
        Ok(())
    }

    /// Consumes this stream, handing its socket (with its existing `IoRes`/TP_IO
    /// retained) to [`TcpListener::accept_into`] for reuse. Call after
    /// `disconnect(true).await` on a finished connection. Keeping the `IoRes`
    /// avoids a completion-port re-association, which Windows forbids.
    pub fn into_reusable(self) -> ReusableSocket {
        ReusableSocket { res: self.res }
    }
}

/// A finished TCP socket (disconnected with reuse via `DisconnectEx` +
/// `TF_REUSE_SOCKET`) retaining its `IoRes`/TP_IO, ready to be recycled by
/// [`TcpListener::accept_into`] without a new `socket()` or completion-port
/// re-association. Dropped without reuse, it closes the socket normally (the
/// `IoRes`'s own `Drop`).
pub struct ReusableSocket {
    res: Arc<IoRes>,
}

/// One element of a [`TransmitPackets`] sequence: an owned memory buffer, or a
/// range of an open file (kept alive via its `IoRes`).
enum Elem {
    Mem(Vec<u8>),
    File {
        /// Keeps the file handle valid for the op's lifetime.
        _keep: Arc<IoRes>,
        handle: HANDLE,
        offset: i64,
        length: u32,
    },
}

/// A builder for [`TcpStream::transmit_packets`]: an ordered mix of in-memory
/// buffers and open-file ranges, sent in one `TransmitPackets` syscall.
#[derive(Default)]
pub struct TransmitPackets {
    elems: Vec<Elem>,
}

impl TransmitPackets {
    /// A new, empty sequence.
    pub fn new() -> Self {
        Self { elems: Vec::new() }
    }

    /// Appends an in-memory buffer (sent inline).
    pub fn bytes(mut self, buf: Vec<u8>) -> Self {
        self.elems.push(Elem::Mem(buf));
        self
    }

    /// Appends a byte range of an open overlapped `file` (`length == 0` sends to
    /// end of file). The file is kept alive until the transmit completes.
    pub fn file(mut self, file: &File, offset: i64, length: u32) -> Self {
        self.elems.push(Elem::File {
            _keep: file.res_arc(),
            handle: file.handle(),
            offset,
            length,
        });
        self
    }
}

/// Op-block payload for `TransmitPackets`: owns every memory buffer and file
/// keepalive, plus the element array the kernel reads.
struct TransmitPacketsPayload {
    elems: Vec<Elem>,
    array: Vec<TRANSMIT_PACKETS_ELEMENT>,
}

// All raw handles/pointers held here are plain kernel values / stable heap
// pointers into this same payload; moving the payload is sound.
unsafe impl Send for TransmitPacketsPayload {}

/// Payload of a WSARecvFrom: the buffer, the flags word, and the source
/// address slot — all written by the provider at completion time, so all must
/// live in the op block.
struct RecvFromPayload {
    buf: Vec<u8>,
    flags: u32,
    from: SOCKADDR_STORAGE,
    from_len: i32,
}

// SOCKADDR_STORAGE is plain POD; the whole payload is safely sendable.
unsafe impl Send for RecvFromPayload {}

/// Payload of a WSASendTo: the buffer plus the destination address, which the
/// provider reads while the send is in flight.
struct SendToPayload {
    buf: Vec<u8>,
    to: SOCKADDR_STORAGE,
    to_len: i32,
}

unsafe impl Send for SendToPayload {}

/// A bound UDP socket on a [`Runtime`]'s pool. Supports connectionless
/// `send_to`/`recv_from` over IPv4 or IPv6.
pub struct UdpSocket {
    res: Arc<IoRes>,
}

impl UdpSocket {
    /// Binds a UDP socket to `addr` (IPv4 or IPv6).
    pub fn bind(rt: &Runtime, addr: SocketAddr) -> windows_core::Result<UdpSocket> {
        let family = family_of(&addr);
        let sock = new_socket(family, SOCK_DGRAM, IPPROTO_UDP)?;
        let register = || -> windows_core::Result<Arc<IoRes>> {
            let mut storage = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
            let len = write_sockaddr(&addr, &mut storage);
            sock_check(unsafe { bind(sock, &storage as *const _ as *const SOCKADDR, len) })?;
            IoRes::register(&rt.inner, HANDLE(sock.0 as *mut c_void), true)
        };
        match register() {
            Ok(res) => Ok(UdpSocket { res }),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }

    /// The locally bound address (useful after binding port 0).
    pub fn local_addr(&self) -> windows_core::Result<SocketAddr> {
        addr_of_socket(self.res.socket())
    }

    /// Sends `buf` as one datagram to `addr`; resolves to the byte count and
    /// hands the buffer back.
    pub fn send_to(
        &self,
        buf: Vec<u8>,
        addr: SocketAddr,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<u8>)> + Send {
        let sock = self.res.socket();
        let mut to = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
        let to_len = write_sockaddr(&addr, &mut to);
        let fut = unsafe {
            submit(&self.res, SendToPayload { buf, to, to_len }, move |ovl, p| {
                let wsabuf = WSABUF {
                    len: p.buf.len() as u32,
                    buf: windows_core::PSTR(p.buf.as_mut_ptr()),
                };
                let r = WSASendTo(
                    sock,
                    &[wsabuf],
                    None,
                    0,
                    Some(&p.to as *const _ as *const SOCKADDR),
                    p.to_len,
                    Some(ovl),
                    None,
                );
                classify_wsa_overlapped(r == 0)
            })
        };
        async move {
            let (result, payload) = fut.await;
            (result, payload.buf)
        }
    }

    /// Receives one datagram into `buf`; resolves to the byte count plus the
    /// source address, and hands the buffer back.
    ///
    /// A datagram larger than `buf` is truncated to `buf.len()` and the op fails
    /// with `WSAEMSGSIZE` (the Windows semantics for an oversized datagram); the
    /// partial bytes are discarded. Size `buf` to the largest datagram expected.
    pub fn recv_from(
        &self,
        buf: Vec<u8>,
    ) -> impl Future<Output = (windows_core::Result<(usize, SocketAddr)>, Vec<u8>)> + Send {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(
                &self.res,
                RecvFromPayload {
                    buf,
                    flags: 0,
                    from: mem::zeroed::<SOCKADDR_STORAGE>(),
                    from_len: size_of::<SOCKADDR_STORAGE>() as i32,
                },
                move |ovl, p| {
                    let wsabuf = WSABUF {
                        len: p.buf.len() as u32,
                        buf: windows_core::PSTR(p.buf.as_mut_ptr()),
                    };
                    let r = WSARecvFrom(
                        sock,
                        &[wsabuf],
                        None,
                        &mut p.flags,
                        Some(&mut p.from as *mut _ as *mut SOCKADDR),
                        Some(&mut p.from_len),
                        Some(ovl),
                        None,
                    );
                    classify_wsa_overlapped(r == 0)
                },
            )
        };
        async move {
            let (result, payload) = fut.await;
            let out = match result {
                Ok(n) => read_sockaddr(&payload.from).map(|addr| (n, addr)),
                Err(e) => Err(e),
            };
            (out, payload.buf)
        }
    }

    /// Sets the default peer: afterwards [`send`](Self::send)/[`recv`](Self::recv)
    /// talk only to `addr`, and datagrams from other sources are dropped. UDP
    /// `connect` is a local operation (no handshake), so this is synchronous.
    pub fn connect(&self, addr: SocketAddr) -> windows_core::Result<()> {
        let mut storage = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
        let len = write_sockaddr(&addr, &mut storage);
        sock_check(unsafe {
            connect(self.res.socket(), &storage as *const _ as *const SOCKADDR, len)
        })
    }

    /// Sends `buf` as one datagram to the connected peer (see
    /// [`connect`](Self::connect)); resolves to the byte count and hands the
    /// buffer back.
    pub fn send(
        &self,
        buf: Vec<u8>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<u8>)> + Send + use<> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                let wsabuf = WSABUF {
                    len: buf.len() as u32,
                    buf: windows_core::PSTR(buf.as_mut_ptr()),
                };
                let r = WSASend(sock, &[wsabuf], None, 0, Some(ovl), None);
                classify_wsa_overlapped(r == 0)
            })
        };
        async move {
            let (result, buf) = fut.await;
            (result, buf)
        }
    }

    /// Receives one datagram from the connected peer into `buf`; resolves to the
    /// byte count and hands the buffer back. Truncation fails with `WSAEMSGSIZE`
    /// (see [`recv_from`](Self::recv_from)).
    pub fn recv(
        &self,
        buf: Vec<u8>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<u8>)> + Send + use<> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, RecvPayload { buf, flags: 0 }, move |ovl, p| {
                let wsabuf = WSABUF {
                    len: p.buf.len() as u32,
                    buf: windows_core::PSTR(p.buf.as_mut_ptr()),
                };
                let r = WSARecv(sock, &[wsabuf], None, &mut p.flags, Some(ovl), None);
                classify_wsa_overlapped(r == 0)
            })
        };
        async move {
            let (result, payload) = fut.await;
            (result, payload.buf)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{block_on, timeout, Runtime};
    use super::*;
    use std::time::Duration;

    #[test]
    fn tcp_echo_self_connect() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();
            assert_ne!(addr.port(), 0);

            // Echo server: accept one connection, echo until the peer
            // half-closes, then half-close back.
            let server = rt.spawn(async move {
                let stream = listener.accept().await.unwrap();
                // Peer address is available post-SO_UPDATE_ACCEPT_CONTEXT.
                assert!(stream.peer_addr().is_ok());
                let mut total = 0usize;
                let mut buf = vec![0u8; 4096];
                loop {
                    let (n, b) = stream.read(buf).await;
                    let n = n.unwrap();
                    if n == 0 {
                        break;
                    }
                    total += n;
                    let (w, b2) = stream.write(b[..n].to_vec()).await;
                    assert_eq!(w.unwrap(), n);
                    buf = b2;
                    buf.resize(4096, 0);
                }
                stream.shutdown_send().unwrap();
                total
            });

            // Client: send a payload, read the echo back, then half-close.
            let client = TcpStream::connect(&rt, addr).await.unwrap();
            // The client's peer address is the listener's address.
            assert_eq!(client.peer_addr().unwrap().port(), addr.port());
            let payload: Vec<u8> = (0..100_000u32).map(|i| (i % 231) as u8).collect();
            let mut sent = 0;
            while sent < payload.len() {
                let chunk = payload[sent..(sent + 8192).min(payload.len())].to_vec();
                let (w, _) = client.write(chunk).await;
                sent += w.unwrap();
            }

            let mut echoed = Vec::with_capacity(payload.len());
            while echoed.len() < payload.len() {
                let (n, b) = client.read(vec![0u8; 8192]).await;
                let n = n.unwrap();
                assert_ne!(n, 0, "server closed early");
                echoed.extend_from_slice(&b[..n]);
            }
            assert_eq!(echoed, payload);

            client.shutdown_send().unwrap();
            // Server saw exactly the payload and closed its side.
            assert_eq!(server.await.unwrap(), payload.len());
            let (n, _) = client.read(vec![0u8; 16]).await;
            assert_eq!(n.unwrap(), 0);
        }));
        result.expect("echo test timed out");
    }

    #[test]
    fn tcp_echo_ipv6() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0)),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();
            assert!(matches!(addr, SocketAddr::V6(_)), "expected an IPv6 bind");
            assert_ne!(addr.port(), 0);

            let server = rt.spawn(async move {
                let stream = listener.accept().await.unwrap();
                let (n, b) = stream.read(vec![0u8; 64]).await;
                let n = n.unwrap();
                stream.write(b[..n].to_vec()).await.0.unwrap();
            });

            let client = TcpStream::connect(&rt, addr).await.unwrap();
            assert!(matches!(client.peer_addr().unwrap(), SocketAddr::V6(_)));
            client.write(b"v6ping".to_vec()).await.0.unwrap();
            let (n, b) = client.read(vec![0u8; 64]).await;
            assert_eq!(&b[..n.unwrap()], b"v6ping");
            server.await.unwrap();
        }));
        result.expect("ipv6 echo test timed out");
    }

    #[test]
    fn udp_send_recv_roundtrip() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let server = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            )
            .unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            )
            .unwrap();
            let server_addr = server.local_addr().unwrap();
            let client_addr = client.local_addr().unwrap();

            // Client -> server.
            let (w, _) = client.send_to(b"ping".to_vec(), server_addr).await;
            assert_eq!(w.unwrap(), 4);

            let (r, b) = server.recv_from(vec![0u8; 64]).await;
            let (n, from) = r.unwrap();
            assert_eq!(&b[..n], b"ping");
            // The datagram's source is the client's bound address.
            assert_eq!(from.port(), client_addr.port());

            // Server echoes back to the reported source.
            let (w, _) = server.send_to(b"pong".to_vec(), from).await;
            assert_eq!(w.unwrap(), 4);
            let (r, b) = client.recv_from(vec![0u8; 64]).await;
            let (n, _) = r.unwrap();
            assert_eq!(&b[..n], b"pong");
        }));
        result.expect("udp roundtrip timed out");
    }

    #[test]
    fn udp_ipv6_roundtrip() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let server = UdpSocket::bind(
                &rt,
                SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0)),
            )
            .unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0)),
            )
            .unwrap();
            let server_addr = server.local_addr().unwrap();
            assert!(matches!(server_addr, SocketAddr::V6(_)));

            client.send_to(b"v6dgram".to_vec(), server_addr).await.0.unwrap();
            let (r, b) = server.recv_from(vec![0u8; 64]).await;
            let (n, from) = r.unwrap();
            assert_eq!(&b[..n], b"v6dgram");
            assert!(matches!(from, SocketAddr::V6(_)));
        }));
        result.expect("udp ipv6 roundtrip timed out");
    }

    #[test]
    fn udp_connected_send_recv() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let server = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            )
            .unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            )
            .unwrap();
            let server_addr = server.local_addr().unwrap();
            let client_addr = client.local_addr().unwrap();

            // Pin each socket to the other so send/recv work without addresses.
            client.connect(server_addr).unwrap();
            server.connect(client_addr).unwrap();

            client.send(b"hi".to_vec()).await.0.unwrap();
            let (n, b) = server.recv(vec![0u8; 64]).await;
            assert_eq!(&b[..n.unwrap()], b"hi");

            server.send(b"yo".to_vec()).await.0.unwrap();
            let (n, b) = client.recv(vec![0u8; 64]).await;
            assert_eq!(&b[..n.unwrap()], b"yo");
        }));
        result.expect("udp connected send/recv timed out");
    }

    #[test]
    fn transmit_file_over_tcp() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let path = {
                let mut p = std::env::temp_dir();
                p.push(format!("tp_transmit_{}.bin", std::process::id()));
                p.to_string_lossy().into_owned()
            };
            let content: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
            let file = crate::tp::File::create(&rt, &path).unwrap();
            let (w, _) = file.write_at(content.clone(), 0).await;
            assert_eq!(w.unwrap(), content.len());

            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();

            // Server streams the whole file over the socket, then half-closes.
            let server = rt.spawn(async move {
                let stream = listener.accept().await.unwrap();
                let n = stream.transmit_file(&file, 0, 0, TransmitFlags::default()).await.unwrap();
                stream.shutdown_send().unwrap();
                n
            });

            let client = TcpStream::connect(&rt, addr).await.unwrap();
            let mut got = Vec::new();
            loop {
                let (n, b) = client.read(vec![0u8; 8192]).await;
                let n = n.unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&b[..n]);
            }
            let sent = server.await.unwrap();
            (sent, got, content, path)
        }));
        let (sent, got, content, path) = result.expect("transmit_file timed out");
        assert_eq!(sent, content.len());
        assert_eq!(got, content);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn transmit_packets_mem_and_file() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let path = {
                let mut p = std::env::temp_dir();
                p.push(format!("tp_packets_{}.bin", std::process::id()));
                p.to_string_lossy().into_owned()
            };
            let body: Vec<u8> = (0..8000u32).map(|i| (i % 251) as u8).collect();
            let file = crate::tp::File::create(&rt, &path).unwrap();
            file.write_at(body.clone(), 0).await.0.unwrap();
            let header = b"HDR:".to_vec();

            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();

            // Server scatters an in-memory header + the whole file in one call.
            let hdr = header.clone();
            let server = rt.spawn(async move {
                let stream = listener.accept().await.unwrap();
                let pk = TransmitPackets::new().bytes(hdr).file(&file, 0, 0);
                let n = stream.transmit_packets(pk, TransmitFlags::default()).await.unwrap();
                stream.shutdown_send().unwrap();
                n
            });

            let client = TcpStream::connect(&rt, addr).await.unwrap();
            let mut got = Vec::new();
            loop {
                let (n, b) = client.read(vec![0u8; 8192]).await;
                let n = n.unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&b[..n]);
            }
            let sent = server.await.unwrap();
            (sent, got, header, body, path)
        }));
        let (sent, got, header, body, path) = result.expect("transmit_packets timed out");
        let mut expected = header;
        expected.extend_from_slice(&body);
        assert_eq!(got, expected, "scattered header+file bytes mismatch");
        assert_eq!(sent, expected.len());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn disconnect_only_completes() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(5), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();
            let client = TcpStream::connect(&rt, addr).await.unwrap();
            let s1 = listener.accept().await.unwrap();
            // DisconnectEx is graceful: it completes once the peer has closed too,
            // so close the server side first (the connection is finished).
            drop(s1);
            client.disconnect(true).await.unwrap();
        }));
        result.expect("disconnect alone timed out");
    }

    #[test]
    fn disconnect_reconnect_client() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();

            // First connection on a fresh client socket.
            let client = TcpStream::connect(&rt, addr).await.unwrap();
            let s1 = listener.accept().await.unwrap();
            client.write(b"one".to_vec()).await.0.unwrap();
            let (n, b) = s1.read(vec![0u8; 8]).await;
            assert_eq!(&b[..n.unwrap()], b"one");

            // Finish the connection (close the peer), then disconnect + reuse the
            // SAME client handle and reconnect — no new socket()/bind()/register.
            drop(s1);
            client.disconnect(true).await.unwrap();
            client.reconnect(addr).await.unwrap();

            let s2 = listener.accept().await.unwrap();
            client.write(b"two".to_vec()).await.0.unwrap();
            let (n, b) = s2.read(vec![0u8; 8]).await;
            assert_eq!(&b[..n.unwrap()], b"two");
        }));
        result.expect("disconnect/reconnect timed out");
    }

    #[test]
    fn accept_into_server_reuse() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();

            let c1 = TcpStream::connect(&rt, addr).await.unwrap();
            let s1 = listener.accept().await.unwrap();
            c1.write(b"a".to_vec()).await.0.unwrap();
            let (n, b) = s1.read(vec![0u8; 4]).await;
            assert_eq!(&b[..n.unwrap()], b"a");

            // Finish the connection (close the client), then the server disconnects
            // its accepted socket for reuse and hands it to the next accept instead
            // of creating a new one.
            drop(c1);
            s1.disconnect(true).await.unwrap();
            let reusable = s1.into_reusable();

            let c2 = TcpStream::connect(&rt, addr).await.unwrap();
            let s2 = listener.accept_into(reusable).await.unwrap();
            c2.write(b"b".to_vec()).await.0.unwrap();
            let (n, b) = s2.read(vec![0u8; 4]).await;
            assert_eq!(&b[..n.unwrap()], b"b");
        }));
        result.expect("accept_into reuse timed out");
    }

    #[test]
    fn cancel_storm_accept_and_read() {
        let rt = Runtime::new();
        block_on(timeout(&rt, Duration::from_secs(30), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();

            // Repeatedly start accepts with no client and drop them mid-flight.
            for _ in 0..1000 {
                drop(listener.accept());
            }

            // A connected pair; reads never complete (peer sends nothing), so
            // dropping the future exercises CancelIoEx on a live op.
            let server_task = rt.spawn({
                let rt = rt.clone();
                async move {
                    let _ = rt; // keep alive
                    listener.accept().await.unwrap()
                }
            });
            let client = TcpStream::connect(&rt, addr).await.unwrap();
            let server = server_task.await.unwrap();
            for _ in 0..1000 {
                drop(client.read(vec![0u8; 1024]));
                drop(server.read(vec![0u8; 1024]));
            }
        }))
        .expect("cancel storm timed out");
        drop(rt);
        #[cfg(debug_assertions)]
        {
            for _ in 0..500 {
                if super::super::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                super::super::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "op blocks leaked"
            );
        }
    }
}
