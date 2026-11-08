//! `SOCKADDR` conversions, shared by every socket engine here.
//!
//! Pure address plumbing — no IO, no runtime — so `tp` and `rio` convert the
//! same way rather than each keeping a copy of the union punning.

use std::mem;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use windows::Win32::Networking::WinSock::{
    getpeername, getsockname, ADDRESS_FAMILY, AF_INET, AF_INET6, IN6_ADDR, IN6_ADDR_0, IN_ADDR,
    IN_ADDR_0, SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6, SOCKADDR_IN6_0, SOCKADDR_STORAGE, SOCKET,
};

use super::winsock::sock_check;
use windows::Win32::Foundation::WIN32_ERROR;

/// The address family a `SocketAddr` needs its socket created with.
pub fn family_of(addr: &SocketAddr) -> ADDRESS_FAMILY {
    match addr {
        SocketAddr::V4(_) => AF_INET,
        SocketAddr::V6(_) => AF_INET6,
    }
}

/// The `0.0.0.0:0` / `[::]:0` wildcard of a family, for the implicit local bind
/// `ConnectEx` requires.
pub fn wildcard(family: ADDRESS_FAMILY) -> SocketAddr {
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
pub fn write_sockaddr(addr: &SocketAddr, storage: &mut SOCKADDR_STORAGE) -> i32 {
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
pub fn read_sockaddr(storage: &SOCKADDR_STORAGE) -> windows_core::Result<SocketAddr> {
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
pub fn addr_of_socket(sock: SOCKET) -> windows_core::Result<SocketAddr> {
    let mut storage = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
    let mut len = size_of::<SOCKADDR_STORAGE>() as i32;
    sock_check(unsafe { getsockname(sock, &mut storage as *mut _ as *mut SOCKADDR, &mut len) })?;
    read_sockaddr(&storage)
}

/// `getpeername` for a connected socket, as a `SocketAddr`.
pub fn peer_of_socket(sock: SOCKET) -> windows_core::Result<SocketAddr> {
    let mut storage = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
    let mut len = size_of::<SOCKADDR_STORAGE>() as i32;
    sock_check(unsafe { getpeername(sock, &mut storage as *mut _ as *mut SOCKADDR, &mut len) })?;
    read_sockaddr(&storage)
}
