//! A RIO UDP socket: a request queue over a [`RioCq`], with futures for send
//! and receive.
//!
//! # How this differs from `tp::io::net::UdpSocket`
//!
//! * **Buffers come from a [`super::buffer::RioBufferPool`]**, not from the
//!   caller's `Vec`.
//! * **The request queue is sized up front.** `max_outstanding_*` is a hard
//!   cap; exceeding it fails the call with `WSAENOBUFS` *synchronously*
//!   rather than queueing. That is the backpressure signal, not an error to
//!   paper over.
//! * **A dropped future does not cancel.** RIO has no per-operation
//!   cancellation — no `CancelIoEx` equivalent — so dropping a send or
//!   receive future merely stops you observing the result. The operation
//!   still runs and its buffer stays held until it completes. The only way to
//!   stop outstanding work is to drop the socket, and even then completions
//!   arrive afterwards (with `WSAECONNABORTED`).
//! * **One socket, one thread at a time.** Neither the request queue nor the
//!   completion queue is internally synchronized, so sends and receives on
//!   one socket are serialized here by a mutex.

use std::sync::Arc;
use windows::Win32::Networking::WinSock::{
    closesocket, RIO_MSG_DEFER, RIO_MSG_DONT_NOTIFY, SOCKADDR, SOCKADDR_INET, SOCKADDR_STORAGE, SOCKET, SOCK_DGRAM,
};

use super::buffer::RioSlice;
use crate::tp::io::net::read_sockaddr;
use super::cq::{CompleteFn, RioCq};
use super::rq::{
    bind_socket, new_rio_socket, resolve, status_to_result, RequestQueue, RqConfig,
    WSAENOBUFS_HRESULT,
};
use crate::common::winsock::sock_check;
use crate::common::oneshot;

use std::net::SocketAddr;

/// A UDP socket driven by Registered I/O.
pub struct RioUdpSocket {
    sock: SOCKET,
    /// Registered slots for the `RIO_BUF` addresses that `RIOReceiveEx` /
    /// `RIOSendEx` need. Sized to the request queue, so a post that the queue
    /// would accept always finds a slot.
    addrs: Arc<super::buffer::RioBufferPool>,
    /// Posts operations and keeps the completion queue — with its pump —
    /// alive for this socket's whole life, including the completions that
    /// arrive after close.
    rq: RequestQueue,
}

unsafe impl Send for RioUdpSocket {}
unsafe impl Sync for RioUdpSocket {}

