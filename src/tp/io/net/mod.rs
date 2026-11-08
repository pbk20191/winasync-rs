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
use super::{submit, IoRes, OpFuture};
use crate::tp::{Runtime, RuntimeInner};
pub(crate) use crate::common::winsock::{
    classify_wsa_overlapped, extension_fn, sock_check, wsa_init,
};
use core::ffi::c_void;
use std::mem;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::{Arc, OnceLock};
use windows::Win32::Foundation::{HANDLE, WIN32_ERROR};
use windows::Win32::Networking::WinSock::{
    IPPROTO_IPV6, IPV6_ADD_MEMBERSHIP, IPV6_DROP_MEMBERSHIP, IPV6_MREQ,
    IPV6_MULTICAST_HOPS, IPV6_MULTICAST_IF, IPV6_MULTICAST_LOOP,
    IP_ADD_MEMBERSHIP, IP_ADD_SOURCE_MEMBERSHIP, IP_BLOCK_SOURCE, IP_DROP_MEMBERSHIP,
    IP_DROP_SOURCE_MEMBERSHIP, IP_MREQ, IP_MREQ_SOURCE, IP_MULTICAST_IF,
    IP_MULTICAST_LOOP, IP_MULTICAST_TTL, IP_UNBLOCK_SOURCE, SO_REUSEADDR,
    getsockopt,
    UDP_COALESCED_INFO, UDP_RECV_MAX_COALESCED_SIZE, UDP_SEND_MSG_SIZE,
    bind, closesocket, connect, getpeername, getsockname, listen, setsockopt, shutdown, AcceptEx, WSARecv, WSARecvFrom, WSASend, WSASendTo, WSASocketW,
    ADDRESS_FAMILY, AF_INET, AF_INET6, IN6_ADDR, IN6_ADDR_0, IN_ADDR, IN_ADDR_0, INVALID_SOCKET,
    IPPROTO, IPPROTO_TCP, IPPROTO_UDP, MSG_PEEK, SD_SEND, SOCKADDR,
    SOCKADDR_IN, SOCKADDR_IN6, SOCKADDR_IN6_0, SOCKADDR_STORAGE, SOCKET, SOCK_DGRAM, SOCK_STREAM,
    SOL_SOCKET, SOMAXCONN, SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT, TF_DISCONNECT,
    TF_REUSE_SOCKET, TP_ELEMENT_FILE, TP_ELEMENT_MEMORY, TRANSMIT_PACKETS_ELEMENT,
    TRANSMIT_PACKETS_ELEMENT_0, TRANSMIT_PACKETS_ELEMENT_0_0, TransmitFile, WINSOCK_SOCKET_TYPE,
    WSABUF, WSAID_CONNECTEX, WSAID_DISCONNECTEX, WSAID_TRANSMITPACKETS,
    WSAID_WSARECVMSG, WSA_FLAG_OVERLAPPED, CMSGHDR, IN_PKTINFO, IPPROTO_IP,
    IP_PKTINFO, LPWSAOVERLAPPED_COMPLETION_ROUTINE, WSAMSG, WSASendMsg,
};

