//! UDP over TP_IO: `UdpSocket` (WSARecvFrom / WSASendTo, plus connected-mode
//! WSARecv / WSASend) and control messages (WSARecvMsg / WSASendMsg, IP_PKTINFO).

use super::*;

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

/// Vectored counterpart of [`RecvFromPayload`]: the shared buffer/`WSABUF` pair
/// plus the source-address slot the provider fills.
struct VectoredFromPayload {
    v: VectoredPayload,
    from: SOCKADDR_STORAGE,
    from_len: i32,
}
unsafe impl Send for VectoredFromPayload {}

/// Vectored counterpart of [`SendToPayload`].
struct VectoredToPayload {
    v: VectoredPayload,
    to: SOCKADDR_STORAGE,
    to_len: i32,
}
unsafe impl Send for VectoredToPayload {}

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

    /// Scatter-receives one datagram into `bufs` with a vectored `WSARecvFrom`;
    /// resolves to the byte count plus the source address, and hands the buffers
    /// back. Truncation still fails with `WSAEMSGSIZE`.
    pub fn recv_from_vectored(
        &self,
        bufs: Vec<Vec<u8>>,
    ) -> impl Future<Output = (windows_core::Result<(usize, SocketAddr)>, Vec<Vec<u8>>)> + Send + use<>
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
                    p.v.fill();
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
            (out, payload.v.bufs)
        }
    }

    /// Gather-sends `bufs` as **one** datagram to `addr` with a vectored
    /// `WSASendTo`; resolves to the byte count and hands the buffers back.
    pub fn send_to_vectored(
        &self,
        bufs: Vec<Vec<u8>>,
        addr: SocketAddr,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<Vec<u8>>)> + Send + use<> {
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
                    p.v.fill();
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
    static PTR: OnceLock<usize> = OnceLock::new();
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

/// Parses the first `IP_PKTINFO` control message out of a `recv_msg` control
/// buffer, returning the local address the datagram arrived on (IPv4).
pub fn parse_ip_pktinfo(control: &[u8]) -> Option<Ipv4Addr> {
    let hdr_size = size_of::<CMSGHDR>();
    // WSA_CMSG_ALIGN to pointer size; data follows the aligned header.
    let data_off = hdr_size.next_multiple_of(size_of::<usize>());
    if control.len() < data_off + size_of::<IN_PKTINFO>() {
        return None;
    }
    let hdr = unsafe { &*(control.as_ptr() as *const CMSGHDR) };
    if hdr.cmsg_level != IPPROTO_IP.0 || hdr.cmsg_type != IP_PKTINFO {
        return None;
    }
    let pktinfo = unsafe { &*(control[data_off..].as_ptr() as *const IN_PKTINFO) };
    let addr = unsafe { pktinfo.ipi_addr.S_un.S_addr };
    Some(Ipv4Addr::from(u32::from_be(addr)))
}
