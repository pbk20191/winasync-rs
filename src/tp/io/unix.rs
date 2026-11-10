//! Async AF_UNIX (Unix domain) stream IO over TP_IO, raw `(Result, Vec<u8>)`
//! API — the `tp`-native counterpart to comio's WinRT-shaped AF_UNIX. Mirrors
//! `tp`'s `TcpListener`/`TcpStream` (AcceptEx / ConnectEx), with `SOCKADDR_UN`
//! addressing; shares the socket helpers in `super::net`.

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
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Networking::WinSock::{
    bind, closesocket, listen, setsockopt, AcceptEx, ADDRESS_FAMILY, AF_UNIX, INVALID_SOCKET,
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
    fn from_socket(rt: &Arc<RuntimeInner>, sock: SOCKET) -> windows_core::Result<UnixStream> {
        match IoRes::register(rt, HANDLE(sock.0 as *mut c_void), true) {
            Ok(res) => Ok(UnixStream { res }),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }

    /// Connects to the AF_UNIX socket at `path`.
    pub async fn connect(rt: &Runtime, path: &str) -> windows_core::Result<UnixStream> {
        let sock = unix_socket()?;
        let prepared = (|| -> windows_core::Result<UnixStream> {
            let (local, local_len) = sockaddr_un_autobind();
            sock_check(unsafe { bind(sock, &local as *const _ as *const SOCKADDR, local_len) })?;
            // Prime the ConnectEx pointer on this socket before registration.
            get_connectex(sock)?;
            UnixStream::from_socket(&rt.inner, sock)
        })();
        let stream = match prepared {
            Ok(s) => s,
            Err(e) => {
                unsafe { closesocket(sock) };
                return Err(e);
            }
        };
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
    pub fn read<B: IoBufMut>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, RecvPayload { buf, flags: 0 }, move |ovl, p| {
                let wsabuf = WSABUF {
                    len: IoBuf::len(&p.buf) as u32,
                    buf: windows_core::PSTR(p.buf.stable_mut_ptr()),
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

    /// Scatter-reads into `bufs` with one vectored `WSARecv`.
    pub fn read_vectored<B: IoBufMut>(
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
            (result, payload.bufs)
        }
    }

    /// Gather-writes `bufs` with one vectored `WSASend`.
    pub fn write_vectored<B: IoBuf>(
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
    pub fn write<B: IoBuf>(
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
}

/// A listening AF_UNIX socket.
pub struct UnixListener {
    res: Arc<IoRes>,
    rt: Runtime,
}

impl UnixListener {
    /// Binds and listens on the AF_UNIX path (the caller is responsible for
    /// removing any stale socket file first). Who may connect is decided by
    /// the socket file's DACL, which it inherits from the directory.
    pub fn bind(rt: &Runtime, path: &str) -> windows_core::Result<UnixListener> {
        Self::bind_inner(rt, path, None)
    }

    /// [`bind`](Self::bind), with `security`'s DACL written onto the socket
    /// file. It is applied between `bind` and `listen`: a connect before
    /// `listen` is refused anyway, so no client ever sees the inherited DACL.
    ///
    /// The DACL is the whole of the check, and it does not stop an elevated
    /// administrator: on a GitHub-hosted runner, which runs elevated, a
    /// SYSTEM-only DACL let the connect through. A named pipe's descriptor
    /// held in the same run.
    pub fn bind_with_security(
        rt: &Runtime,
        path: &str,
        security: &SecurityDescriptor,
    ) -> windows_core::Result<UnixListener> {
        Self::bind_inner(rt, path, Some(security))
    }

    fn bind_inner(
        rt: &Runtime,
        path: &str,
        security: Option<&SecurityDescriptor>,
    ) -> windows_core::Result<UnixListener> {
        let sock = unix_socket()?;
        let register = || -> windows_core::Result<Arc<IoRes>> {
            let (sa, len) = sockaddr_un(path)?;
            sock_check(unsafe { bind(sock, &sa as *const _ as *const SOCKADDR, len) })?;
            if let Some(security) = security {
                security.apply_dacl_to_file(path)?;
            }
            sock_check(unsafe { listen(sock, SOMAXCONN as i32) })?;
            IoRes::register(&rt.inner, HANDLE(sock.0 as *mut c_void), true)
        };
        match register() {
            Ok(res) => Ok(UnixListener {
                res,
                rt: rt.clone(),
            }),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
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
        let finish = || -> windows_core::Result<UnixStream> {
            let listen_val = listen_sock.0.to_ne_bytes();
            sock_check(unsafe {
                setsockopt(sock, SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, Some(&listen_val))
            })?;
            UnixStream::from_socket(&self.rt.inner, sock)
        };
        match finish() {
            Ok(stream) => Ok(stream),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{block_on, timeout, Runtime};
    use super::*;
    use std::time::Duration;

    #[test]
    fn unix_stream_echo() {
        let rt = Runtime::new();
        let path = std::env::temp_dir()
            .join(format!("tp_unix_{}.sock", std::process::id()))
            .to_string_lossy()
            .into_owned();
        let _ = std::fs::remove_file(&path);
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = UnixListener::bind(&rt, &path).unwrap();
            let p2 = path.clone();
            let server = rt.spawn(async move {
                let s = listener.accept().await.unwrap();
                let (n, b) = s.read(vec![0u8; 16]).await;
                let n = n.unwrap();
                s.write(b[..n].to_vec()).await.0.unwrap();
                let _ = p2;
            });
            let client = UnixStream::connect(&rt, &path).await.unwrap();
            client.write(b"unix!".to_vec()).await.0.unwrap();
            let (n, b) = client.read(vec![0u8; 16]).await;
            assert_eq!(&b[..n.unwrap()], b"unix!");
            server.await.unwrap();
        }));
        let _ = std::fs::remove_file(&path);
        result.expect("unix echo timed out");
    }

    fn temp_path(tag: &str) -> String {
        let path = std::env::temp_dir()
            .join(format!("tp_unix_{tag}_{}.sock", std::process::id()))
            .to_string_lossy()
            .into_owned();
        let _ = std::fs::remove_file(&path);
        path
    }

    /// Both ends report the other's process, and through it the account.
    #[test]
    fn each_end_identifies_the_other() {
        let rt = Runtime::new();
        let path = temp_path("peer");
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = UnixListener::bind(&rt, &path).unwrap();
            let server = rt.spawn(async move { listener.accept().await.unwrap() });
            let client = UnixStream::connect(&rt, &path).await.unwrap();
            let server = server.await.unwrap();
            let me = std::process::id();
            let my_sid = security::process_user_sid(me).unwrap();
            assert_eq!(server.peer_process_id().unwrap(), me);
            assert_eq!(server.peer_user_sid().unwrap(), my_sid);
            assert_eq!(client.peer_process_id().unwrap(), me);
        }));
        let _ = std::fs::remove_file(&path);
        result.expect("timed out");
    }

    /// The socket file's DACL is what `connect` is checked against, so a
    /// descriptor that does not grant this account keeps it out.
    #[test]
    fn a_descriptor_keeps_out_who_it_does_not_grant() {
        let rt = Runtime::new();
        let path = temp_path("deny");
        let system_only = SecurityDescriptor::from_sddl("D:P(A;;GA;;;SY)").unwrap();
        let listener = UnixListener::bind_with_security(&rt, &path, &system_only).unwrap();
        let result =
            block_on(timeout(&rt, Duration::from_secs(10), UnixStream::connect(&rt, &path)))
                .expect("timed out");
        drop(listener);
        let _ = std::fs::remove_file(&path);
        // An elevated administrator is not kept out by the socket file's DACL
        // (measured on a GitHub-hosted runner); report rather than assert.
        if security::process_is_elevated() {
            eprintln!("elevated: AF_UNIX connect through a SYSTEM-only DACL -> {:?}", result.as_ref().map(|_| ()));
            return;
        }
        let err = result.err().expect("connected through a DACL that does not grant this account");
        // WSAEACCES
        assert_eq!(err.code(), windows_core::HRESULT::from_win32(10013), "{err}");
    }
}
