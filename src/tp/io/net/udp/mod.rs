//! UDP over TP_IO: `UdpSocket` (WSARecvFrom / WSASendTo, plus connected-mode
//! WSARecv / WSASend) and control messages (WSARecvMsg / WSASendMsg, IP_PKTINFO).

use std::os::windows::io::AsHandle;
use crate::common::winsock::ExtFnCache;
use super::*;

/// Payload of a WSARecvFrom: the buffer, the flags word, and the source
/// address slot — all written by the provider at completion time, so all must
/// live in the op block.
struct RecvFromPayload<B> {
    buf: B,
    flags: u32,
    from: SOCKADDR_STORAGE,
    from_len: i32,
}

// SOCKADDR_STORAGE is plain POD; the whole payload is safely sendable.
unsafe impl<B: Send> Send for RecvFromPayload<B> {}

/// Payload of a WSASendTo: the buffer plus the destination address, which the
/// provider reads while the send is in flight.
struct SendToPayload<B> {
    buf: B,
    to: SOCKADDR_STORAGE,
    to_len: i32,
}

unsafe impl<B: Send> Send for SendToPayload<B> {}

/// Vectored counterpart of [`RecvFromPayload`]: the shared buffer/`WSABUF` pair
/// plus the source-address slot the provider fills.
struct VectoredFromPayload<B> {
    v: VectoredPayload<B>,
    from: SOCKADDR_STORAGE,
    from_len: i32,
}
unsafe impl<B: Send> Send for VectoredFromPayload<B> {}

/// Vectored counterpart of [`SendToPayload`].
struct VectoredToPayload<B> {
    v: VectoredPayload<B>,
    to: SOCKADDR_STORAGE,
    to_len: i32,
}
unsafe impl<B: Send> Send for VectoredToPayload<B> {}

/// A bound UDP socket on a [`Runtime`]'s pool. Supports connectionless
/// `send_to`/`recv_from` over IPv4 or IPv6.
pub struct UdpSocket {
    res: Arc<IoRes>,
}

impl UdpSocket {
    /// Sends a sequence of in-memory buffers and/or file ranges as one
    /// `TransmitPackets` call -- a single-syscall scatter of mixed memory and
    /// file sources, transmitted in insertion order. Resolves to the total
    /// number of bytes sent -- one datagram. See [`TransmitPackets`](crate::tp::TransmitPackets)
    /// for building the sequence.
    pub fn transmit_packets(
        &self,
        packets: crate::tp::TransmitPackets,
        flags: crate::tp::TransmitFlags,
    ) -> impl Future<Output = windows_core::Result<usize>> + Send + use<> {
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

    /// Binds a UDP socket to `addr` (IPv4 or IPv6), created as `options`
    /// says -- [`SocketOptions::reuse_address`] for several receivers on one
    /// port. Options that only mean something before `bind` are why this
    /// takes them here rather than offering setters.
    pub fn bind(
        rt: &Runtime,
        addr: SocketAddr,
        options: &SocketOptions,
    ) -> windows_core::Result<UdpSocket> {
        let family = family_of(&addr);
        let sock = new_socket_with(family, SOCK_DGRAM, IPPROTO_UDP, options.wsa_flags)?;
        // Closed on any early return from here on.
        let owned = unsafe { crate::common::overlapped::IoHandle::from_socket(sock) };
        if options.reuse_address {
            let v: u32 = 1;
            sock_check(unsafe { setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, Some(&v.to_ne_bytes())) })?;
        }
        let mut storage = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
        let len = write_sockaddr(&addr, &mut storage);
        sock_check(unsafe { bind(sock, &storage as *const _ as *const SOCKADDR, len) })?;
        let res = crate::tp::io::register_on(&rt.inner, owned)?;
        Ok(UdpSocket { res })
    }

    /// Skips the completion packet whenever an op on this handle completes
    /// **synchronously** — a datagram already sitting in the socket
    /// buffer, or a send that fits in it. The op is then finished on the issuing
    /// thread, with no pool hand-off and no context switch.
    ///
    /// Enable it before issuing ops. Issue closures report a synchronous
    /// success as a fact and the core applies this mode, so there is no
    /// per-op contract to get wrong; semantics are unchanged, only sooner.
    pub fn set_skip_completion_on_success(&self) -> windows_core::Result<()> {
        self.res.set_skip_completion_on_success()
    }