impl RioUdpSocket {
    /// Binds a RIO UDP socket and attaches a request queue to `cq`.
    ///
    /// The socket is created with `WSA_FLAG_REGISTERED_IO`, which only the
    /// TCP/IP families accept — AF_UNIX and AF_HYPERV fail here with
    /// `WSAEINVAL`, at creation rather than at queue setup.
    pub fn bind(
        cq: &Arc<RioCq>,
        addr: SocketAddr,
        config: RqConfig,
    ) -> windows_core::Result<Self> {
        let sock = new_rio_socket(
            &addr,
            SOCK_DGRAM.0,
            windows::Win32::Networking::WinSock::IPPROTO_UDP.0,
        )?;

        let build = || -> windows_core::Result<RequestQueue> {
            bind_socket(sock, &addr)?;
            RequestQueue::attach(sock, cq, config)
        };

        let addrs = match super::buffer::RioBufferPool::new(
            size_of::<SOCKADDR_INET>(),
            config.max_outstanding_recv + config.max_outstanding_send,
        ) {
            Ok(p) => p,
            Err(e) => {
                unsafe { closesocket(sock) };
                return Err(e);
            }
        };

        match build() {
            Ok(rq) => Ok(Self { sock, addrs, rq }),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }

    /// The bound address (useful after binding port 0).
    pub fn local_addr(&self) -> windows_core::Result<SocketAddr> {
        local_addr_of(self.sock)
    }

    /// Posts a receive into `slice`.
    ///
    /// Resolves to the byte count and the slice. Fails synchronously with
    /// `WSAENOBUFS` when the request queue is already at
    /// `max_outstanding_recv` — back off and retry rather than treating it as
    /// fatal.
    ///
    /// Dropping the returned future does **not** cancel the receive; see the
    /// module note.
    pub fn recv(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.rq.post(slice, false, 0)
    }

    /// Posts a send of `slice`'s current length to the connected peer.
    ///
    /// Same queue-full and cancellation semantics as
    /// [`recv`](Self::recv).
    pub fn send(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.rq.post(slice, true, 0)
    }

    /// Connects the socket so [`send`](Self::send) has a destination. RIO has
    /// no connect of its own; this is the ordinary Winsock one.
    pub fn connect(&self, addr: SocketAddr) -> windows_core::Result<()> {
        let mut storage = unsafe { core::mem::zeroed::<SOCKADDR_STORAGE>() };
        let len = write_sockaddr(&addr, &mut storage);
        sock_check(unsafe {
            windows::Win32::Networking::WinSock::connect(
                self.sock,
                &storage as *const _ as *const SOCKADDR,
                len,
            )
        })
    }

    /// Receives from any peer, reporting the source address — the unconnected
    /// counterpart of [`recv`](Self::recv), over `RIOReceiveEx`.
    ///
    /// RIO takes the address as a `RIO_BUF` too, so it must live in a
    /// registered buffer: the socket keeps a small internal pool for that,
    /// sized to its request queue, and the slot is released with the
    /// completion.
    pub fn recv_from(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<
        impl Future<Output = (windows_core::Result<(usize, SocketAddr)>, RioSlice)> + use<>,
    > {
        self.post_ex(slice, None)
    }

    /// Sends `slice` to `dest` without connecting, over `RIOSendEx`.
    pub fn send_to(
        &self,
        slice: RioSlice,
        dest: SocketAddr,
    ) -> windows_core::Result<
        impl Future<Output = (windows_core::Result<(usize, SocketAddr)>, RioSlice)> + use<>,
    > {
        self.post_ex(slice, Some(dest))
    }

    /// Queues a send without executing it (`RIO_MSG_DEFER`), to be released
    /// later by [`commit_sends`](Self::commit_sends).
    ///
    /// Deferring lets several sends be handed to the provider and then
    /// dispatched with one crossing, which is the send-side half of what a
    /// completion queue does for results. Each deferred send still completes
    /// individually.
    pub fn send_deferred(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.rq.post(slice, true, RIO_MSG_DEFER)
    }

    /// Posts a receive whose completion will **not** raise a notification
    /// (`RIO_MSG_DONT_NOTIFY`).
    ///
    /// The result still lands in the completion queue; it just does not wake
    /// anyone. That is the building block for a hybrid loop: poll the queue
    /// while traffic is heavy and pay nothing per completion, and arm a
    /// notification only when it runs dry. Published measurements have that
    /// arrangement doing nearly all the work on a couple of threads where a
    /// notify-per-completion design kept every thread busy.
    ///
    /// On a pool-driven queue this means the completion may sit undispatched
    /// until some *other* operation's notification fires — so use it with
    /// [`RioCq::polling`](super::cq::RioCq::polling) or an IOCP loop that
    /// drains on its own schedule.
    pub fn recv_without_notify(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.rq.post(slice, false, RIO_MSG_DONT_NOTIFY)
    }

    /// Send counterpart of
    /// [`recv_without_notify`](Self::recv_without_notify).
    pub fn send_without_notify(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.rq.post(slice, true, RIO_MSG_DONT_NOTIFY)
    }

    /// Resizes this socket's outstanding-operation budget; see
    /// [`RequestQueue::resize`](super::rq::RequestQueue::resize).
    pub fn resize_queue(&self, max_recv: u32, max_send: u32) -> windows_core::Result<()> {
        self.rq.resize(max_recv, max_send)
    }

    /// Dispatches everything queued by
    /// [`send_deferred`](Self::send_deferred) (`RIO_MSG_COMMIT_ONLY`).
    ///
    /// Microsoft documents this as the one request-queue entry point that
    /// does *not* need to be serialized with other sends — it may run on
    /// another thread concurrently with a `RIOSend` on the same queue. This
    /// implementation still takes the socket's lock, because the lock also
    /// guards the queue handle itself and the cost is a non-issue next to the
    /// dispatch.
    pub fn commit_sends(&self) -> windows_core::Result<()> {
        self.rq.commit_sends()
    }

    fn post_ex(
        &self,
        slice: RioSlice,
        dest: Option<SocketAddr>,
    ) -> windows_core::Result<
        impl Future<Output = (windows_core::Result<(usize, SocketAddr)>, RioSlice)> + use<>,
    > {
        let (tx, rx) = oneshot::channel::<(
            windows_core::Result<(usize, SocketAddr)>,
            Option<RioSlice>,
        )>();

        // The address travels in its own registered slice, because RIO names
        // it with a RIO_BUF like any other buffer.
        let mut addr_slice = self
            .addrs
            .acquire()
            .ok_or_else(|| windows_core::Error::from_hresult(WSAENOBUFS_HRESULT))?;
        addr_slice.set_len(size_of::<SOCKADDR_INET>());
        if let Some(dest) = dest {
            let mut storage = unsafe { core::mem::zeroed::<SOCKADDR_STORAGE>() };
            write_sockaddr(&dest, &mut storage);
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    &storage as *const _ as *const u8,
                    size_of::<SOCKADDR_INET>(),
                )
            };
            addr_slice.copy_from_slice(bytes);
        } else {
            addr_slice.fill(0);
        }
        let addr_buf = addr_slice.rio_buf();
        let data_buf = slice.rio_buf();

        // The closure owns the address slice: by the time it runs, the kernel
        // has finished with the buffer, so reading the peer here is sound and
        // the slot returns to the pool on the way out.
        let complete: CompleteFn = Box::new(move |status, bytes, data| {
            let result = if status == 0 {
                let mut storage = unsafe { core::mem::zeroed::<SOCKADDR_STORAGE>() };
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        addr_slice.as_ptr(),
                        &mut storage as *mut _ as *mut u8,
                        size_of::<SOCKADDR_INET>(),
                    );
                }
                read_sockaddr(&storage).map(|a| (bytes as usize, a))
            } else {
                Err(status_to_result(status, bytes).unwrap_err())
            };
            drop(addr_slice);
            let _ = tx.send((result, data));
        });

        self.rq
            .post_ex(&data_buf, &addr_buf, dest.is_some(), slice, complete)?;

        Ok(async move { resolve(rx).await })
    }

}

