//! Async sockets over TP_IO: `TcpListener` (AcceptEx), `TcpStream`
//! (ConnectEx / WSARecv / WSASend), and `UdpSocket` (WSARecvFrom / WSASendTo).
//! IPv4 and IPv6 are both supported through `std::net::SocketAddr`; the sockaddr
//! marshalling helpers ([`write_sockaddr`]/[`read_sockaddr`]) and socket
//! creation (`new_socket`) are shared across all three.
//!
//! # IPv6 caveats
//! An IPv6 socket is single-stack by default (Windows sets `IPV6_V6ONLY`), so an
//! IPv6 listener does not accept IPv4-mapped connections; bind a second IPv4
//! socket (or clear `IPV6_V6ONLY`) for dual-stack. `flowinfo`/`scope_id` are
//! passed through to/from the OS `sockaddr_in6` as-is (no byte-swap); only
//! non-zero flow labels crossing to a differing stack would be affected.

use crate::common::winsock::{ExtFnCache, WsaSocket};
use super::file::File;
use super::{submit, IoRes, OpFuture};
use crate::common::buffer::{IoBuf, IoBufMut};
use crate::tp::{Runtime, RuntimeInner};
pub(crate) use crate::common::sockaddr::{
    addr_of_socket, family_of, peer_of_socket, read_sockaddr, wildcard, write_sockaddr,
};
pub(crate) use crate::common::winsock::{
    classify_wsa_overlapped, connectex, extension_fn, sock_check, ConnectExFn,
};
use core::ffi::c_void;
use std::mem;
#[allow(unused_imports)]
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::Arc;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Networking::WinSock::{
    IPPROTO_IPV6, IPV6_ADD_MEMBERSHIP, IPV6_DROP_MEMBERSHIP, IPV6_MREQ,
    IPV6_MULTICAST_HOPS, IPV6_MULTICAST_IF, IPV6_MULTICAST_LOOP,
    IP_ADD_MEMBERSHIP, IP_ADD_SOURCE_MEMBERSHIP, IP_BLOCK_SOURCE, IP_DROP_MEMBERSHIP,
    IP_DROP_SOURCE_MEMBERSHIP, IP_MREQ, IP_MREQ_SOURCE, IP_MULTICAST_IF,
    IP_MULTICAST_LOOP, IP_MULTICAST_TTL, IP_UNBLOCK_SOURCE, SO_REUSEADDR,
    getsockopt,
    UDP_COALESCED_INFO, UDP_RECV_MAX_COALESCED_SIZE, UDP_SEND_MSG_SIZE,
    bind, connect, listen, setsockopt, shutdown, AcceptEx, WSARecv, WSARecvFrom, WSASend, WSASendTo,
    ADDRESS_FAMILY, IN6_ADDR, IN6_ADDR_0, IN_ADDR, IN_ADDR_0,
    IPPROTO, IPPROTO_TCP, IPPROTO_UDP, MSG_PEEK, SD_SEND, SOCKADDR,
    SOCKADDR_STORAGE, SOCKET, SOCK_DGRAM, SOCK_STREAM,
    SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT, TF_DISCONNECT,
    TF_REUSE_SOCKET, TP_ELEMENT_FILE, TP_ELEMENT_MEMORY, TRANSMIT_PACKETS_ELEMENT,
    TRANSMIT_PACKETS_ELEMENT_0, TRANSMIT_PACKETS_ELEMENT_0_0, TransmitFile, WINSOCK_SOCKET_TYPE,
    WSABUF, WSAID_DISCONNECTEX, WSAID_TRANSMITPACKETS,
    WSAID_WSARECVMSG, CMSGHDR, IN_PKTINFO, IPPROTO_IP,
    IP_PKTINFO, LPWSAOVERLAPPED_COMPLETION_ROUTINE, WSAMSG, WSASendMsg,
};

/// Creates an overlapped socket of the given family/type/protocol. `WSASocketW`
/// is wrapped by the `windows` crate to map `INVALID_SOCKET` to an `Err`.
/// How a TCP or UDP socket is created. [`SocketOptions::new()`] for the
/// defaults, which is what almost every caller wants.
#[derive(Clone, Copy, Debug, Default)]
pub struct SocketOptions {
    pub(super) wsa_flags: u32,
    pub(super) reuse_address: bool,
}

impl SocketOptions {
    /// The defaults: no extra flags, no address reuse.
    pub fn new() -> Self {
        Self::default()
    }

    /// Extra `WSASocketW` flags -- on a TCP listener, on its listen socket
    /// *and* on every socket `accept` creates.
    ///
    /// For composing with another completion engine: create the sockets the
    /// way that engine needs, do the handshake here, hand the connected
    /// socket over. This module never interprets them.
    pub fn wsa_flags(mut self, flags: u32) -> Self {
        self.wsa_flags = flags;
        self
    }

    /// `SO_REUSEADDR`, set before `bind`, so several sockets on this host can
    /// hold the same port -- the usual arrangement for multicast receivers.
    ///
    /// UDP only: a TCP `bind` or `connect` with it fails with `E_INVALIDARG`.
    /// On Windows the option lets a *later* socket take over a port another
    /// is listening on, which for TCP is a hijack, not a convenience.
    pub fn reuse_address(mut self, on: bool) -> Self {
        self.reuse_address = on;
        self
    }

