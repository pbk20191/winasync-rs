//! TCP over TP_IO: `TcpListener` (AcceptEx), `TcpStream` (ConnectEx /
//! WSARecv / WSASend), zero-copy sends (TransmitFile / TransmitPackets), and
//! socket-handle reuse (DisconnectEx + TF_REUSE_SOCKET).

use std::os::windows::io::AsHandle;
use super::*;
/// A listening TCP socket bound to a [`Runtime`]'s pool.
pub struct TcpListener {
    res: Arc<IoRes>,
    /// The address family the listen socket was created with; accepted sockets
    /// must be created with the same family for `AcceptEx`.
    family: ADDRESS_FAMILY,
    /// Extra `WSASocketW` flags carried onto every accepted socket, so a
    /// listener created for another engine's needs keeps producing sockets
    /// that engine can use. `AcceptEx` takes a socket the caller made, so
    /// unlike plain `accept()` nothing is inherited automatically.
    socket_flags: u32,
}

impl TcpListener {
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

    /// Binds and listens on `addr` (IPv4 or IPv6), creating sockets as
    /// `options` says.
    pub fn bind(
        rt: &Runtime,
        addr: SocketAddr,
        options: &SocketOptions,
    ) -> windows_core::Result<TcpListener> {
        let extra_flags = options.for_tcp()?.wsa_flags;
        let family = family_of(&addr);
        let sock = new_socket_with(family, SOCK_STREAM, IPPROTO_TCP, extra_flags)?;
        // Closed on any early return from here on.
        let owned = unsafe { crate::common::overlapped::IoHandle::from_socket(sock) };
        let mut storage = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
        let len = write_sockaddr(&addr, &mut storage);
        sock_check(unsafe { bind(sock, &storage as *const _ as *const SOCKADDR, len) })?;
        sock_check(unsafe { listen(sock, SOMAXCONN as i32) })?;
        let res = crate::tp::io::register_on(&rt.inner, owned)?;
        Ok(TcpListener {
            res,
            family,
            socket_flags: extra_flags,
        })
    }

    /// The locally bound address (useful after binding port 0).
    pub fn local_addr(&self) -> windows_core::Result<SocketAddr> {
        addr_of_socket(self.res.socket())
    }

    /// Accepts one connection.
    pub async fn accept(&self) -> windows_core::Result<TcpStream> {
        let accept_sock =
            new_socket_with(self.family, SOCK_STREAM, IPPROTO_TCP, self.socket_flags)?;
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
        // Closed on any early return from here on.
        let owned = unsafe { crate::common::overlapped::IoHandle::from_socket(sock) };
        // Until SO_UPDATE_ACCEPT_CONTEXT (value = the listen socket) the
        // accepted socket is half-configured (shutdown/getsockname/
        // getpeername fail).
        let listen_val = listen_sock.0.to_ne_bytes();
        sock_check(unsafe { setsockopt(sock, SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, Some(&listen_val)) })?;
        let stream = TcpStream::from_socket(self.res.runtime_inner(), owned)?;
        Ok(stream)
    }

