//! Async TCP over TP_IO: `TcpListener` (AcceptEx) and `TcpStream`
//! (ConnectEx / WSARecv / WSASend).

use super::{submit, IoRes, Issue};
use crate::tp::{Runtime, RuntimeInner};
use core::ffi::c_void;
use std::mem;
use std::net::SocketAddrV4;
use std::sync::{Arc, Once, OnceLock};
use windows::Win32::Foundation::{HANDLE, WIN32_ERROR};
use windows::Win32::Networking::WinSock::{
    bind, closesocket, getsockname, listen, setsockopt, shutdown, AcceptEx, WSAGetLastError,
    WSAIoctl, WSARecv, WSASend, WSASocketW, WSAStartup, AF_INET, IN_ADDR, IN_ADDR_0, INVALID_SOCKET,
    IPPROTO_TCP, SD_SEND, SIO_GET_EXTENSION_FUNCTION_POINTER, SOCKADDR, SOCKADDR_IN,
    SOCKADDR_STORAGE, SOCKET, SOCK_STREAM, SOL_SOCKET, SOMAXCONN, SO_UPDATE_ACCEPT_CONTEXT,
    SO_UPDATE_CONNECT_CONTEXT, WSABUF, WSADATA, WSAID_CONNECTEX, WSA_FLAG_OVERLAPPED,
    WSA_IO_PENDING,
};

/// Process-wide Winsock initialization; never torn down (`WSACleanup` is
/// deliberately not called).
pub fn wsa_init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        let mut data = WSADATA::default();
        let r = WSAStartup(0x0202, &mut data);
        assert_eq!(r, 0, "WSAStartup failed: {r}");
    });
}

fn wsa_last_hr() -> windows_core::HRESULT {
    WIN32_ERROR(unsafe { WSAGetLastError() }.0 as u32).to_hresult()
}

fn wsa_last_err() -> windows_core::Error {
    windows_core::Error::from_hresult(wsa_last_hr())
}

fn sock_check(r: i32) -> windows_core::Result<()> {
    if r == 0 {
        Ok(())
    } else {
        Err(wsa_last_err())
    }
}

fn new_socket() -> windows_core::Result<SOCKET> {
    wsa_init();
    unsafe {
        WSASocketW(
            AF_INET.0 as i32,
            SOCK_STREAM.0,
            IPPROTO_TCP.0,
            None,
            0,
            WSA_FLAG_OVERLAPPED,
        )
    }
}

fn sockaddr_v4(addr: SocketAddrV4) -> SOCKADDR_IN {
    SOCKADDR_IN {
        sin_family: AF_INET,
        sin_port: addr.port().to_be(),
        sin_addr: IN_ADDR {
            S_un: IN_ADDR_0 {
                S_addr: u32::from(*addr.ip()).to_be(),
            },
        },
        sin_zero: [0; 8],
    }
}

fn addr_of_socket(sock: SOCKET) -> windows_core::Result<SocketAddrV4> {
    let mut sa = SOCKADDR_IN::default();
    let mut len = size_of::<SOCKADDR_IN>() as i32;
    sock_check(unsafe { getsockname(sock, &mut sa as *mut _ as *mut SOCKADDR, &mut len) })?;
    Ok(SocketAddrV4::new(
        u32::from_be(unsafe { sa.sin_addr.S_un.S_addr }).into(),
        u16::from_be(sa.sin_port),
    ))
}

type ConnectExFn = unsafe extern "system" fn(
    s: SOCKET,
    name: *const SOCKADDR,
    namelen: i32,
    lpsendbuffer: *const c_void,
    dwsenddatalength: u32,
    lpdwbytessent: *mut u32,
    lpoverlapped: *mut windows::Win32::System::IO::OVERLAPPED,
) -> windows_core::BOOL;

/// `ConnectEx` is not a direct export: fetch its provider function pointer via
/// `WSAIoctl(SIO_GET_EXTENSION_FUNCTION_POINTER)` once and cache it (the base
/// TCP provider's pointer is process-stable).
fn get_connectex(sock: SOCKET) -> windows_core::Result<ConnectExFn> {
    static PTR: OnceLock<usize> = OnceLock::new();
    if let Some(&p) = PTR.get() {
        return Ok(unsafe { mem::transmute::<usize, ConnectExFn>(p) });
    }
    let mut fptr: Option<ConnectExFn> = None;
    let mut bytes = 0u32;
    let r = unsafe {
        WSAIoctl(
            sock,
            SIO_GET_EXTENSION_FUNCTION_POINTER,
            Some(&WSAID_CONNECTEX as *const _ as *const c_void),
            size_of::<windows_core::GUID>() as u32,
            Some(&mut fptr as *mut _ as *mut c_void),
            size_of::<Option<ConnectExFn>>() as u32,
            &mut bytes,
            None,
            None,
        )
    };
    if r != 0 {
        return Err(wsa_last_err());
    }
    let f = fptr.expect("SIO_GET_EXTENSION_FUNCTION_POINTER returned null ConnectEx");
    let _ = PTR.set(f as usize);
    Ok(f)
}

