//! Async Hyper-V sockets (AF_HYPERV, "vsock" on Linux guests) over TP_IO.
//!
//! A Hyper-V socket is a stream between a partition and the host, or between
//! two partitions, that needs no virtual NIC — the transport is VMBus. On the
//! Linux side of a VM the same endpoints appear as `AF_VSOCK`, which is why the
//! service GUID has a template form carrying a 32-bit vsock port.
//!
//! Addressing — `SOCKADDR_HV`, the protocol number, the socket options and the
//! well-known VM GUIDs — lives in [`crate::common::hvaddr`], shared with
//! `comio`'s Hyper-V transport.
//!
//! Streams mirror [`super::unix`]: `AcceptEx` / `ConnectEx` for the handshake,
//! `WSARecv` / `WSASend` for data.

use crate::common::winsock::ExtFnCache;
use super::net::{
    classify_wsa_overlapped, new_socket, sock_check, AcceptPayload, RecvPayload,
    VectoredPayload, ADDR_SLOT,
};
use super::{submit, IoRes};
use crate::common::buffer::{IoBuf, IoBufMut};
use crate::tp::{Runtime, RuntimeInner};
use crate::common::winsock::{connectex, ConnectExFn};
use core::ffi::c_void;
use std::mem;
use std::sync::Arc;
use windows::Win32::Networking::WinSock::{
    bind, listen, setsockopt, AcceptEx, ADDRESS_FAMILY, AF_HYPERV, INVALID_SOCKET,
    IPPROTO, SOCKADDR, SOCKET, SOCK_STREAM, SOL_SOCKET, SOMAXCONN, SO_UPDATE_ACCEPT_CONTEXT,
    SO_UPDATE_CONNECT_CONTEXT, WSABUF, WSARecv, WSASend,
};

// The whole addressing surface, re-exported so callers reach it as
// `tp::io::hvsock::*` rather than through `common`.
#[allow(unused_imports)]
pub use crate::common::hvaddr::{
    local_addr_of, peer_addr_of, vsock_port_of, vsock_service_id, HvSocketAddr,
    HVSOCKET_CONNECTED_SUSPEND, HVSOCKET_CONNECT_TIMEOUT,
    HVSOCKET_CONTAINER_PASSTHRU, HV_GUID_BROADCAST, HV_GUID_CHILDREN, HV_GUID_LOOPBACK,
    HV_GUID_PARENT, HV_GUID_SILOHOST, HV_GUID_WILDCARD, HV_GUID_ZERO, HV_PROTOCOL_RAW,
};

/// `ConnectEx` for the **HvSocket** provider. The signature lives in
/// `common::winsock`; only this cache is local, because the pointer belongs
/// to the provider.
fn get_connectex(sock: SOCKET) -> windows_core::Result<ConnectExFn> {
    static PTR: ExtFnCache = ExtFnCache::new();
    connectex(sock, &PTR)
}

/// Creates an AF_HYPERV stream socket. Fails with `WSAEAFNOSUPPORT` (10047)
/// where the Hyper-V socket provider is not present.
fn hv_socket() -> windows_core::Result<SOCKET> {
    new_socket(
        ADDRESS_FAMILY(AF_HYPERV),
        SOCK_STREAM,
        IPPROTO(HV_PROTOCOL_RAW),
    )
}


/// A connected Hyper-V socket stream.
pub struct HvStream {
    res: Arc<IoRes>,
}

impl HvStream {
    /// Sends (part of) an open overlapped `file`'s contents over the socket
    /// using the kernel's `TransmitFile` -- a zero-copy path straight from
    /// the file-system cache, without staging the bytes through user space.
    ///
    /// `offset` is the starting position in the file; `count == 0` sends from
    /// `offset` to end of file. Resolves to the number of bytes transmitted.
    pub fn transmit_file(
        &self,
        file: &crate::tp::File,
        offset: u64,
        count: u32,
        flags: crate::tp::TransmitFlags,
    ) -> impl Future<Output = windows_core::Result<usize>> + Send + use<> {
        crate::tp::io::net::transmit::transmit_file(&self.res, file, offset, count, flags)
    }

    /// Sends a sequence of in-memory buffers and/or file ranges as one
    /// `TransmitPackets` call -- a single-syscall scatter of mixed memory and
    /// file sources, in order. Resolves to the total number of bytes sent.
    /// See [`TransmitSequence`](crate::tp::TransmitSequence) for what a
    /// sequence is made of.
    pub fn transmit_packets<S: crate::tp::TransmitSequence>(
        &self,
        packets: S,
        flags: crate::tp::TransmitFlags,
    ) -> impl Future<Output = windows_core::Result<usize>> + Send + use<S> {
        crate::tp::io::net::transmit::transmit_packets(&self.res, packets, flags)
    }

