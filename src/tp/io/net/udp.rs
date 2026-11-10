//! UDP over TP_IO: `UdpSocket` (WSARecvFrom / WSASendTo, plus connected-mode
//! WSARecv / WSASend) and control messages (WSARecvMsg / WSASendMsg, IP_PKTINFO).

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
        let res = IoRes::register(&rt.inner, owned)?;
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
    pub unsafe fn send_unchecked<B: IoBuf>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let sock = self.res.socket().0;
        super::super::evented(&self.res, buf, move |mem, buf| unsafe {
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
    pub unsafe fn recv_unchecked<B: IoBufMut>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let sock = self.res.socket().0;
        let fut = super::super::evented(&self.res, buf, move |mem, buf| unsafe {
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

/// `WSARecvMsg` for the TCP/IP provider. (`WSASendMsg` *is* a direct export,
/// so it needs no lookup.)
fn get_wsarecvmsg(sock: SOCKET) -> windows_core::Result<WsaRecvMsgFn> {
    static PTR: ExtFnCache = ExtFnCache::new();
    unsafe { extension_fn(sock, &WSAID_WSARECVMSG, &PTR) }
}

/// Op-block payload for `recv_msg`: the data buffer, the control (ancillary)
/// buffer, the source address, and the `WSABUF`/`WSAMSG` the kernel reads
/// (self-referential pointers into this same payload, valid for the op).
struct RecvMsgPayload {
    data: Vec<u8>,
    control: Vec<u8>,
    source: SOCKADDR_STORAGE,
    wsabuf: WSABUF,
    msg: WSAMSG,
}
unsafe impl Send for RecvMsgPayload {}

/// Op-block payload for `send_msg` (dest address + control, same discipline).
struct SendMsgPayload {
    data: Vec<u8>,
    control: Vec<u8>,
    dest: SOCKADDR_STORAGE,
    wsabuf: WSABUF,
    msg: WSAMSG,
}
unsafe impl Send for SendMsgPayload {}

/// Outcome of [`UdpSocket::recv_msg`]: the datagram size, its source, and the
/// raw ancillary-data (control) bytes (parse with [`parse_ip_pktinfo`]).
pub struct ReceivedMsg {
    pub bytes: usize,
    pub source: SocketAddr,
    pub control: Vec<u8>,
}

impl UdpSocket {
    /// Enables `IP_PKTINFO` so `recv_msg` receives the local interface/address a
    /// datagram arrived on as an `IN_PKTINFO` control message (IPv4).
    /// Enables **UDP Segmentation Offload** (USO): hands the stack one large
    /// buffer and lets it cut the buffer into datagrams of `segment_size`
    /// bytes, instead of making one `WSASendTo` call per datagram.
    ///
    /// The last segment may be short. `None` turns it off.
    ///
    /// This is the cheap end of high-rate UDP sending: the saving is one
    /// user/kernel transition per *batch* rather than per datagram, with no
    /// change to the buffer model and no separate engine — the stack (and, if
    /// it supports it, the NIC) does the segmentation. Where the hardware does
    /// not offload, the stack segments in software and the win is smaller but
    /// still positive.
    ///
    /// Applies to every subsequent send on this socket; for a per-send size,
    /// pass [`cmsg_send_segment_size`] to [`send_msg`](Self::send_msg)
    /// instead.
    pub fn set_send_segment_size(&self, segment_size: Option<u16>) -> windows_core::Result<()> {
        let v: u32 = segment_size.unwrap_or(0) as u32;
        sock_check(unsafe {
            setsockopt(
                self.res.socket(),
                IPPROTO_UDP.0,
                UDP_SEND_MSG_SIZE,
                Some(&v.to_ne_bytes()),
            )
        })
    }

    // ---- Multicast -------------------------------------------------------
    //
    // Joins are per (group, interface): a socket joins on one interface at a
    // time, and joining on several means several calls. Bind the socket
    // before joining — a membership is attached to the bound port.

    fn set_udp_opt(&self, level: i32, name: i32, value: &[u8]) -> windows_core::Result<()> {
        sock_check(unsafe { setsockopt(self.res.socket(), level, name, Some(value)) })
    }

    fn mreq_v4(group: Ipv4Addr, interface: Ipv4Addr) -> IP_MREQ {
        IP_MREQ {
            imr_multiaddr: IN_ADDR {
                S_un: IN_ADDR_0 {
                    S_addr: u32::from(group).to_be(),
                },
            },
            imr_interface: IN_ADDR {
                S_un: IN_ADDR_0 {
                    S_addr: u32::from(interface).to_be(),
                },
            },
        }
    }

    fn mreq_source_v4(group: Ipv4Addr, source: Ipv4Addr, interface: Ipv4Addr) -> IP_MREQ_SOURCE {
        IP_MREQ_SOURCE {
            imr_multiaddr: IN_ADDR {
                S_un: IN_ADDR_0 {
                    S_addr: u32::from(group).to_be(),
                },
            },
            imr_sourceaddr: IN_ADDR {
                S_un: IN_ADDR_0 {
                    S_addr: u32::from(source).to_be(),
                },
            },
            imr_interface: IN_ADDR {
                S_un: IN_ADDR_0 {
                    S_addr: u32::from(interface).to_be(),
                },
            },
        }
    }

    fn as_bytes<T>(v: &T) -> &[u8] {
        unsafe { core::slice::from_raw_parts(v as *const T as *const u8, size_of::<T>()) }
    }

    /// Joins an IPv4 multicast group on `interface` (`Ipv4Addr::UNSPECIFIED`
    /// lets the stack choose). Any-source multicast: traffic from every
    /// sender to the group is accepted.
    pub fn join_multicast_v4(
        &self,
        group: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> windows_core::Result<()> {
        let m = Self::mreq_v4(group, interface);
        self.set_udp_opt(IPPROTO_IP.0, IP_ADD_MEMBERSHIP, Self::as_bytes(&m))
    }

    /// Leaves a group joined by [`join_multicast_v4`](Self::join_multicast_v4).
    pub fn leave_multicast_v4(
        &self,
        group: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> windows_core::Result<()> {
        let m = Self::mreq_v4(group, interface);
        self.set_udp_opt(IPPROTO_IP.0, IP_DROP_MEMBERSHIP, Self::as_bytes(&m))
    }

    /// Joins an IPv6 multicast group on the interface with index
    /// `interface_index` (0 = let the stack choose).
    pub fn join_multicast_v6(
        &self,
        group: Ipv6Addr,
        interface_index: u32,
    ) -> windows_core::Result<()> {
        let m = IPV6_MREQ {
            ipv6mr_multiaddr: IN6_ADDR {
                u: IN6_ADDR_0 { Byte: group.octets() },
            },
            ipv6mr_interface: interface_index,
        };
        self.set_udp_opt(IPPROTO_IPV6.0, IPV6_ADD_MEMBERSHIP, Self::as_bytes(&m))
    }

    /// Leaves a group joined by [`join_multicast_v6`](Self::join_multicast_v6).
    pub fn leave_multicast_v6(
        &self,
        group: Ipv6Addr,
        interface_index: u32,
    ) -> windows_core::Result<()> {
        let m = IPV6_MREQ {
            ipv6mr_multiaddr: IN6_ADDR {
                u: IN6_ADDR_0 { Byte: group.octets() },
            },
            ipv6mr_interface: interface_index,
        };
        self.set_udp_opt(IPPROTO_IPV6.0, IPV6_DROP_MEMBERSHIP, Self::as_bytes(&m))
    }

    /// Source-specific multicast (SSM): joins `group` but accepts traffic from
    /// `source` only, filtering in the network rather than in user code.
    pub fn join_multicast_source_v4(
        &self,
        group: Ipv4Addr,
        source: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> windows_core::Result<()> {
        let m = Self::mreq_source_v4(group, source, interface);
        self.set_udp_opt(IPPROTO_IP.0, IP_ADD_SOURCE_MEMBERSHIP, Self::as_bytes(&m))
    }

    /// Leaves one source of an SSM join.
    pub fn leave_multicast_source_v4(
        &self,
        group: Ipv4Addr,
        source: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> windows_core::Result<()> {
        let m = Self::mreq_source_v4(group, source, interface);
        self.set_udp_opt(IPPROTO_IP.0, IP_DROP_SOURCE_MEMBERSHIP, Self::as_bytes(&m))
    }

    /// Blocks one source on an *any-source* join — the inverse filter to
    /// [`join_multicast_source_v4`](Self::join_multicast_source_v4).
    pub fn block_multicast_source_v4(
        &self,
        group: Ipv4Addr,
        source: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> windows_core::Result<()> {
        let m = Self::mreq_source_v4(group, source, interface);
        self.set_udp_opt(IPPROTO_IP.0, IP_BLOCK_SOURCE, Self::as_bytes(&m))
    }

    /// Unblocks a source blocked by
    /// [`block_multicast_source_v4`](Self::block_multicast_source_v4).
    pub fn unblock_multicast_source_v4(
        &self,
        group: Ipv4Addr,
        source: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> windows_core::Result<()> {
        let m = Self::mreq_source_v4(group, source, interface);
        self.set_udp_opt(IPPROTO_IP.0, IP_UNBLOCK_SOURCE, Self::as_bytes(&m))
    }

    /// Hop limit for outgoing IPv4 multicast (default 1 — link-local only).
    pub fn set_multicast_ttl_v4(&self, ttl: u32) -> windows_core::Result<()> {
        self.set_udp_opt(IPPROTO_IP.0, IP_MULTICAST_TTL, &ttl.to_ne_bytes())
    }

    /// Hop limit for outgoing IPv6 multicast (default 1).
    pub fn set_multicast_hops_v6(&self, hops: u32) -> windows_core::Result<()> {
        self.set_udp_opt(IPPROTO_IPV6.0, IPV6_MULTICAST_HOPS, &hops.to_ne_bytes())
    }

    /// Interface for outgoing IPv4 multicast.
    pub fn set_multicast_if_v4(&self, interface: Ipv4Addr) -> windows_core::Result<()> {
        let addr = u32::from(interface).to_be();
        self.set_udp_opt(IPPROTO_IP.0, IP_MULTICAST_IF, &addr.to_ne_bytes())
    }

    /// Interface index for outgoing IPv6 multicast.
    pub fn set_multicast_if_v6(&self, interface_index: u32) -> windows_core::Result<()> {
        self.set_udp_opt(
            IPPROTO_IPV6.0,
            IPV6_MULTICAST_IF,
            &interface_index.to_ne_bytes(),
        )
    }

    /// **Winsock's `IP_MULTICAST_LOOP` is not the BSD/UNIX one.**
    ///
    /// On UNIX the option governs the *send* path: whether a datagram this
    /// socket sends is looped back to local members. On Winsock it governs
    /// the *receive* path: whether this socket receives datagrams sent from
    /// the local host. Code ported either way silently targets the wrong
    /// direction, because both platforms accept the call and neither errors.
    ///
    /// Set it on the socket whose *reception* you want to change.
    pub fn set_multicast_loop_v4(&self, on: bool) -> windows_core::Result<()> {
        let v: u32 = on as u32;
        self.set_udp_opt(IPPROTO_IP.0, IP_MULTICAST_LOOP, &v.to_ne_bytes())
    }

    /// IPv6 counterpart of
    /// [`set_multicast_loop_v4`](Self::set_multicast_loop_v4) — same Winsock
    /// receive-path semantics.
    pub fn set_multicast_loop_v6(&self, on: bool) -> windows_core::Result<()> {
        let v: u32 = on as u32;
        self.set_udp_opt(IPPROTO_IPV6.0, IPV6_MULTICAST_LOOP, &v.to_ne_bytes())
    }

    /// Reads back the socket-wide USO segment size; `None` when off.
    pub fn send_segment_size(&self) -> windows_core::Result<Option<u16>> {
        Ok(match self.get_udp_dword(UDP_SEND_MSG_SIZE)? {
            0 => None,
            v => Some(v as u16),
        })
    }

    /// Reads back the URO coalescing limit; `None` when off.
    pub fn recv_coalesced_size(&self) -> windows_core::Result<Option<u32>> {
        Ok(match self.get_udp_dword(UDP_RECV_MAX_COALESCED_SIZE)? {
            0 => None,
            v => Some(v),
        })
    }

    /// `getsockopt(IPPROTO_UDP, name)` for the DWORD-valued UDP options,
    /// matching the SDK's inline wrappers (which pass `sizeof(DWORD)` in and
    /// let the provider write the length back).
    fn get_udp_dword(&self, name: i32) -> windows_core::Result<u32> {
        let mut v: u32 = 0;
        let mut len = size_of::<u32>() as i32;
        sock_check(unsafe {
            getsockopt(
                self.res.socket(),
                IPPROTO_UDP.0,
                name,
                windows_core::PSTR(&mut v as *mut u32 as *mut u8),
                &mut len,
            )
        })?;
        Ok(v)
    }

    /// Enables **UDP Receive Coalescing** (URO): lets the stack deliver several
    /// datagrams in one receive, up to `max_coalesced_size` bytes total.
    ///
    /// The receive buffer must be at least that large. The segment size the
    /// stack used is reported per receive in a `UDP_COALESCED_INFO` control
    /// message — read it with [`parse_udp_coalesced_info`], since without it
    /// the datagram boundaries inside the buffer are unknowable.
    ///
    /// `None` turns it off.
    pub fn set_recv_coalesced_size(&self, max_coalesced_size: Option<u32>) -> windows_core::Result<()> {
        let v: u32 = max_coalesced_size.unwrap_or(0);
        sock_check(unsafe {
            setsockopt(
                self.res.socket(),
                IPPROTO_UDP.0,
                UDP_RECV_MAX_COALESCED_SIZE,
                Some(&v.to_ne_bytes()),
            )
        })
    }

    pub fn set_recv_pktinfo(&self, on: bool) -> windows_core::Result<()> {
        let v: i32 = on as i32;
        sock_check(unsafe {
            setsockopt(self.res.socket(), IPPROTO_IP.0, IP_PKTINFO, Some(&v.to_ne_bytes()))
        })
    }

    /// Receives one datagram with ancillary data via `WSARecvMsg`. `control_cap`
    /// sizes the control buffer. Resolves to [`ReceivedMsg`] plus the data buffer.
    pub fn recv_msg(
        &self,
        buf: Vec<u8>,
        control_cap: usize,
    ) -> impl Future<Output = (windows_core::Result<ReceivedMsg>, Vec<u8>)> + Send + use<> {
        let sock = self.res.socket();
        let started: windows_core::Result<OpFuture<RecvMsgPayload>> = (|| {
            let recvmsg = get_wsarecvmsg(sock)?;
            let payload = RecvMsgPayload {
                data: buf,
                control: vec![0u8; control_cap],
                source: unsafe { mem::zeroed() },
                wsabuf: unsafe { mem::zeroed() },
                msg: unsafe { mem::zeroed() },
            };
            Ok(unsafe {
                submit(&self.res, payload, move |ovl, p| {
                    let data_ptr = p.data.as_mut_ptr();
                    let data_len = p.data.len() as u32;
                    let ctrl_ptr = p.control.as_mut_ptr();
                    let ctrl_len = p.control.len() as u32;
                    let src_ptr = &mut p.source as *mut SOCKADDR_STORAGE as *mut SOCKADDR;
                    p.wsabuf = WSABUF {
                        len: data_len,
                        buf: windows_core::PSTR(data_ptr),
                    };
                    let buf_ptr = &mut p.wsabuf as *mut WSABUF;
                    p.msg = WSAMSG {
                        name: src_ptr,
                        namelen: size_of::<SOCKADDR_STORAGE>() as i32,
                        lpBuffers: buf_ptr,
                        dwBufferCount: 1,
                        Control: WSABUF {
                            len: ctrl_len,
                            buf: windows_core::PSTR(ctrl_ptr),
                        },
                        dwFlags: 0,
                    };
                    let r = recvmsg(sock, &mut p.msg, core::ptr::null_mut(), ovl, None);
                    classify_wsa_overlapped(r == 0)
                })
            })
        })();
        async move {
            match started {
                Ok(fut) => {
                    let (result, mut payload) = fut.await;
                    let out = match result {
                        Ok(n) => read_sockaddr(&payload.source).map(|source| {
                            let clen = payload.msg.Control.len as usize;
                            payload.control.truncate(clen);
                            ReceivedMsg {
                                bytes: n,
                                source,
                                control: std::mem::take(&mut payload.control),
                            }
                        }),
                        Err(e) => Err(e),
                    };
                    (out, payload.data)
                }
                Err(e) => (Err(e), Vec::new()),
            }
        }
    }

    /// Sends one datagram to `dest` with the given raw ancillary-data (control)
    /// bytes via `WSASendMsg` (empty `control` = an ordinary datagram). Resolves
    /// to the byte count and hands the data buffer back.
    pub fn send_msg(
        &self,
        buf: Vec<u8>,
        dest: SocketAddr,
        control: Vec<u8>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<u8>)> + Send + use<> {
        let sock = self.res.socket();
        let mut dest_storage = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
        let dest_len = write_sockaddr(&dest, &mut dest_storage);
        let payload = SendMsgPayload {
            data: buf,
            control,
            dest: dest_storage,
            wsabuf: unsafe { mem::zeroed() },
            msg: unsafe { mem::zeroed() },
        };
        let fut = unsafe {
            submit(&self.res, payload, move |ovl, p| {
                let data_ptr = p.data.as_mut_ptr();
                let data_len = p.data.len() as u32;
                let ctrl_len = p.control.len() as u32;
                // An empty control buffer must present a NULL pointer, not Vec's
                // dangling one (WSASendMsg rejects the latter with WSAEFAULT).
                let ctrl_ptr = if p.control.is_empty() {
                    core::ptr::null_mut()
                } else {
                    p.control.as_mut_ptr()
                };
                let dst_ptr = &mut p.dest as *mut SOCKADDR_STORAGE as *mut SOCKADDR;
                p.wsabuf = WSABUF {
                    len: data_len,
                    buf: windows_core::PSTR(data_ptr),
                };
                let buf_ptr = &mut p.wsabuf as *mut WSABUF;
                p.msg = WSAMSG {
                    name: dst_ptr,
                    namelen: dest_len,
                    lpBuffers: buf_ptr,
                    dwBufferCount: 1,
                    Control: WSABUF {
                        len: ctrl_len,
                        buf: windows_core::PSTR(ctrl_ptr),
                    },
                    dwFlags: 0,
                };
                let r = WSASendMsg(sock, &p.msg, 0, None, Some(ovl), None);
                classify_wsa_overlapped(r == 0)
            })
        };
        async move {
            let (result, payload) = fut.await;
            (result, payload.data)
        }
    }
}

/// `WSA_CMSG_ALIGN` — control-message headers and payloads are pointer-aligned.
const fn cmsg_align(n: usize) -> usize {
    n.next_multiple_of(size_of::<usize>())
}

/// Offset from a control message's start to its payload.
const fn cmsg_data_offset() -> usize {
    cmsg_align(size_of::<CMSGHDR>())
}

/// Builds a one-message control buffer for [`UdpSocket::send_msg`].
pub fn build_cmsg(level: i32, ty: i32, data: &[u8]) -> Vec<u8> {
    let data_off = cmsg_data_offset();
    // cmsg_len counts the unpadded header + payload; the buffer itself is
    // padded so a following message would start aligned.
    let cmsg_len = data_off + data.len();
    let mut buf = vec![0u8; cmsg_align(cmsg_len)];
    let hdr = CMSGHDR {
        cmsg_len,
        cmsg_level: level,
        cmsg_type: ty,
    };
    unsafe {
        core::ptr::write_unaligned(buf.as_mut_ptr() as *mut CMSGHDR, hdr);
    }
    buf[data_off..data_off + data.len()].copy_from_slice(data);
    buf
}

/// Walks a received control buffer, yielding `(level, type, payload)` per
/// message.
///
/// A datagram can carry several — `IP_PKTINFO` alongside `UDP_COALESCED_INFO`,
/// for instance — so a parser that only ever looks at the first one silently
/// misses the rest.
pub fn iter_cmsgs(control: &[u8]) -> impl Iterator<Item = (i32, i32, &[u8])> {
    let data_off = cmsg_data_offset();
    let mut pos = 0usize;
    core::iter::from_fn(move || {
        if pos + data_off > control.len() {
            return None;
        }
        // The buffer comes from the provider with no alignment guarantee of
        // its own beyond the Vec's, so read the header unaligned.
        let hdr: CMSGHDR =
            unsafe { core::ptr::read_unaligned(control[pos..].as_ptr() as *const CMSGHDR) };
        if hdr.cmsg_len < data_off || pos + hdr.cmsg_len > control.len() {
            return None;
        }
        let data = &control[pos + data_off..pos + hdr.cmsg_len];
        pos += cmsg_align(hdr.cmsg_len);
        Some((hdr.cmsg_level, hdr.cmsg_type, data))
    })
}

/// Control message setting the USO segment size for **one** `send_msg`,
/// without touching the socket-wide option.
///
/// ```ignore
/// let ctrl = cmsg_send_segment_size(1200);
/// sock.send_msg(big_buffer, dest, ctrl).await;
/// ```
pub fn cmsg_send_segment_size(segment_size: u16) -> Vec<u8> {
    build_cmsg(
        IPPROTO_UDP.0,
        UDP_SEND_MSG_SIZE,
        &(segment_size as u32).to_ne_bytes(),
    )
}

/// Parses the `UDP_COALESCED_INFO` control message, giving the segment size
/// the stack used when coalescing — i.e. how to cut the received buffer back
/// into datagrams.
///
/// Absent when only one datagram was delivered, which is the common case; a
/// caller should then treat the whole buffer as a single datagram.
///
/// Note that `UDP_COALESCED_INFO` (the control-message type) and
/// `UDP_RECV_MAX_COALESCED_SIZE` (the socket option) are both `3`, in
/// different namespaces — they are not interchangeable.
pub fn parse_udp_coalesced_info(control: &[u8]) -> Option<u32> {
    for (level, ty, data) in iter_cmsgs(control) {
        if level == IPPROTO_UDP.0 && ty == UDP_COALESCED_INFO as i32 && data.len() >= 4 {
            return Some(u32::from_ne_bytes([data[0], data[1], data[2], data[3]]));
        }
    }
    None
}

/// Parses the `IP_PKTINFO` control message out of a `recv_msg` control buffer,
/// returning the local address the datagram arrived on (IPv4).
pub fn parse_ip_pktinfo(control: &[u8]) -> Option<Ipv4Addr> {
    for (level, ty, data) in iter_cmsgs(control) {
        if level == IPPROTO_IP.0 && ty == IP_PKTINFO && data.len() >= size_of::<IN_PKTINFO>() {
            let pktinfo = unsafe { core::ptr::read_unaligned(data.as_ptr() as *const IN_PKTINFO) };
            let addr = unsafe { pktinfo.ipi_addr.S_un.S_addr };
            return Some(Ipv4Addr::from(u32::from_be(addr)));
        }
    }
    None
}
