//! Async AF_UNIX (Unix domain) stream IO over TP_IO, raw `(Result, Vec<u8>)`
//! API — the `tp`-native counterpart to comio's WinRT-shaped AF_UNIX. Mirrors
//! `tp`'s `TcpListener`/`TcpStream` (AcceptEx / ConnectEx), with `SOCKADDR_UN`
//! addressing; shares the socket helpers in `super::net`.

use std::os::windows::io::AsHandle;
use crate::common::winsock::ExtFnCache;
use super::net::{
    classify_wsa_overlapped, new_socket, sock_check, AcceptPayload, RecvPayload,
    VectoredPayload, ADDR_SLOT,
};
use super::{submit, IoRes};
use crate::common::buffer::{IoBuf, IoBufMut};
use crate::common::security::{self, SecurityDescriptor};
use crate::tp::{Runtime, RuntimeInner};
use crate::common::winsock::{connectex, ConnectExFn};
use core::ffi::c_void;
use std::mem;
use std::sync::Arc;
use windows::Win32::Networking::WinSock::{
    bind, listen, setsockopt, AcceptEx, ADDRESS_FAMILY, AF_UNIX, INVALID_SOCKET,
    IPPROTO, SOCKADDR, SOCKADDR_UN, SOCKET, SOCK_STREAM, SOL_SOCKET, SOMAXCONN,
    SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT, WSABUF, WSARecv, WSASend,
};
use windows_core::Error;

/// `ConnectEx` for the **AFUNIX** provider. The signature lives in
/// `common::winsock`; only this cache is local, because the pointer belongs
/// to the provider.
fn get_connectex(sock: SOCKET) -> windows_core::Result<ConnectExFn> {
    static PTR: ExtFnCache = ExtFnCache::new();
    connectex(sock, &PTR)
}

/// Builds a `SOCKADDR_UN` for `path` (NUL-terminated `sun_path`, <= 107 bytes).
fn sockaddr_un(path: &str) -> windows_core::Result<(SOCKADDR_UN, i32)> {
    let bytes = path.as_bytes();
    if bytes.len() > 107 {
        return Err(Error::from_hresult(
            windows::Win32::Foundation::ERROR_INVALID_PARAMETER.to_hresult(),
        ));
    }
    let mut sa = SOCKADDR_UN {
        sun_family: ADDRESS_FAMILY(AF_UNIX),
        sun_path: [0i8; 108],
    };
    for (i, &b) in bytes.iter().enumerate() {
        sa.sun_path[i] = b as i8;
    }
    Ok((sa, (size_of::<ADDRESS_FAMILY>() + bytes.len() + 1) as i32))
}

/// Zero-path "autobind" address — `ConnectEx` requires a bound socket.
fn sockaddr_un_autobind() -> (SOCKADDR_UN, i32) {
    (
        SOCKADDR_UN {
            sun_family: ADDRESS_FAMILY(AF_UNIX),
            sun_path: [0i8; 108],
        },
        size_of::<ADDRESS_FAMILY>() as i32,
    )
}

fn unix_socket() -> windows_core::Result<SOCKET> {
    new_socket(ADDRESS_FAMILY(AF_UNIX), SOCK_STREAM, IPPROTO(0))
}


/// A connected AF_UNIX stream socket.
pub struct UnixStream {
    res: Arc<IoRes>,
}

impl UnixStream {
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

    fn from_socket(rt: &Arc<RuntimeInner>, sock: crate::common::overlapped::IoHandle) -> windows_core::Result<UnixStream> {
        let res = crate::tp::io::register_on(rt, sock)?;
        Ok(UnixStream { res })
    }

    /// Connects to the AF_UNIX socket at `path`.
    pub async fn connect(rt: &Runtime, path: &str) -> windows_core::Result<UnixStream> {
        let sock = unix_socket()?;
        // Closed on any early return from here on.
        let owned = unsafe { crate::common::overlapped::IoHandle::from_socket(sock) };
        let (local, local_len) = sockaddr_un_autobind();
        sock_check(unsafe { bind(sock, &local as *const _ as *const SOCKADDR, local_len) })?;
        // Prime the ConnectEx pointer on this socket before registration.
        get_connectex(sock)?;
        let stream = UnixStream::from_socket(&rt.inner, owned)?;
        let connect_ex = get_connectex(sock)?;
        let (remote, remote_len) = sockaddr_un(path)?;
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

    /// The process id at the other end (`SIO_AF_UNIX_GETPEERPID`), from
    /// either end of the connection.
    pub fn peer_process_id(&self) -> windows_core::Result<u32> {
        security::unix_peer_pid(self.res.socket())
    }

    /// The account the peer process runs as, as a string SID.
    ///
    /// Windows reports only the peer's process id for AF_UNIX, so this is
    /// that process's token: it needs the right to open the process (usually
    /// only when both run as the same user), and names whoever holds the id
    /// at the time of the call.
    pub fn peer_user_sid(&self) -> windows_core::Result<String> {
        security::process_user_sid(self.peer_process_id()?)
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
        let fut = super::evented(&self.res, buf, event, move |mem, buf| unsafe {
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
        super::evented(&self.res, buf, event, move |mem, buf| unsafe {
            let wsabuf = WSABUF {
                len: IoBuf::len(buf) as u32,
                buf: windows_core::PSTR(buf.stable_ptr() as *mut u8),
            };
            let r = WSASend(SOCKET(sock), &[wsabuf], None, 0, Some(&mut mem.ovl), None);
            classify_wsa_overlapped(r == 0)
        })
    }
}

/// A listening AF_UNIX socket.
pub struct UnixListener {
    res: Arc<IoRes>,
    rt: Runtime,
}

impl UnixListener {
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

    /// Binds and listens on the AF_UNIX path (the caller is responsible for
    /// removing any stale socket file first).
    ///
    /// `security`, if given, has its DACL written onto the socket file
    /// between `bind` and `listen` -- a connect before `listen` is refused
    /// anyway, so no client ever sees the inherited DACL. Without it, who may
    /// connect is decided by the DACL the file inherits from its directory.
    ///
    /// Treat that DACL as defence in depth, not as the boundary. On the
    /// development machine it refused every connect it should have; on
    /// GitHub-hosted runners it was enforced in some jobs and not in others --
    /// same image, same token, same code -- and no cause was found. A named
    /// pipe's descriptor was enforced everywhere. Where the boundary must
    /// hold, use a named pipe, or check the peer's SID at accept.
    pub fn bind(
        rt: &Runtime,
        path: &str,
        security: Option<&SecurityDescriptor>,
    ) -> windows_core::Result<UnixListener> {
        let sock = unix_socket()?;
        // Closed on any early return from here on.
        let owned = unsafe { crate::common::overlapped::IoHandle::from_socket(sock) };
        let (sa, len) = sockaddr_un(path)?;
        sock_check(unsafe { bind(sock, &sa as *const _ as *const SOCKADDR, len) })?;
        if let Some(security) = security {
            security.apply_dacl_to_file(path)?;
        }
        sock_check(unsafe { listen(sock, SOMAXCONN as i32) })?;
        let res = crate::tp::io::register_on(&rt.inner, owned)?;
        Ok(UnixListener {
            res,
            rt: rt.clone(),
        })
    }

    /// Accepts one connection.
    pub async fn accept(&self) -> windows_core::Result<UnixStream> {
        let accept_sock = unix_socket()?;
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
        let stream = UnixStream::from_socket(&self.rt.inner, owned)?;
        Ok(stream)
    }
}

#[cfg(test)]
#[path = "unix_tests.rs"]
mod tests;