    /// Sends a Winsock control code (`WSAIoctl`, overlapped) with `input`,
    /// filling `output`; resolves to the bytes written into `output`, with
    /// both buffers handed back.
    ///
    /// The socket-level counterpart of [`Device::ioctl`](crate::tp::io::device::Device::ioctl),
    /// and it takes buffers the same way. Codes that finish at once
    /// (`SIO_KEEPALIVE_VALS`, `SIO_ADDRESS_LIST_QUERY`) resolve straight away;
    /// codes that wait for something (`SIO_ADDRESS_LIST_CHANGE`,
    /// `SIO_ROUTING_INTERFACE_CHANGE`) resolve when it happens, and dropping
    /// the future cancels them.
    pub fn ioctl<I: IoBuf + 'static, O: IoBufMut + 'static>(
        &self,
        code: u32,
        input: I,
        output: O,
    ) -> impl Future<Output = (windows_core::Result<usize>, I, O)> + Send + use<I, O> {
        crate::tp::io::socket_ioctl(&self.res, code, input, output)
    }

    fn from_socket(rt: &Arc<RuntimeInner>, sock: crate::common::overlapped::IoHandle) -> windows_core::Result<HvStream> {
        let res = crate::tp::io::register_on(rt, sock)?;
        Ok(HvStream { res })
    }

    /// Connects to `addr`. `connect_timeout_ms`, when given, caps how long the
    /// provider waits for the target partition to answer — without it a
    /// connect to a VM that is not listening can hang for the provider's own
    /// (long) default.
    pub async fn connect(
        rt: &Runtime,
        addr: HvSocketAddr,
        connect_timeout_ms: Option<u32>,
    ) -> windows_core::Result<HvStream> {
        let sock = hv_socket()?;
        // Closed on any early return from here on.
        let owned = unsafe { crate::common::overlapped::IoHandle::from_socket(sock) };
        if let Some(ms) = connect_timeout_ms {
            sock_check(unsafe {
                setsockopt(sock, HV_PROTOCOL_RAW, HVSOCKET_CONNECT_TIMEOUT, Some(&ms.to_ne_bytes()))
            })?;
        }
        // ConnectEx requires a bound socket; bind to the wildcard endpoint.
        let (local, local_len) = HvSocketAddr::new(HV_GUID_ZERO, HV_GUID_ZERO).to_raw();
        sock_check(unsafe { bind(sock, &local as *const _ as *const SOCKADDR, local_len) })?;
        get_connectex(sock)?;
        let stream = HvStream::from_socket(&rt.inner, owned)?;
        let connect_ex = get_connectex(sock)?;
        let (remote, remote_len) = addr.to_raw();
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
        fut.await.0?;
        sock_check(unsafe { setsockopt(sock, SOL_SOCKET, SO_UPDATE_CONNECT_CONTEXT, None) })?;
        Ok(stream)
    }

    /// Skips the completion packet whenever an op on this handle completes
    /// **synchronously** — VMBus hands small messages over without
    /// ever blocking. The op is then finished on the issuing
    /// thread, with no pool hand-off and no context switch.
    ///
    /// Enable it before issuing ops. Issue closures report a synchronous
    /// success as a fact and the core applies this mode, so there is no
    /// per-op contract to get wrong; semantics are unchanged, only sooner.
    pub fn set_skip_completion_on_success(&self) -> windows_core::Result<()> {
        self.res.set_skip_completion_on_success()
    }

    /// Which partition is on the other end, and which service it reached.
    ///
    /// For a host service this is the authorization primitive: a listener on
    /// the wildcard VM id accepts from every partition, so this is the only
    /// way to tell *which guest* a connection came from.
    pub fn peer_addr(&self) -> windows_core::Result<HvSocketAddr> {
        peer_addr_of(self.res.socket())
    }

    /// This socket's own AF_HYPERV name.
    pub fn local_addr(&self) -> windows_core::Result<HvSocketAddr> {
        local_addr_of(self.res.socket())
    }