    /// Accepts one connection **and** its first segment in a single op, saving
    /// the extra `WSARecv` round trip for protocols where the client speaks
    /// first (HTTP, TLS ClientHello, a length-prefixed request header).
    ///
    /// Resolves to the stream plus however many bytes arrived — never more than
    /// `recv_len`, often fewer, since one TCP segment is all the kernel waits
    /// for. Zero bytes are possible when the first segment is empty.
    ///
    /// Caveat: with a non-zero receive length AcceptEx does **not** complete on
    /// the TCP handshake alone — it waits for data. A client that connects and
    /// stays silent holds the accept (and its accept socket) indefinitely, so a
    /// server exposed to untrusted peers should pair this with
    /// `SO_CONNECT_TIME` reaping or a timeout. Use [`accept`](Self::accept) when
    /// the server speaks first.
    pub async fn accept_with_data(
        &self,
        recv_len: usize,
    ) -> windows_core::Result<(TcpStream, Vec<u8>)> {
        let accept_sock =
            new_socket_with(self.family, SOCK_STREAM, IPPROTO_TCP, self.socket_flags)?;
        let listen_sock = self.res.socket();
        let fut = unsafe {
            submit(
                &self.res,
                AcceptDataPayload {
                    accept_sock,
                    buf: vec![0u8; recv_len + 2 * ADDR_SLOT],
                    recv_len,
                },
                |ovl, p| {
                    let mut bytes = 0u32;
                    let ok = AcceptEx(
                        listen_sock,
                        p.accept_sock,
                        p.buf.as_mut_ptr() as *mut c_void,
                        p.recv_len as u32,
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
        // The completion byte count covers only the received data, not the
        // address slots (those are filled regardless).
        let received = result?;
        let sock = mem::replace(&mut payload.accept_sock, INVALID_SOCKET);
        let mut data = mem::take(&mut payload.buf);
        data.truncate(received.min(payload.recv_len));
        // Closed on any early return from here on.
        let owned = unsafe { crate::common::overlapped::IoHandle::from_socket(sock) };
        let listen_val = listen_sock.0.to_ne_bytes();
        sock_check(unsafe { setsockopt(sock, SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, Some(&listen_val)) })?;
        let stream = TcpStream::from_socket(self.res.runtime_inner(), owned)?;
        Ok((stream, data))
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


/// A connected TCP socket bound to a [`Runtime`]'s pool.
pub struct TcpStream {
    res: Arc<IoRes>,
}

impl TcpStream {
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

    fn from_socket(rt: &Arc<RuntimeInner>, sock: crate::common::overlapped::IoHandle) -> windows_core::Result<TcpStream> {
        let res = crate::tp::io::register_on(rt, sock)?;
        Ok(TcpStream { res })
    }

    /// Connects to `addr` (IPv4 or IPv6), creating the socket as `options`
    /// says.
    pub async fn connect(
        rt: &Runtime,
        addr: SocketAddr,
        options: &SocketOptions,
    ) -> windows_core::Result<TcpStream> {
        let extra_flags = options.for_tcp()?.wsa_flags;
        let family = family_of(&addr);
        let sock = new_socket_with(family, SOCK_STREAM, IPPROTO_TCP, extra_flags)?;
        // Closed on any early return from here on.
        let owned = unsafe { crate::common::overlapped::IoHandle::from_socket(sock) };
        // ConnectEx fails with WSAEINVAL on an unbound socket.
        let mut local = unsafe { mem::zeroed::<SOCKADDR_STORAGE>() };
        let local_len = write_sockaddr(&wildcard(family), &mut local);
        sock_check(unsafe { bind(sock, &local as *const _ as *const SOCKADDR, local_len) })?;
        let connect_ex = get_connectex(sock)?;
        let stream = TcpStream::from_socket(&rt.inner, owned)?;

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

    /// [`read`](Self::read), with a buffer that need not be `'static`:
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
    pub unsafe fn read_unchecked<B: IoBufMut, E: AsHandle + Send>(
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

    /// Sends `buf`; resolves to the byte count and hands the buffer back.
    pub fn write<B: IoBuf + 'static>(
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

    /// [`write`](Self::write), with a buffer that need not be `'static`:
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
    pub unsafe fn write_unchecked<B: IoBuf, E: AsHandle + Send>(
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

    /// Like [`read`](Self::read), but leaves the data queued (`MSG_PEEK`), so a
    /// subsequent read returns the same bytes. Useful for inspecting a protocol
    /// header before deciding how much to consume.
    pub fn peek<B: IoBufMut + 'static>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(
                &self.res,
                RecvPayload {
                    buf,
                    flags: MSG_PEEK.0 as u32,
                },
                move |ovl, p| {
                    let wsabuf = WSABUF {
                        len: IoBuf::bytes_total(&p.buf) as u32,
                        buf: windows_core::PSTR(p.buf.stable_mut_ptr()),
                    };
                    let r = WSARecv(sock, &[wsabuf], None, &mut p.flags, Some(ovl), None);
                    classify_wsa_overlapped(r == 0)
                },
            )
        };
        async move {
            let (result, payload) = fut.await;
            {
                let n = result.as_ref().ok().copied();
                (result, crate::common::buffer::mark_read(n, payload.buf))
            }
        }
    }

    /// Scatter-reads into `bufs` with a single vectored `WSARecv` — the socket
    /// counterpart to `File::read_scatter`. Resolves to the total byte count
    /// (0 = peer closed) and hands the buffers back; they are filled in order.
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

    /// Gather-writes `bufs` with a single vectored `WSASend` — the socket
    /// counterpart to `File::write_gather`. Resolves to the total byte count and
    /// hands the buffers back; they are sent in order as one contiguous stream.
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

    /// Skips the completion packet whenever an op on this socket completes
    /// synchronously — the common case for a read whose data is already in the
    /// socket buffer, or a write that fits in the send buffer. The op is then
    /// finished on the issuing thread, with no pool hand-off and no context
    /// switch, which is where most of the per-op cost of a chatty connection
    /// sits.
    ///
    /// Enable it *before* issuing ops. The mode belongs to the socket handle,
    /// so a stream recycled through `into_reusable` / `accept_into` keeps it.
    ///
    /// Semantics do not change — an op still resolves through its future, only
    /// sooner and possibly before the await yields.
    pub fn set_skip_completion_on_success(&self) -> windows_core::Result<()> {
        self.res.set_skip_completion_on_success()
    }

    /// Half-closes the send direction; the peer's next read returns 0.
    pub fn shutdown_send(&self) -> windows_core::Result<()> {
        sock_check(unsafe { shutdown(self.res.socket(), SD_SEND) })
    }

    /// A second handle onto the same socket (shared via the refcounted
    /// `IoRes`); the socket closes only once all handles drop. Used by
    /// `TcpStreamIo::split` to drive the read and write halves independently —
    /// concurrent WSARecv (read) and WSASend (write) on one socket is supported.
    pub(in crate::tp::io) fn share(&self) -> TcpStream {
        TcpStream {
            res: self.res.clone(),
        }
    }

    /// The underlying socket, for another engine in this crate to attach to.
    ///
    /// Borrowed, never surrendered: this stream keeps ownership and closes it
    /// on drop, so the borrower must not outlive the stream.
    pub(crate) fn raw_socket(&self) -> SOCKET {
        self.res.socket()
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
                                    // One pointer type for both directions;
                                    // a transmit never writes through it.
                                    pBuffer: buf.stable_ptr() as *mut c_void,
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
    /// Any owned buffer: the kernel only reads it.
    Mem(Box<dyn IoBuf>),
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

    /// Appends an in-memory buffer (sent inline): any owned buffer, so a
    /// shared `Arc<[u8]>` header can go out on many connections uncopied.
    pub fn bytes(mut self, buf: impl IoBuf + 'static) -> Self {
        self.elems.push(Elem::Mem(Box::new(buf)));
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
