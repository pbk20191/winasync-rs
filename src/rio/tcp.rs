//! A RIO TCP stream.
//!
//! RIO has no accept and no connect, so a connection is established the
//! ordinary way and only then gets a request queue. That split is the whole
//! story of this module:
//!
//! 1. create the socket with `WSA_FLAG_OVERLAPPED | WSA_FLAG_REGISTERED_IO`;
//! 2. connect or accept it with plain Winsock (or `ConnectEx`/`AcceptEx`);
//! 3. `RIOCreateRequestQueue` on it, before or after — measurement found no
//!    ordering constraint, and a send posted before connect simply completes
//!    with `WSAENOTCONN`.
//!
//! Note that `WSA_FLAG_REGISTERED_IO` is **inherited by `accept`**, like
//! `WSA_FLAG_OVERLAPPED`: a socket accepted from a RIO-flagged listener can
//! take a request queue without the flag being set again.

use std::sync::{Arc, Mutex};
use windows::Win32::Networking::WinSock::{
    bind, closesocket, listen, SOCKADDR, SOCKADDR_STORAGE, SOCKET, SOCK_STREAM, SOMAXCONN,
    WSASocketW, WSA_FLAG_OVERLAPPED, WSA_FLAG_REGISTERED_IO,
};

use super::buffer::RioSlice;
use super::cq::{CompleteFn, Request, RioCq};
use super::socket::RqConfig;
use super::table::rio_table;
use crate::common::oneshot;
use crate::common::winsock::{sock_check, wsa_init, wsa_last_err};
use crate::tp::io::net::{addr_of_socket, write_sockaddr};
use std::net::SocketAddr;

const RIO_INVALID_RQ: windows::Win32::Networking::WinSock::RIO_RQ =
    windows::Win32::Networking::WinSock::RIO_RQ(0);

fn new_rio_socket(addr: &SocketAddr) -> windows_core::Result<SOCKET> {
    wsa_init();
    let family = match addr {
        SocketAddr::V4(_) => windows::Win32::Networking::WinSock::AF_INET,
        SocketAddr::V6(_) => windows::Win32::Networking::WinSock::AF_INET6,
    };
    unsafe {
        WSASocketW(
            family.0 as i32,
            SOCK_STREAM.0,
            windows::Win32::Networking::WinSock::IPPROTO_TCP.0,
            None,
            0,
            WSA_FLAG_OVERLAPPED | WSA_FLAG_REGISTERED_IO,
        )
    }
}

/// A listening socket whose accepted connections can take RIO request queues.
///
/// The accept itself is ordinary Winsock — RIO provides no alternative — so
/// this is a thin wrapper that exists mainly to carry the RIO flag onto the
/// accepted sockets.
pub struct RioTcpListener {
    sock: SOCKET,
}

unsafe impl Send for RioTcpListener {}
unsafe impl Sync for RioTcpListener {}

impl RioTcpListener {
    pub fn bind(addr: SocketAddr) -> windows_core::Result<Self> {
        let sock = new_rio_socket(&addr)?;
        let setup = || -> windows_core::Result<()> {
            let mut storage = unsafe { core::mem::zeroed::<SOCKADDR_STORAGE>() };
            let len = write_sockaddr(&addr, &mut storage);
            sock_check(unsafe { bind(sock, &storage as *const _ as *const SOCKADDR, len) })?;
            sock_check(unsafe { listen(sock, SOMAXCONN as i32) })
        };
        match setup() {
            Ok(()) => Ok(Self { sock }),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }

    pub fn local_addr(&self) -> windows_core::Result<SocketAddr> {
        addr_of_socket(self.sock)
    }

    /// Accepts one connection and attaches a request queue to it.
    ///
    /// Blocking, because RIO offers no accept of its own and wrapping the
    /// overlapped `AcceptEx` here would mean pulling in the other engine.
    /// Call it from a pool thread (`Runtime::submit`) if that matters.
    pub fn accept_blocking(
        &self,
        cq: &Arc<RioCq>,
        config: RqConfig,
    ) -> windows_core::Result<RioTcpStream> {
        let accepted =
            unsafe { windows::Win32::Networking::WinSock::accept(self.sock, None, None) }?;
        // WSA_FLAG_REGISTERED_IO is inherited, so the accepted socket is
        // already RIO-capable.
        RioTcpStream::attach(accepted, cq, config)
    }
}

impl Drop for RioTcpListener {
    fn drop(&mut self) {
        unsafe { closesocket(self.sock) };
    }
}

/// A connected TCP socket driven by Registered I/O.
///
/// Same engine caveats as [`super::socket::RioUdpSocket`]: posts fail with
/// `WSAENOBUFS` when the request queue is full, dropping a future does not
/// cancel, and operations on one stream are serialized.
pub struct RioTcpStream {
    sock: SOCKET,
    rq: Mutex<windows::Win32::Networking::WinSock::RIO_RQ>,
    _cq: Arc<RioCq>,
}

unsafe impl Send for RioTcpStream {}
unsafe impl Sync for RioTcpStream {}

impl RioTcpStream {
    /// Connects (blocking) and attaches a request queue.
    pub fn connect_blocking(
        cq: &Arc<RioCq>,
        addr: SocketAddr,
        config: RqConfig,
    ) -> windows_core::Result<Self> {
        let sock = new_rio_socket(&addr)?;
        let mut storage = unsafe { core::mem::zeroed::<SOCKADDR_STORAGE>() };
        let len = write_sockaddr(&addr, &mut storage);
        if let Err(e) = sock_check(unsafe {
            windows::Win32::Networking::WinSock::connect(
                sock,
                &storage as *const _ as *const SOCKADDR,
                len,
            )
        }) {
            unsafe { closesocket(sock) };
            return Err(e);
        }
        Self::attach(sock, cq, config)
    }