fn classify_wsa_overlapped(succeeded: bool) -> Issue {
    if succeeded {
        // Synchronous success still queues a completion (we never set
        // FILE_SKIP_COMPLETION_PORT_ON_SUCCESS).
        Issue::Pending
    } else if unsafe { WSAGetLastError() } == WSA_IO_PENDING {
        Issue::Pending
    } else {
        Issue::Failed(wsa_last_hr())
    }
}

/// AcceptEx address slot: sockaddr size + 16 bytes, per the API contract.
const ADDR_SLOT: usize = size_of::<SOCKADDR_STORAGE>() + 16;

/// Owns the pre-created accept socket and address buffer while AcceptEx is in
/// flight. If the op is cancelled the completion callback frees the block and
/// this Drop closes the socket.
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

// SOCKET is a plain kernel handle value.
unsafe impl Send for AcceptPayload {}

/// A listening TCP socket bound to a [`Runtime`]'s pool.
pub struct TcpListener {
    res: Arc<IoRes>,
}

impl TcpListener {
    pub fn bind(rt: &Runtime, addr: SocketAddrV4) -> windows_core::Result<TcpListener> {
        let sock = new_socket()?;
        let register = || -> windows_core::Result<Arc<IoRes>> {
            let sa = sockaddr_v4(addr);
            sock_check(unsafe {
                bind(
                    sock,
                    &sa as *const _ as *const SOCKADDR,
                    size_of::<SOCKADDR_IN>() as i32,
                )
            })?;
            sock_check(unsafe { listen(sock, SOMAXCONN as i32) })?;
            IoRes::register(&rt.inner, HANDLE(sock.0 as *mut c_void), true)
        };
        match register() {
            Ok(res) => Ok(TcpListener { res }),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }

    /// The locally bound address (useful after binding port 0).
    pub fn local_addr(&self) -> windows_core::Result<SocketAddrV4> {
        addr_of_socket(self.res.socket())
    }

    /// Accepts one connection.
    pub async fn accept(&self) -> windows_core::Result<TcpStream> {
        let accept_sock = new_socket()?;
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
        let finish = || -> windows_core::Result<TcpStream> {
            // Until SO_UPDATE_ACCEPT_CONTEXT (value = the listen socket) the
            // accepted socket is half-configured (shutdown/getsockname fail).
            let listen_val = listen_sock.0.to_ne_bytes();
            sock_check(unsafe {
                setsockopt(sock, SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, Some(&listen_val))
            })?;
            TcpStream::from_socket(&self.res.rt, sock)
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

/// Payload of a WSARecv: the buffer plus the flags word, which the provider
/// may write at completion time and therefore must live in the op block.
struct RecvPayload {
    buf: Vec<u8>,
    flags: u32,
}

/// A connected TCP socket bound to a [`Runtime`]'s pool.
pub struct TcpStream {
    res: Arc<IoRes>,
}

impl TcpStream {
    fn from_socket(rt: &Arc<RuntimeInner>, sock: SOCKET) -> windows_core::Result<TcpStream> {
        match IoRes::register(rt, HANDLE(sock.0 as *mut c_void), true) {
            Ok(res) => Ok(TcpStream { res }),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }

    /// Connects to `addr`.
    pub async fn connect(rt: &Runtime, addr: SocketAddrV4) -> windows_core::Result<TcpStream> {
        let sock = new_socket()?;
        let prepared = (|| -> windows_core::Result<(ConnectExFn, TcpStream)> {
            // ConnectEx fails with WSAEINVAL on an unbound socket.
            let local = sockaddr_v4(SocketAddrV4::new(std::net::Ipv4Addr::UNSPECIFIED, 0));
            sock_check(unsafe {
                bind(
                    sock,
                    &local as *const _ as *const SOCKADDR,
                    size_of::<SOCKADDR_IN>() as i32,
                )
            })?;
            let connect_ex = get_connectex(sock)?;
            Ok((connect_ex, TcpStream::from_socket(&rt.inner, sock)?))
        })();
        let (connect_ex, stream) = match prepared {
            Ok(v) => v,
            Err(e) => {
                unsafe { closesocket(sock) };
                return Err(e);
            }
        };

        let remote = sockaddr_v4(addr);
        let fut = unsafe {
            submit(&stream.res, remote, move |ovl, sa| {
                let mut sent = 0u32;
                let ok = connect_ex(
                    sock,
                    sa as *const _ as *const SOCKADDR,
                    size_of::<SOCKADDR_IN>() as i32,
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
    pub fn read(
        &self,
        buf: Vec<u8>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<u8>)> + Send {
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

    /// Sends `buf`; resolves to the byte count and hands the buffer back.
    pub fn write(
        &self,
        buf: Vec<u8>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<u8>)> + Send {
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

    /// Half-closes the send direction; the peer's next read returns 0.
    pub fn shutdown_send(&self) -> windows_core::Result<()> {
        sock_check(unsafe { shutdown(self.res.socket(), SD_SEND) })
    }

    pub fn local_addr(&self) -> windows_core::Result<SocketAddrV4> {
        addr_of_socket(self.res.socket())
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{block_on, timeout, Runtime};
    use super::*;
    use std::net::Ipv4Addr;
    use std::time::Duration;

    #[test]
    fn tcp_echo_self_connect() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener =
                TcpListener::bind(&rt, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
            let addr = listener.local_addr().unwrap();
            assert_ne!(addr.port(), 0);

            // Echo server: accept one connection, echo until the peer
            // half-closes, then half-close back.
            let server = rt.spawn(async move {
                let stream = listener.accept().await.unwrap();
                let mut total = 0usize;
                let mut buf = vec![0u8; 4096];
                loop {
                    let (n, b) = stream.read(buf).await;
                    let n = n.unwrap();
                    if n == 0 {
                        break;
                    }
                    total += n;
                    let (w, b2) = stream.write(b[..n].to_vec()).await;
                    assert_eq!(w.unwrap(), n);
                    buf = b2;
                    buf.resize(4096, 0);
                }
                stream.shutdown_send().unwrap();
                total
            });

            // Client: send a payload, read the echo back, then half-close.
            let client = TcpStream::connect(&rt, addr).await.unwrap();
            let payload: Vec<u8> = (0..100_000u32).map(|i| (i % 231) as u8).collect();
            let mut sent = 0;
            while sent < payload.len() {
                let chunk = payload[sent..(sent + 8192).min(payload.len())].to_vec();
                let (w, _) = client.write(chunk).await;
                sent += w.unwrap();
            }

            let mut echoed = Vec::with_capacity(payload.len());
            while echoed.len() < payload.len() {
                let (n, b) = client.read(vec![0u8; 8192]).await;
                let n = n.unwrap();
                assert_ne!(n, 0, "server closed early");
                echoed.extend_from_slice(&b[..n]);
            }
            assert_eq!(echoed, payload);

            client.shutdown_send().unwrap();
            // Server saw exactly the payload and closed its side.
            assert_eq!(server.await.unwrap(), payload.len());
            let (n, _) = client.read(vec![0u8; 16]).await;
            assert_eq!(n.unwrap(), 0);
        }));
        result.expect("echo test timed out");
    }

    #[test]
    fn cancel_storm_accept_and_read() {
        let rt = Runtime::new();
        block_on(timeout(&rt, Duration::from_secs(30), async {
            let listener =
                TcpListener::bind(&rt, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
            let addr = listener.local_addr().unwrap();

            // Repeatedly start accepts with no client and drop them mid-flight.
            for _ in 0..1000 {
                drop(listener.accept());
            }

            // A connected pair; reads never complete (peer sends nothing), so
            // dropping the future exercises CancelIoEx on a live op.
            let server_task = rt.spawn({
                let rt = rt.clone();
                async move {
                    let _ = rt; // keep alive
                    listener.accept().await.unwrap()
                }
            });
            let client = TcpStream::connect(&rt, addr).await.unwrap();
            let server = server_task.await.unwrap();
            for _ in 0..1000 {
                drop(client.read(vec![0u8; 1024]));
                drop(server.read(vec![0u8; 1024]));
            }
        }))
        .expect("cancel storm timed out");
        drop(rt);
        #[cfg(debug_assertions)]
        {
            for _ in 0..500 {
                if super::super::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                super::super::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "op blocks leaked"
            );
        }
    }
}