impl Drop for RioUdpSocket {
    /// Closing is the only way to stop outstanding operations. Their
    /// completions still arrive — with `WSAECONNABORTED` — and the completion
    /// queue stays alive to receive them because this socket holds an `Arc`
    /// of it.
    fn drop(&mut self) {
        unsafe { closesocket(self.sock) };
    }
}

fn write_sockaddr(addr: &SocketAddr, storage: &mut SOCKADDR_STORAGE) -> i32 {
    crate::tp::io::net::write_sockaddr(addr, storage)
}

fn local_addr_of(sock: SOCKET) -> windows_core::Result<SocketAddr> {
    crate::tp::io::net::addr_of_socket(sock)
}

#[cfg(test)]
mod tests {
    use super::super::buffer::RioBufferPool;
    use super::super::rq::{WSAENOBUFS, WSAENOBUFS_HRESULT};
    use super::*;
    use crate::common::block_on;
    use crate::tp::{timeout, Runtime};
    use std::net::{Ipv4Addr, SocketAddrV4};
    use std::time::Duration;

    fn v4() -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
    }

    /// The whole engine end to end: registered buffers, a request queue, the
    /// RIONotify/TP_WAIT pump, and a real datagram.
    #[test]
    fn rio_udp_roundtrip() {
        let rt = Runtime::new();
        let cq = RioCq::new(&rt, 256).expect("completion queue");
        let pool = RioBufferPool::new(2048, 8).expect("buffer pool");
        let server = RioUdpSocket::bind(&cq, v4(), RqConfig::default()).expect("server");
        let client = RioUdpSocket::bind(&cq, v4(), RqConfig::default()).expect("client");
        let server_addr = server.local_addr().unwrap();
        client.connect(server_addr).unwrap();

        let result = block_on(timeout(&rt, Duration::from_secs(5), async {
            // Post the receive first so the datagram has somewhere to land.
            let rx_slice = pool.acquire().unwrap();
            let rx = server.recv(rx_slice).expect("post recv");

            let mut tx_slice = pool.acquire().unwrap();
            tx_slice[..5].copy_from_slice(b"rio!!");
            tx_slice.set_len(5);
            let tx = client.send(tx_slice).expect("post send");

            let (sent, _slice) = tx.await;
            assert_eq!(sent.unwrap(), 5);
            let (received, slice) = rx.await;
            let n = received.unwrap();
            assert_eq!(n, 5);
            assert_eq!(&slice[..n], b"rio!!");
        }));
        result.expect("RIO roundtrip timed out");

        // Both buffers came back through their completions.
        drop(server);
        drop(client);
        assert_eq!(pool.available(), 8, "every slice returned to the pool");
        drop(pool);
        drop(cq);
        drop(rt);
    }

    #[test]
    fn wsaenobufs_hresult_is_assembled_correctly() {
        assert_eq!(
            WSAENOBUFS_HRESULT,
            windows_core::HRESULT::from_win32(WSAENOBUFS as u32)
        );
    }

    /// Unconnected send/receive over RIOSendEx/RIOReceiveEx, where RIO names
    /// the peer address with a RIO_BUF of its own.
    #[test]
    fn rio_udp_addressed_roundtrip() {
        let rt = Runtime::new();
        let cq = RioCq::new(&rt, 256).expect("completion queue");
        let pool = RioBufferPool::new(2048, 8).expect("buffer pool");

        let server = RioUdpSocket::bind(&cq, v4(), RqConfig::default()).expect("server");
        let client = RioUdpSocket::bind(&cq, v4(), RqConfig::default()).expect("client");
        let server_addr = server.local_addr().unwrap();
        let client_addr = client.local_addr().unwrap();

        let result = block_on(timeout(&rt, Duration::from_secs(5), async {
            let rx = server
                .recv_from(pool.acquire().unwrap())
                .expect("post recv_from");

            let mut tx_slice = pool.acquire().unwrap();
            tx_slice[..4].copy_from_slice(b"addr");
            tx_slice.set_len(4);
            let tx = client
                .send_to(tx_slice, server_addr)
                .expect("post send_to");
            let (sent, _s) = tx.await;
            assert_eq!(sent.unwrap().0, 4);

            let (got, slice) = rx.await;
            let (n, from) = got.unwrap();
            assert_eq!(&slice[..n], b"addr");
            // The source address came back through the registered address
            // slot, not from a connect.
            assert_eq!(from.port(), client_addr.port());
        }));
        result.expect("addressed RIO roundtrip timed out");

        drop(server);
        drop(client);
        assert_eq!(pool.available(), 8, "data slices all returned");
    }

    /// Deferred sends are queued and only dispatched by the commit.
    #[test]
    fn deferred_sends_dispatch_on_commit() {
        let rt = Runtime::new();
        let cq = RioCq::new(&rt, 256).expect("completion queue");
        let pool = RioBufferPool::new(1024, 8).expect("buffer pool");

        let server = RioUdpSocket::bind(&cq, v4(), RqConfig::default()).expect("server");
        let client = RioUdpSocket::bind(&cq, v4(), RqConfig::default()).expect("client");
        client.connect(server.local_addr().unwrap()).unwrap();

        let result = block_on(timeout(&rt, Duration::from_secs(5), async {
            let r1 = server.recv(pool.acquire().unwrap()).expect("recv 1");
            let r2 = server.recv(pool.acquire().unwrap()).expect("recv 2");

            let mut a = pool.acquire().unwrap();
            a[..2].copy_from_slice(b"d1");
            a.set_len(2);
            let mut b = pool.acquire().unwrap();
            b[..2].copy_from_slice(b"d2");
            b.set_len(2);

            let s1 = client.send_deferred(a).expect("defer 1");
            let s2 = client.send_deferred(b).expect("defer 2");
            // Nothing has gone out yet; one crossing releases both.
            client.commit_sends().expect("commit");

            assert_eq!(s1.await.0.unwrap(), 2);
            assert_eq!(s2.await.0.unwrap(), 2);

            let (n1, b1) = r1.await;
            let (n2, b2) = r2.await;
            let mut seen = [&b1[..n1.unwrap()], &b2[..n2.unwrap()]];
            seen.sort();
            assert_eq!(seen, [b"d1".as_slice(), b"d2".as_slice()]);
        }));
        result.expect("deferred send timed out");
    }

    /// A polling queue with notification-free posts: no event, no TP_WAIT,
    /// no IOCP — the caller drains when it chooses. This is the shape of a
    /// hybrid loop's hot path.
    #[test]
    fn polling_queue_with_notification_free_posts() {
        use super::super::cq::RioCq;
        let cq = RioCq::polling(64).expect("polling cq");
        let pool = RioBufferPool::new(512, 4).expect("pool");
        let rq = RqConfig {
            max_outstanding_recv: 8,
            max_outstanding_send: 8,
        };
        let server = RioUdpSocket::bind(&cq, v4(), rq).expect("server");
        let client = RioUdpSocket::bind(&cq, v4(), rq).expect("client");
        client.connect(server.local_addr().unwrap()).unwrap();

        // notify() is a no-op here, so one drain-and-rearm loop works across
        // every mode.
        cq.notify().expect("no-op in polling mode");

        let rx = server
            .recv_without_notify(pool.acquire().unwrap())
            .expect("post recv");
        let mut out = pool.acquire().unwrap();
        out[..4].copy_from_slice(b"poll");
        out.set_len(4);
        let tx = client.send_without_notify(out).expect("post send");

        let mut drained = 0usize;
        for _ in 0..1000 {
            drained += cq.drain().expect("queue not corrupt");
            if drained >= 2 {
                break;
            }
            std::thread::yield_now();
        }
        assert_eq!(drained, 2, "both completions came back without any notification");

        assert_eq!(crate::common::block_on(tx).0.unwrap(), 4);
        let (n, slice) = crate::common::block_on(rx);
        assert_eq!(&slice[..n.unwrap()], b"poll");
    }

    /// Request queues grow live; shrinking below what is outstanding is
    /// refused rather than silently dropping work.
    #[test]
    fn request_queue_resizes() {
        let rt = Runtime::new();
        let cq = RioCq::new(&rt, 256).expect("cq");
        let pool = RioBufferPool::new(512, 8).expect("pool");
        let sock = RioUdpSocket::bind(
            &cq,
            v4(),
            RqConfig {
                max_outstanding_recv: 2,
                max_outstanding_send: 2,
            },
        )
        .expect("socket");

        let _a = sock.recv(pool.acquire().unwrap()).expect("first");
        let _b = sock.recv(pool.acquire().unwrap()).expect("second");
        assert!(sock.recv(pool.acquire().unwrap()).is_err(), "cap enforced");

        sock.resize_queue(6, 6).expect("grow");
        let _c = sock.recv(pool.acquire().unwrap()).expect("room after growing");

        // Three receives are outstanding; shrinking under that is refused.
        const WSAETOOMANYREFS: u32 = 10059;
        let err = sock.resize_queue(1, 1).expect_err("shrink below outstanding");
        assert_eq!(err.code(), windows_core::HRESULT::from_win32(WSAETOOMANYREFS));
    }

    /// The request queue is a hard cap, reported synchronously — RIO's
    /// backpressure signal, not a failure.
    #[test]
    fn request_queue_full_is_synchronous_backpressure() {
        let rt = Runtime::new();
        let cq = RioCq::new(&rt, 64).expect("completion queue");
        let pool = RioBufferPool::new(512, 8).expect("buffer pool");
        let sock = RioUdpSocket::bind(
            &cq,
            v4(),
            RqConfig {
                max_outstanding_recv: 2,
                max_outstanding_send: 2,
            },
        )
        .expect("socket");

        let _a = sock.recv(pool.acquire().unwrap()).expect("first");
        let _b = sock.recv(pool.acquire().unwrap()).expect("second");
        // Third exceeds max_outstanding_recv.
        let third = sock.recv(pool.acquire().unwrap());
        match third {
            Ok(_) => panic!("queue cap not enforced"),
            Err(e) => {
                assert_eq!(
                    e.code(),
                    windows_core::HRESULT::from_win32(WSAENOBUFS as u32),
                    "a full request queue must report WSAENOBUFS"
                );
            }
        }
        // The rejected post reclaimed its slice rather than leaking it.
        assert_eq!(pool.available(), 6);
    }
}