    pub fn local_addr(&self) -> windows_core::Result<SocketAddr> {
        addr_of_socket(self.res.socket())
    }

    /// Sends `buf` as one datagram to `addr`; resolves to the byte count and
    /// hands the buffer back.
    pub fn send_to<B: IoBuf + 'static>(
        &self,
        buf: B,
        addr: SocketAddr,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let sock = self.res.socket();
        let mut to = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
        let to_len = write_sockaddr(&addr, &mut to);
        let fut = unsafe {
            submit(&self.res, SendToPayload { buf, to, to_len }, move |ovl, p| {
                let wsabuf = WSABUF {
                    len: IoBuf::len(&p.buf) as u32,
                    buf: windows_core::PSTR(p.buf.stable_ptr() as *mut u8),
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
    pub fn recv_from<B: IoBufMut + 'static>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<(usize, SocketAddr)>, B)> + Send + use<B> {
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
                        len: IoBuf::bytes_total(&p.buf) as u32,
                        buf: windows_core::PSTR(p.buf.stable_mut_ptr()),
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
            {
                let n = out.as_ref().ok().map(|(n, _)| *n);
                (out, crate::common::buffer::mark_read(n, payload.buf))
            }
        }
    }

    /// Scatter-receives one datagram into `bufs` with a vectored `WSARecvFrom`;
    /// resolves to the byte count plus the source address, and hands the buffers
    /// back. Truncation still fails with `WSAEMSGSIZE`.
    pub fn recv_from_vectored<B: IoBufMut + 'static>(
        &self,
        bufs: Vec<B>,
    ) -> impl Future<Output = (windows_core::Result<(usize, SocketAddr)>, Vec<B>)> + Send + use<B>
    {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(
                &self.res,
                VectoredFromPayload {
                    v: VectoredPayload::new(bufs),
                    from: mem::zeroed::<SOCKADDR_STORAGE>(),
                    from_len: size_of::<SOCKADDR_STORAGE>() as i32,
                },
                move |ovl, p| {
                    p.v.fill_recv();
                    let r = WSARecvFrom(
                        sock,
                        &p.v.wsabufs,
                        None,
                        &mut p.v.flags,
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
            {
                let n = out.as_ref().ok().map(|(n, _)| *n);
                (out, crate::common::buffer::mark_read_vectored(n, payload.v.bufs))
            }
        }
    }

    /// Gather-sends `bufs` as **one** datagram to `addr` with a vectored
    /// `WSASendTo`; resolves to the byte count and hands the buffers back.
    pub fn send_to_vectored<B: IoBuf + 'static>(
        &self,
        bufs: Vec<B>,
        addr: SocketAddr,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<B>)> + Send + use<B> {
        let sock = self.res.socket();
        let mut to = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
        let to_len = write_sockaddr(&addr, &mut to);
        let fut = unsafe {
            submit(
                &self.res,
                VectoredToPayload {
                    v: VectoredPayload::new(bufs),
                    to,
                    to_len,
                },
                move |ovl, p| {
                    p.v.fill_send();
                    let r = WSASendTo(
                        sock,
                        &p.v.wsabufs,
                        None,
                        0,
                        Some(&p.to as *const _ as *const SOCKADDR),
                        p.to_len,
                        Some(ovl),
                        None,
                    );
                    classify_wsa_overlapped(r == 0)
                },
            )
        };
        async move {
            let (result, payload) = fut.await;
            (result, payload.v.bufs)
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
    pub fn send<B: IoBuf + 'static>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                let wsabuf = WSABUF {
                    len: IoBuf::len(buf) as u32,
                    buf: windows_core::PSTR(buf.stable_ptr() as *mut u8),
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

    /// [`send`](Self::send), with a buffer that need not be `'static`:
    /// a borrowed `&[u8]` included. See
    /// [borrowed buffers](crate::common::buffer::IoBuf#borrowed-buffers).
    ///
    /// The operation completes through an event rather than the pool's
    /// completion port, and dropping the future before it finishes cancels
    /// it and waits for that event -- so the borrow cannot end while the
    /// kernel still uses the buffer, and the wait needs no pool thread.
    ///
    /// # Safety
    /// The future must be dropped or run to completion -- never forgotten or
    /// leaked: not `mem::forget`, not a reference cycle, not `Box::leak`, not
    /// an executor that frees tasks without dropping them. A future that
    /// disappears without its `Drop` running leaves the kernel reading from
    /// memory the borrow has already given back.
    /// `event` is a manual-reset event, unsignaled and used by nothing
    /// else until the future is done or dropped; it is left signaled. Owned
    /// (an `OwnedHandle`) it moves into the future; borrowed, the future
    /// cannot outlive it. See [`io_event`](crate::tp::io_event).
    pub unsafe fn send_unchecked<B: IoBuf, E: AsHandle + Send>(
        &self,
        buf: B,
        event: E,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B, E> {
        let sock = self.res.socket().0;
        super::super::evented(&self.res, buf, event, move |mem, buf| unsafe {
            let wsabuf = WSABUF {
                len: IoBuf::len(buf) as u32,
                buf: windows_core::PSTR(buf.stable_ptr() as *mut u8),
            };
            let r = WSASend(SOCKET(sock), &[wsabuf], None, 0, Some(&mut mem.ovl), None);
            classify_wsa_overlapped(r == 0)
        })
    }

    /// Receives one datagram from the connected peer into `buf`; resolves to the
    /// byte count and hands the buffer back. Truncation fails with `WSAEMSGSIZE`
    /// (see [`recv_from`](Self::recv_from)).
    pub fn recv<B: IoBufMut + 'static>(
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

    /// [`recv`](Self::recv), with a buffer that need not be `'static`:
    /// a borrowed `&mut [u8]` included. See
    /// [borrowed buffers](crate::common::buffer::IoBuf#borrowed-buffers).
    ///
    /// The operation completes through an event rather than the pool's
    /// completion port, and dropping the future before it finishes cancels
    /// it and waits for that event -- so the borrow cannot end while the
    /// kernel still uses the buffer, and the wait needs no pool thread.
    ///
    /// # Safety
    /// The future must be dropped or run to completion -- never forgotten or
    /// leaked: not `mem::forget`, not a reference cycle, not `Box::leak`, not
    /// an executor that frees tasks without dropping them. A future that
    /// disappears without its `Drop` running leaves the kernel writing into
    /// memory the borrow has already given back.
    /// `event` is a manual-reset event, unsignaled and used by nothing
    /// else until the future is done or dropped; it is left signaled. Owned
    /// (an `OwnedHandle`) it moves into the future; borrowed, the future
    /// cannot outlive it. See [`io_event`](crate::tp::io_event).
    pub unsafe fn recv_unchecked<B: IoBufMut, E: AsHandle + Send>(
        &self,
        buf: B,
        event: E,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B, E> {
        let sock = self.res.socket().0;
        let fut = super::super::evented(&self.res, buf, event, move |mem, buf| unsafe {
            let wsabuf = WSABUF {
                len: IoBuf::bytes_total(buf) as u32,
                buf: windows_core::PSTR(buf.stable_mut_ptr()),
            };
            let r = WSARecv(SOCKET(sock), &[wsabuf], None, &mut mem.flags, Some(&mut mem.ovl), None);
            classify_wsa_overlapped(r == 0)
        });
        async move {
            let (result, buf) = fut.await;
            let n = result.as_ref().ok().copied();
            (result, crate::common::buffer::mark_read(n, buf))
        }
    }
}

// ---------------------------------------------------------------------------
// UDP control messages (WSARecvMsg / WSASendMsg) — ancillary data (IP_PKTINFO…)
// ---------------------------------------------------------------------------

type WsaRecvMsgFn = unsafe extern "system" fn(
    s: SOCKET,
    lpmsg: *mut WSAMSG,
    lpdwnumberofbytesrecvd: *mut u32,
    lpoverlapped: *mut windows::Win32::System::IO::OVERLAPPED,
    lpcompletionroutine: LPWSAOVERLAPPED_COMPLETION_ROUTINE,
) -> i32;

mod cmsg;
mod msg;
pub use cmsg::*;
pub use msg::*;