    /// Refuses what a TCP socket must not have.
    pub(super) fn for_tcp(&self) -> windows_core::Result<&Self> {
        if self.reuse_address {
            return Err(windows_core::Error::new(
                windows::Win32::Foundation::E_INVALIDARG,
                "SO_REUSEADDR lets another socket take over a TCP port; it is for UDP only",
            ));
        }
        Ok(self)
    }
}

pub(super) fn new_socket(
    family: ADDRESS_FAMILY,
    socktype: WINSOCK_SOCKET_TYPE,
    proto: IPPROTO,
) -> windows_core::Result<WsaSocket> {
    new_socket_with(family, socktype, proto, 0)
}

/// `new_socket` plus `extra_flags`, OR-ed into the `WSASocketW` flags
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
) -> windows_core::Result<WsaSocket> {
    crate::common::winsock::socket(family.0 as i32, socktype.0, proto.0, extra_flags)
}


/// `ConnectEx` for the **TCP/IP** provider. The signature lives in
/// `common::winsock`; only this cache is local, because the pointer belongs
/// to the provider.
pub(super) fn get_connectex(sock: SOCKET) -> windows_core::Result<ConnectExFn> {
    static PTR: ExtFnCache = ExtFnCache::new();
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
    static PTR: ExtFnCache = ExtFnCache::new();
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
    static PTR: ExtFnCache = ExtFnCache::new();
    unsafe { extension_fn(sock, &WSAID_DISCONNECTEX, &PTR) }
}


/// AcceptEx address slot: sockaddr size + 16 bytes, per the API contract.
pub(super) const ADDR_SLOT: usize = size_of::<SOCKADDR_STORAGE>() + 16;

/// Owns the pre-created accept socket and address buffer while AcceptEx is in
/// flight. If the op is cancelled the completion callback frees the block and
/// the socket closes with it; on success the socket is moved out.
pub(in crate::tp::io) struct AcceptPayload {
    pub(in crate::tp::io) accept_sock: WsaSocket,
    pub(in crate::tp::io) addr_buf: Box<[u8; 2 * ADDR_SLOT]>,
}

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
    accept_sock: WsaSocket,
    /// `recv_len + 2 * ADDR_SLOT` bytes: data, then local addr, then remote.
    buf: Vec<u8>,
    recv_len: usize,
}

/// Payload of a WSARecv: the buffer plus the flags word, which the provider
/// may write at completion time and therefore must live in the op block.
pub(in crate::tp::io) struct RecvPayload<B> {
    pub(in crate::tp::io) buf: B,
    pub(in crate::tp::io) flags: u32,
}

/// Payload of a **vectored** WSARecv/WSASend: the caller's buffers plus the
/// `WSABUF` array the provider reads. Both live in the op block for the op's
/// duration; the array is built once in `issue` with exact capacity so it never
/// reallocates out from under the kernel's pointer.
pub(in crate::tp::io) struct VectoredPayload<B> {
    pub(in crate::tp::io) bufs: Vec<B>,
    pub(in crate::tp::io) wsabufs: Vec<WSABUF>,
    pub(in crate::tp::io) flags: u32,
}

// WSABUF holds a raw pointer; the whole payload is owned by one op at a time.
unsafe impl<B: Send> Send for VectoredPayload<B> {}

impl<B> VectoredPayload<B> {
    pub(in crate::tp::io) fn new(bufs: Vec<B>) -> Self {
        let n = bufs.len();
        Self {
            bufs,
            wsabufs: Vec::with_capacity(n),
            flags: 0,
        }
    }
}

impl<B: IoBuf> VectoredPayload<B> {
    /// (Re)builds the `WSABUF` array for a **send**, from shared pointers:
    /// the provider only reads these buffers.
    ///
    /// Called from `issue`, i.e. once, after the payload is parked in the op
    /// block — the array is pre-reserved to its exact length so it never
    /// reallocates out from under the kernel's pointer.
    pub(in crate::tp::io) fn fill_send(&mut self) {
        self.wsabufs.clear();
        for b in self.bufs.iter() {
            self.wsabufs.push(WSABUF {
                len: IoBuf::len(b) as u32,
                // WSABUF is one type for both directions; a send never writes
                // through it.
                buf: windows_core::PSTR(b.stable_ptr() as *mut u8),
            });
        }
    }
}

impl<B: IoBufMut> VectoredPayload<B> {
    /// (Re)builds the `WSABUF` array for a **receive**, from mutable
    /// pointers, so the provenance matches what the provider does with them.
    pub(in crate::tp::io) fn fill_recv(&mut self) {
        self.wsabufs.clear();
        for b in self.bufs.iter_mut() {
            self.wsabufs.push(WSABUF {
                len: IoBuf::bytes_total(b) as u32,
                buf: windows_core::PSTR(b.stable_mut_ptr()),
            });
        }
    }
}

mod resolve;
pub(crate) mod transmit;
mod tcp;
mod udp;
#[cfg(test)]
mod tests;

pub use resolve::resolve;
pub use tcp::{ReusableSocket, TcpListener, TcpStream};
pub use transmit::{FileRange, List, Then, TransmitSequence};
// Consumed by tests and by callers of this crate, not by the binary itself.
#[allow(unused_imports)]
pub use udp::{
    build_cmsg, cmsg_send_segment_size, iter_cmsgs, parse_ip_pktinfo, parse_udp_coalesced_info,
    ReceivedMsg, UdpSocket,
};
