//! `WSARecvMsg` / `WSASendMsg`: datagrams with control messages.

use super::*;

/// `WSARecvMsg` for the TCP/IP provider. (`WSASendMsg` *is* a direct export,
/// so it needs no lookup.)
pub(super) fn get_wsarecvmsg(sock: SOCKET) -> windows_core::Result<WsaRecvMsgFn> {
    static PTR: ExtFnCache = ExtFnCache::new();
    unsafe { extension_fn(sock, &WSAID_WSARECVMSG, &PTR) }
}

/// Op-block payload for `recv_msg`: the data buffer, the control (ancillary)
/// buffer, the source address, and the `WSABUF`/`WSAMSG` the kernel reads
/// (self-referential pointers into this same payload, valid for the op).
pub(super) struct RecvMsgPayload {
    pub(super) data: Vec<u8>,
    pub(super) control: Vec<u8>,
    pub(super) source: SOCKADDR_STORAGE,
    pub(super) wsabuf: WSABUF,
    pub(super) msg: WSAMSG,
}
unsafe impl Send for RecvMsgPayload {}

/// Op-block payload for `send_msg` (dest address + control, same discipline).
pub(super) struct SendMsgPayload {
    pub(super) data: Vec<u8>,
    pub(super) control: Vec<u8>,
    pub(super) dest: SOCKADDR_STORAGE,
    pub(super) wsabuf: WSABUF,
    pub(super) msg: WSAMSG,
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

    pub(super) fn set_udp_opt(&self, level: i32, name: i32, value: &[u8]) -> windows_core::Result<()> {
        sock_check(unsafe { setsockopt(self.res.socket(), level, name, Some(value)) })
    }

    pub(super) fn mreq_v4(group: Ipv4Addr, interface: Ipv4Addr) -> IP_MREQ {
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

    pub(super) fn mreq_source_v4(group: Ipv4Addr, source: Ipv4Addr, interface: Ipv4Addr) -> IP_MREQ_SOURCE {
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

    pub(super) fn as_bytes<T>(v: &T) -> &[u8] {
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
    pub(super) fn get_udp_dword(&self, name: i32) -> windows_core::Result<u32> {
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
pub(super) const fn cmsg_align(n: usize) -> usize {
    n.next_multiple_of(size_of::<usize>())
}

/// Offset from a control message's start to its payload.
pub(super) const fn cmsg_data_offset() -> usize {
    cmsg_align(size_of::<CMSGHDR>())
}
