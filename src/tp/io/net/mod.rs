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
pub(crate) use crate::common::sockaddr::{
    addr_of_socket, family_of, peer_of_socket, read_sockaddr, wildcard, write_sockaddr,
};
pub(crate) use crate::common::winsock::{
    classify_wsa_overlapped, connectex, extension_fn, sock_check, wsa_init, ConnectExFn,
};
use core::ffi::c_void;
use std::mem;
#[allow(unused_imports)]
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::{Arc, OnceLock};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Networking::WinSock::{
    IPPROTO_IPV6, IPV6_ADD_MEMBERSHIP, IPV6_DROP_MEMBERSHIP, IPV6_MREQ,
    IPV6_MULTICAST_HOPS, IPV6_MULTICAST_IF, IPV6_MULTICAST_LOOP,
    IP_ADD_MEMBERSHIP, IP_ADD_SOURCE_MEMBERSHIP, IP_BLOCK_SOURCE, IP_DROP_MEMBERSHIP,
    IP_DROP_SOURCE_MEMBERSHIP, IP_MREQ, IP_MREQ_SOURCE, IP_MULTICAST_IF,
    IP_MULTICAST_LOOP, IP_MULTICAST_TTL, IP_UNBLOCK_SOURCE, SO_REUSEADDR,
    getsockopt,
    UDP_COALESCED_INFO, UDP_RECV_MAX_COALESCED_SIZE, UDP_SEND_MSG_SIZE,
    bind, closesocket, connect, listen, setsockopt, shutdown, AcceptEx, WSARecv, WSARecvFrom, WSASend, WSASendTo, WSASocketW,
    ADDRESS_FAMILY, IN6_ADDR, IN6_ADDR_0, IN_ADDR, IN_ADDR_0, INVALID_SOCKET,
    IPPROTO, IPPROTO_TCP, IPPROTO_UDP, MSG_PEEK, SD_SEND, SOCKADDR,
    SOCKADDR_STORAGE, SOCKET, SOCK_DGRAM, SOCK_STREAM,
    SOL_SOCKET, SOMAXCONN, SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT, TF_DISCONNECT,
    TF_REUSE_SOCKET, TP_ELEMENT_FILE, TP_ELEMENT_MEMORY, TRANSMIT_PACKETS_ELEMENT,
    TRANSMIT_PACKETS_ELEMENT_0, TRANSMIT_PACKETS_ELEMENT_0_0, TransmitFile, WINSOCK_SOCKET_TYPE,
    WSABUF, WSAID_DISCONNECTEX, WSAID_TRANSMITPACKETS,
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
    new_socket_with(family, socktype, proto, 0)
}

/// [`new_socket`] plus `extra_flags`, OR-ed into the `WSASocketW` flags
/// beside `WSA_FLAG_OVERLAPPED`.
///
/// The seam another engine needs: a socket may have to be created with flags
/// this module knows nothing about — `WSA_FLAG_REGISTERED_IO`, say — and
/// still be driven through this one's overlapped accept and connect. Keeping
/// the parameter opaque means `tp` does not grow a dependency on whatever
/// wants the flag.
pub(crate) fn new_socket_with(
    family: ADDRESS_FAMILY,
    socktype: WINSOCK_SOCKET_TYPE,
    proto: IPPROTO,
    extra_flags: u32,
) -> windows_core::Result<SOCKET> {
    wsa_init();
    unsafe {
        WSASocketW(
            family.0 as i32,
            socktype.0,
            proto.0,
            None,
            0,
            WSA_FLAG_OVERLAPPED | extra_flags,
        )
    }
}


/// `ConnectEx` for the **TCP/IP** provider. The signature lives in
/// `common::winsock`; only this cache is local, because the pointer belongs
/// to the provider.
pub(super) fn get_connectex(sock: SOCKET) -> windows_core::Result<ConnectExFn> {
    static PTR: OnceLock<usize> = OnceLock::new();
    connectex(sock, &PTR)
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
pub(in crate::tp::io) struct AcceptPayload {
    pub(in crate::tp::io) accept_sock: SOCKET,
    pub(in crate::tp::io) addr_buf: Box<[u8; 2 * ADDR_SLOT]>,
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
pub(in crate::tp::io) struct RecvPayload {
    pub(in crate::tp::io) buf: Vec<u8>,
    pub(in crate::tp::io) flags: u32,
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