    /// Gives an already-connected RIO-capable socket a request queue.
    ///
    /// Takes ownership: the stream closes the socket when dropped.
    pub fn attach(
        sock: SOCKET,
        cq: &Arc<RioCq>,
        config: RqConfig,
    ) -> windows_core::Result<Self> {
        let table = rio_table()?;
        let rq = unsafe {
            (table.raw().RIOCreateRequestQueue.unwrap())(
                sock,
                config.max_outstanding_recv,
                1,
                config.max_outstanding_send,
                1,
                cq.raw(),
                cq.raw(),
                core::ptr::null(),
            )
        };
        if rq == RIO_INVALID_RQ {
            let e = wsa_last_err();
            unsafe { closesocket(sock) };
            return Err(e);
        }
        Ok(Self {
            sock,
            rq: Mutex::new(rq),
            _cq: cq.clone(),
        })
    }

    pub fn local_addr(&self) -> windows_core::Result<SocketAddr> {
        addr_of_socket(self.sock)
    }

    /// Posts a receive; resolves to the byte count (0 = peer closed).
    pub fn recv(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.post(slice, false)
    }

    /// Posts a send of `slice`'s current length.
    pub fn send(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.post(slice, true)
    }

    fn post(
        &self,
        slice: RioSlice,
        is_send: bool,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        let table = rio_table()?;
        let (tx, rx) = oneshot::channel::<(windows_core::Result<usize>, Option<RioSlice>)>();
        let buf = slice.rio_buf();

        let complete: CompleteFn = Box::new(move |status, bytes, returned| {
            let result = if status == 0 {
                Ok(bytes as usize)
            } else {
                Err(windows_core::Error::from_hresult(
                    windows::Win32::Foundation::WIN32_ERROR(status as u32).to_hresult(),
                ))
            };
            let _ = tx.send((result, returned));
        });

        let ctx = Request {
            complete,
            slice: Some(slice),
        }
        .into_context();

        let rq = *self.rq.lock().unwrap();
        let ok = unsafe {
            if is_send {
                (table.raw().RIOSend.unwrap())(rq, &buf, 1, 0, ctx)
            } else {
                (table.raw().RIOReceive.unwrap())(rq, &buf, 1, 0, ctx)
            }
        };
        if !ok.as_bool() {
            let err = wsa_last_err();
            let _ = unsafe { Box::from_raw(ctx as *mut Request) };
            return Err(err);
        }

        Ok(async move {
            match rx.await {
                Ok((result, slice)) => (result, slice.expect("pump returns the slice")),
                Err(_) => unreachable!("the pump always completes a posted request"),
            }
        })
    }
}

impl Drop for RioTcpStream {
    fn drop(&mut self) {
        unsafe { closesocket(self.sock) };
    }
}

#[cfg(test)]
mod tests {
    use super::super::buffer::RioBufferPool;
    use super::*;
    use crate::common::block_on;
    use crate::tp::{timeout, Runtime};
    use std::net::{Ipv4Addr, SocketAddrV4};
    use std::time::Duration;

    #[test]
    fn rio_tcp_echo() {
        let rt = Runtime::new();
        let cq = RioCq::new(&rt, 256).expect("completion queue");
        let pool = RioBufferPool::new(1024, 8).expect("buffer pool");

        let listener = RioTcpListener::bind(SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::LOCALHOST,
            0,
        )))
        .expect("listen");
        let addr = listener.local_addr().unwrap();

        // The accept is blocking, so it runs on a pool thread while the main
        // thread connects.
        let (tx, rx) = std::sync::mpsc::channel();
        let cq2 = cq.clone();
        rt.submit(move || {
            let r = listener.accept_blocking(&cq2, RqConfig::default());
            let _ = tx.send(r.map(|s| Box::new(s)));
        });

        let client = RioTcpStream::connect_blocking(&cq, addr, RqConfig::default())
            .expect("connect");
        let server = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("accept")
            .expect("accepted stream");

        let result = block_on(timeout(&rt, Duration::from_secs(5), async {
            let srv_rx = server.recv(pool.acquire().unwrap()).expect("post recv");

            let mut out = pool.acquire().unwrap();
            out[..6].copy_from_slice(b"riotcp");
            out.set_len(6);
            let sent = client.send(out).expect("post send").await;
            assert_eq!(sent.0.unwrap(), 6);

            let (n, got) = srv_rx.await;
            let n = n.unwrap();
            assert_eq!(&got[..n], b"riotcp");

            // Echo back, proving the accepted socket's queue works both ways.
            let cli_rx = client.recv(pool.acquire().unwrap()).expect("client recv");
            let mut back = pool.acquire().unwrap();
            back[..n].copy_from_slice(&got[..n]);
            back.set_len(n);
            server.send(back).expect("server send").await.0.unwrap();

            let (n2, echoed) = cli_rx.await;
            assert_eq!(&echoed[..n2.unwrap()], b"riotcp");
        }));
        result.expect("RIO TCP echo timed out");
    }
}