/// Process-wide Winsock initialization; never torn down (`WSACleanup` is
/// deliberately not called).

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
pub(crate) fn write_sockaddr(addr: &SocketAddr, storage: &mut SOCKADDR_STORAGE) -> i32 {
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
pub(crate) fn addr_of_socket(sock: SOCKET) -> windows_core::Result<SocketAddr> {
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

/// `ConnectEx` for the **TCP/IP** provider. Its own cache: the AF_UNIX and
/// AF_HYPERV transports fetch theirs separately, since the pointer belongs to
/// the provider (see [`extension_fn`]).
pub(super) fn get_connectex(sock: SOCKET) -> windows_core::Result<ConnectExFn> {
    static PTR: OnceLock<usize> = OnceLock::new();
    unsafe { extension_fn(sock, &WSAID_CONNECTEX, &PTR) }
}

type TransmitPacketsFn = unsafe extern "system" fn(
    s: SOCKET,
    lppacketarray: *mut TRANSMIT_PACKETS_ELEMENT,
    nelementcount: u32,
    nsendsize: u32,
    lpoverlapped: *mut windows::Win32::System::IO::OVERLAPPED,
    dwflags: u32,
) -> windows_core::BOOL;

/// `TransmitPackets` for the TCP/IP provider.
fn get_transmitpackets(sock: SOCKET) -> windows_core::Result<TransmitPacketsFn> {
    static PTR: OnceLock<usize> = OnceLock::new();
    unsafe { extension_fn(sock, &WSAID_TRANSMITPACKETS, &PTR) }
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

/// `DisconnectEx` for the TCP/IP provider.
fn get_disconnectex(sock: SOCKET) -> windows_core::Result<DisconnectExFn> {
    static PTR: OnceLock<usize> = OnceLock::new();
    unsafe { extension_fn(sock, &WSAID_DISCONNECTEX, &PTR) }
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

/// Payload for [`TcpListener::accept_with_data`]: like [`AcceptPayload`], but
/// the single AcceptEx output buffer holds `recv_len` bytes of first-segment
/// data **before** the two address slots, so it is one heap `Vec` instead of a
/// fixed array.
struct AcceptDataPayload {
    accept_sock: SOCKET,
    /// `recv_len + 2 * ADDR_SLOT` bytes: data, then local addr, then remote.
    buf: Vec<u8>,
    recv_len: usize,
}

impl Drop for AcceptDataPayload {
    fn drop(&mut self) {
        if self.accept_sock != INVALID_SOCKET {
            unsafe { closesocket(self.accept_sock) };
        }
    }
}

unsafe impl Send for AcceptDataPayload {}

/// Payload of a WSARecv: the buffer plus the flags word, which the provider
/// may write at completion time and therefore must live in the op block.
struct RecvPayload {
    buf: Vec<u8>,
    flags: u32,
}

/// Payload of a **vectored** WSARecv/WSASend: the caller's buffers plus the
/// `WSABUF` array the provider reads. Both live in the op block for the op's
/// duration; the array is built once in `issue` with exact capacity so it never
/// reallocates out from under the kernel's pointer.
pub(in crate::tp::io) struct VectoredPayload {
    pub(in crate::tp::io) bufs: Vec<Vec<u8>>,
    pub(in crate::tp::io) wsabufs: Vec<WSABUF>,
    pub(in crate::tp::io) flags: u32,
}

// WSABUF holds a raw pointer; the whole payload is owned by one op at a time.
unsafe impl Send for VectoredPayload {}

impl VectoredPayload {
    pub(in crate::tp::io) fn new(bufs: Vec<Vec<u8>>) -> Self {
        let n = bufs.len();
        Self {
            bufs,
            wsabufs: Vec::with_capacity(n),
            flags: 0,
        }
    }

    /// (Re)builds the `WSABUF` array to point at the owned buffers. Called from
    /// `issue`, i.e. once, after the payload is parked in the op block.
    pub(in crate::tp::io) fn fill(&mut self) {
        self.wsabufs.clear();
        for b in self.bufs.iter_mut() {
            self.wsabufs.push(WSABUF {
                len: b.len() as u32,
                buf: windows_core::PSTR(b.as_mut_ptr()),
            });
        }
    }
}

mod tcp;
mod udp;
#[cfg(test)]
mod tests;

pub use tcp::{ReusableSocket, TcpListener, TcpStream, TransmitPackets};
// Consumed by tests and by callers of this crate, not by the binary itself.
#[allow(unused_imports)]
pub use udp::{
    build_cmsg, cmsg_send_segment_size, iter_cmsgs, parse_ip_pktinfo, parse_udp_coalesced_info,
    ReceivedMsg, UdpSocket,
};
