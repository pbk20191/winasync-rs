//! Async AF_UNIX (Unix domain) stream IO over TP_IO, raw `(Result, Vec<u8>)`
//! API — the `tp`-native counterpart to comio's WinRT-shaped AF_UNIX. Mirrors
//! `tp`'s `TcpListener`/`TcpStream` (AcceptEx / ConnectEx), with `SOCKADDR_UN`
//! addressing; shares the socket helpers in `super::net`.

use super::net::{classify_wsa_overlapped, get_connectex, new_socket, sock_check, ADDR_SLOT};
use super::{submit, IoRes, Issue};
use crate::tp::{Runtime, RuntimeInner};
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

struct RecvPayload {
    buf: Vec<u8>,
    flags: u32,
}

struct AcceptPayload {
    accept_sock: SOCKET,
    addr_buf: Box<[u8; 2 * ADDR_SLOT]>,
}
impl Drop for AcceptPayload {
    fn drop(&mut self) {
        if self.accept_sock != INVALID_SOCKET {
            unsafe { closesocket(self.accept_sock) };
        }
    }
}
unsafe impl Send for AcceptPayload {}

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

    /// Receives into `buf`; resolves to the byte count (0 = peer closed).
    pub fn read(
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

    /// Sends `buf`; resolves to the byte count.
    pub fn write(
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
        async move { fut.await }
    }
}

/// A listening AF_UNIX socket.
pub struct UnixListener {
    res: Arc<IoRes>,
    rt: Runtime,
}

impl UnixListener {
    /// Binds and listens on the AF_UNIX path (the caller is responsible for
    /// removing any stale socket file first).
    pub fn bind(rt: &Runtime, path: &str) -> windows_core::Result<UnixListener> {
        let sock = unix_socket()?;
        let register = || -> windows_core::Result<Arc<IoRes>> {
            let (sa, len) = sockaddr_un(path)?;
            sock_check(unsafe { bind(sock, &sa as *const _ as *const SOCKADDR, len) })?;
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
}
