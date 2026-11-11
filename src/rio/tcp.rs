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
    closesocket, connect, listen, IPPROTO_TCP, RIO_MSG_DEFER, RIO_MSG_DONT_NOTIFY, SOCKADDR, SOCKADDR_STORAGE,
    SOCKET, SOCK_STREAM,
};

use super::buffer::RioSlice;
use super::cq::RioCq;
use super::rq::{bind_socket, new_rio_socket, RequestQueue, RqConfig};
use crate::common::winsock::sock_check;
use crate::common::sockaddr::{addr_of_socket, write_sockaddr};
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
            sock_check(unsafe { listen(sock, crate::common::winsock::LISTEN_BACKLOG) })
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
    /// Blocking: RIO has no accept, and with `rio` alone there is no
    /// overlapped engine here to borrow one from. Enable `tp` as well and use
    /// `RioTcpAcceptor::accept`, which awaits an `AcceptEx` instead.
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
    /// Set when the socket came from the overlapped engine's handshake. That
    /// stream owns the socket and its TP_IO registration, so this one must
    /// not close it — exactly one owner, whichever path created it.
    #[cfg(feature = "tp")]
    owner: Option<crate::tp::io::net::TcpStream>,
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
            Ok(rq) => Ok(Self {
                sock,
                rq,
                #[cfg(feature = "tp")]
                owner: None,
            }),
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

    /// Posts a send whose completion raises no notification
    /// (`RIO_MSG_DONT_NOTIFY`); see
    /// [`RioUdpSocket::recv_without_notify`](super::socket::RioUdpSocket::recv_without_notify).
    pub fn send_without_notify(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.rq.post(slice, true, RIO_MSG_DONT_NOTIFY)
    }

    /// Receive counterpart of
    /// [`send_without_notify`](Self::send_without_notify).
    pub fn recv_without_notify(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.rq.post(slice, false, RIO_MSG_DONT_NOTIFY)
    }

    /// Resizes this stream's outstanding-operation budget.
    pub fn resize_queue(&self, max_recv: u32, max_send: u32) -> windows_core::Result<()> {
        self.rq.resize(max_recv, max_send)
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
        // A stream built on a `tp` handshake does not own its socket; the
        // overlapped stream it kept does, and closes it on its own drop.
        #[cfg(feature = "tp")]
        if self.owner.is_some() {
            return;
        }
        unsafe { closesocket(self.sock) };
    }
}

#[cfg(test)]
#[path = "tcp_tests.rs"]
mod tests;

/// Composition with the overlapped engine.
///
/// RIO has no accept and no connect, so with `tp` also enabled the handshake
/// is simply done by *that* engine and the connected socket handed over: one
/// socket, two engines, each doing the part it has. The socket is created
/// with `WSA_FLAG_REGISTERED_IO` through `tp`'s flag-carrying constructors,
/// which is why those exist — `tp` never learns what the flag means.
///
/// The `tp` stream is kept alive for the RIO stream's whole life rather than
/// being unwrapped: it owns the socket and its TP_IO registration, so keeping
/// it means exactly one owner closes the socket, and the overlapped side
/// stays usable alongside RIO if a caller wants both.
#[cfg(feature = "tp")]
mod compose {
    use super::*;
    use crate::tp::io::net::{SocketOptions, TcpListener as TpListener, TcpStream as TpStream};
    use crate::tp::Runtime;
    use windows::Win32::Networking::WinSock::WSA_FLAG_REGISTERED_IO;

    /// A listener whose accepted sockets can take RIO request queues, with an
    /// `async` accept driven by `AcceptEx`.
    pub struct RioTcpAcceptor {
        inner: TpListener,
    }

    impl RioTcpAcceptor {
        /// Binds, creating sockets RIO can use.
        pub fn bind(rt: &Runtime, addr: SocketAddr) -> windows_core::Result<Self> {
            Ok(Self {
                inner: TpListener::bind(
                    rt,
                    addr,
                    &SocketOptions::new().wsa_flags(WSA_FLAG_REGISTERED_IO),
                )?,
            })
        }

        pub fn local_addr(&self) -> windows_core::Result<SocketAddr> {
            self.inner.local_addr()
        }

        /// Awaits one connection and attaches a RIO request queue to it.
        pub async fn accept(
            &self,
            cq: &Arc<RioCq>,
            config: RqConfig,
        ) -> windows_core::Result<RioTcpStream> {
            let stream = self.inner.accept().await?;
            RioTcpStream::from_tp(stream, cq, config)
        }

        /// Awaits a connection *and* its first segment, then attaches a queue
        /// — `AcceptEx` with a receive length, so a client that speaks first
        /// costs one op instead of two.
        pub async fn accept_with_data(
            &self,
            cq: &Arc<RioCq>,
            config: RqConfig,
            recv_len: usize,
        ) -> windows_core::Result<(RioTcpStream, Vec<u8>)> {
            let (stream, data) = self.inner.accept_with_data(recv_len).await?;
            Ok((RioTcpStream::from_tp(stream, cq, config)?, data))
        }
    }

    impl RioTcpStream {
        /// Connects with `ConnectEx` and attaches a RIO request queue.
        pub async fn connect(
            rt: &Runtime,
            cq: &Arc<RioCq>,
            addr: SocketAddr,
            config: RqConfig,
        ) -> windows_core::Result<Self> {
            let stream =
                TpStream::connect(
                    rt,
                    addr,
                    &SocketOptions::new().wsa_flags(WSA_FLAG_REGISTERED_IO),
                ).await?;
            Self::from_tp(stream, cq, config)
        }

        /// Adopts a connected `tp` stream, adding a RIO request queue.
        ///
        /// The socket must have been created with `WSA_FLAG_REGISTERED_IO`,
        /// which the constructors above arrange; without it
        /// `RIOCreateRequestQueue` fails with `WSAEOPNOTSUPP`. The stream is
        /// kept, not consumed, so it keeps owning the socket.
        pub fn from_tp(
            stream: TpStream,
            cq: &Arc<RioCq>,
            config: RqConfig,
        ) -> windows_core::Result<Self> {
            let rq = RequestQueue::attach(stream.raw_socket(), cq, config)?;
            Ok(Self {
                sock: stream.raw_socket(),
                owner: Some(stream),
                rq,
            })
        }
    }
}

#[cfg(feature = "tp")]
#[allow(unused_imports)]
pub use compose::RioTcpAcceptor;