    /// Receives into `buf`; resolves to the byte count (0 = peer closed).
    pub fn read<B: IoBufMut + 'static>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, RecvPayload { buf, flags: 0 }, move |ovl, p| {
                let wsabuf = WSABUF {
                    len: IoBuf::bytes_total(&p.buf) as u32,
                    buf: windows_core::PSTR(p.buf.stable_mut_ptr()),
                };
                let r = WSARecv(sock, &[wsabuf], None, &mut p.flags, Some(ovl), None);
                classify_wsa_overlapped(r == 0)
            })
        };
        async move {
            let (result, payload) = fut.await;
            {
                let n = result.as_ref().ok().copied();
                (result, crate::common::buffer::mark_read(n, payload.buf))
            }
        }
    }

    /// Sends `buf`; resolves to the byte count.
    pub fn write<B: IoBuf + 'static>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let sock = self.res.socket();
        
        unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                let wsabuf = WSABUF {
                    len: IoBuf::len(buf) as u32,
                    buf: windows_core::PSTR(buf.stable_ptr() as *mut u8),
                };
                let r = WSASend(sock, &[wsabuf], None, 0, Some(ovl), None);
                classify_wsa_overlapped(r == 0)
            })
        }
    }

    /// Scatter-reads into `bufs` with one vectored `WSARecv`.
    pub fn read_vectored<B: IoBufMut + 'static>(
        &self,
        bufs: Vec<B>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<B>)> + Send + use<B> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, VectoredPayload::new(bufs), move |ovl, p| {
                p.fill_recv();
                let r = WSARecv(sock, &p.wsabufs, None, &mut p.flags, Some(ovl), None);
                classify_wsa_overlapped(r == 0)
            })
        };
        async move {
            let (result, payload) = fut.await;
            {
                let n = result.as_ref().ok().copied();
                (result, crate::common::buffer::mark_read_vectored(n, payload.bufs))
            }
        }
    }

    /// Gather-writes `bufs` with one vectored `WSASend`.
    pub fn write_vectored<B: IoBuf + 'static>(
        &self,
        bufs: Vec<B>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<B>)> + Send + use<B> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, VectoredPayload::new(bufs), move |ovl, p| {
                p.fill_send();
                let r = WSASend(sock, &p.wsabufs, None, 0, Some(ovl), None);
                classify_wsa_overlapped(r == 0)
            })
        };
        async move {
            let (result, payload) = fut.await;
            (result, payload.bufs)
        }
    }
}

/// A listening Hyper-V socket.
pub struct HvListener {
    res: Arc<IoRes>,
    rt: Runtime,
}

impl HvListener {
    /// Sends a Winsock control code (`WSAIoctl`, overlapped) with `input`,
    /// filling `output`; resolves to the bytes written into `output`, with
    /// both buffers handed back.
    ///
    /// The socket-level counterpart of [`Device::ioctl`](crate::tp::io::device::Device::ioctl),
    /// and it takes buffers the same way. Codes that finish at once
    /// (`SIO_KEEPALIVE_VALS`, `SIO_ADDRESS_LIST_QUERY`) resolve straight away;
    /// codes that wait for something (`SIO_ADDRESS_LIST_CHANGE`,
    /// `SIO_ROUTING_INTERFACE_CHANGE`) resolve when it happens, and dropping
    /// the future cancels them.
    pub fn ioctl<I: IoBuf + 'static, O: IoBufMut + 'static>(
        &self,
        code: u32,
        input: I,
        output: O,
    ) -> impl Future<Output = (windows_core::Result<usize>, I, O)> + Send + use<I, O> {
        crate::tp::io::socket_ioctl(&self.res, code, input, output)
    }

    /// Binds and listens on `addr`.
    ///
    /// Bind the VM id to `HV_GUID_WILDCARD` to accept from any partition. For
    /// a **guest** to reach the service, its GUID must first be registered on
    /// the host — see the registration section in [`crate::common::hvaddr`].
    /// Host-to-host loopback works without it, so a listener that answers
    /// locally may still be unreachable from a VM.
    pub fn bind(rt: &Runtime, addr: HvSocketAddr) -> windows_core::Result<HvListener> {
        let sock = hv_socket()?;
        // Closed on any early return from here on.
        let owned = unsafe { crate::common::overlapped::IoHandle::from_socket(sock) };
        let (sa, len) = addr.to_raw();
        sock_check(unsafe { bind(sock, &sa as *const _ as *const SOCKADDR, len) })?;
        sock_check(unsafe { listen(sock, SOMAXCONN as i32) })?;
        let res = crate::tp::io::register_on(&rt.inner, owned)?;
        Ok(HvListener {
            res,
            rt: rt.clone(),
        })
    }

    /// Accepts one connection.
    pub async fn accept(&self) -> windows_core::Result<HvStream> {
        let accept_sock = hv_socket()?;
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
        let (result, mut payload) = fut.await;
        result?;
        let sock = mem::replace(&mut payload.accept_sock, INVALID_SOCKET);
        // Closed on any early return from here on.
        let owned = unsafe { crate::common::overlapped::IoHandle::from_socket(sock) };
        let listen_val = listen_sock.0.to_ne_bytes();
        sock_check(unsafe { setsockopt(sock, SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, Some(&listen_val)) })?;
        let stream = HvStream::from_socket(&self.rt.inner, owned)?;
        Ok(stream)
    }
}

#[cfg(test)]
#[path = "hvsock_tests.rs"]
mod tests;
