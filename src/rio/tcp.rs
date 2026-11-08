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

use std::sync::Arc;
use windows::Win32::Networking::WinSock::{
    closesocket, connect, listen, IPPROTO_TCP, RIO_MSG_DEFER, SOCKADDR, SOCKADDR_STORAGE,
    SOCKET, SOCK_STREAM, SOMAXCONN,
};

use super::buffer::RioSlice;
use super::cq::RioCq;
use super::rq::{bind_socket, new_rio_socket, RequestQueue, RqConfig};
use crate::common::winsock::sock_check;
use crate::tp::io::net::{addr_of_socket, write_sockaddr};
use std::net::SocketAddr;

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
        let sock = new_rio_socket(&addr, SOCK_STREAM.0, IPPROTO_TCP.0)?;
        let setup = || -> windows_core::Result<()> {
            bind_socket(sock, &addr)?;
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
    rq: RequestQueue,
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
        let sock = new_rio_socket(&addr, SOCK_STREAM.0, IPPROTO_TCP.0)?;
        let mut storage = unsafe { core::mem::zeroed::<SOCKADDR_STORAGE>() };
        let len = write_sockaddr(&addr, &mut storage);
        if let Err(e) =
            sock_check(unsafe { connect(sock, &storage as *const _ as *const SOCKADDR, len) })
        {
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
        match RequestQueue::attach(sock, cq, config) {
            Ok(rq) => Ok(Self { sock, rq }),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
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
        self.rq.post(slice, false, 0)
    }

    /// Posts a send of `slice`'s current length.
    pub fn send(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.rq.post(slice, true, 0)
    }

    /// Queues a send without dispatching it (`RIO_MSG_DEFER`), to be released
    /// by [`commit_sends`](Self::commit_sends).
    ///
    /// Worth more on TCP than anywhere else in this engine. The published TCP
    /// comparisons have RIO at or below plain IOCP by default — one
    /// game-server benchmark measured RIO *behind* IOCP until sends were
    /// batched and deferred, at which point it moved ahead by a few percent.
    /// Deferring is what amortises RIO's per-operation floor across a batch,
    /// and on a stream protocol there is usually something to batch.
    pub fn send_deferred(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.rq.post(slice, true, RIO_MSG_DEFER)
    }

    /// Dispatches everything queued by
    /// [`send_deferred`](Self::send_deferred), in one crossing
    /// (`RIO_MSG_COMMIT_ONLY`). Each deferred send still completes
    /// individually.
    pub fn commit_sends(&self) -> windows_core::Result<()> {
        self.rq.commit_sends()
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

    /// Several sends queued with RIO_MSG_DEFER and released by one commit.
    ///
    /// TCP has no message boundaries, so the receiver may see the batch as
    /// one read or several — the test accepts either and only insists the
    /// bytes arrive, in order, after the commit.
    #[test]
    fn rio_tcp_deferred_sends_commit_as_a_batch() {
        let rt = Runtime::new();
        let cq = RioCq::new(&rt, 256).expect("completion queue");
        let pool = RioBufferPool::new(512, 8).expect("buffer pool");

        let listener = RioTcpListener::bind(SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::LOCALHOST,
            0,
        )))
        .expect("listen");
        let addr = listener.local_addr().unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        let cq2 = cq.clone();
        rt.submit(move || {
            let r = listener.accept_blocking(&cq2, RqConfig::default());
            let _ = tx.send(r.map(Box::new));
        });

        let client =
            RioTcpStream::connect_blocking(&cq, addr, RqConfig::default()).expect("connect");
        let server = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("accept")
            .expect("accepted stream");

        let result = block_on(timeout(&rt, Duration::from_secs(5), async {
            let srv_rx = server.recv(pool.acquire().unwrap()).expect("post recv");

            let mut futures = Vec::new();
            for chunk in [b"aaa", b"bbb", b"ccc"] {
                let mut s = pool.acquire().unwrap();
                s[..3].copy_from_slice(chunk);
                s.set_len(3);
                futures.push(client.send_deferred(s).expect("defer"));
            }
            // Nothing has gone out yet; one crossing releases all three.
            client.commit_sends().expect("commit");

            for f in futures {
                assert_eq!(f.await.0.unwrap(), 3);
            }

            let (n, got) = srv_rx.await;
            let n = n.unwrap();
            assert!(n > 0 && n <= 9, "got {n} bytes");
            // Whatever the segmentation, the stream starts with the batch in
            // submission order.
            assert_eq!(&got[..n], &b"aaabbbccc"[..n]);
        }));
        result.expect("deferred TCP sends timed out");
    }

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
