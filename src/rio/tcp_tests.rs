// Several of these are tp-only; their imports are shared.
#![allow(unused_imports)]
use super::super::buffer::RioBufferPool;
use super::*;
use crate::common::block_on;
#[cfg(feature = "tp")]
use crate::tp::{timeout, Runtime};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

/// Several sends queued with RIO_MSG_DEFER and released by one commit.
///
/// TCP has no message boundaries, so the receiver may see the batch as
/// one read or several — the test accepts either and only insists the
/// bytes arrive, in order, after the commit.
#[cfg(feature = "tp")]
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

/// One socket, two engines: `tp` awaits the handshake, RIO carries the
/// data. No blocking accept, no second thread.
#[cfg(feature = "tp")]
#[test]
fn composed_handshake_then_rio_data_path() {
    use super::RioTcpAcceptor;
    let rt = Runtime::new();
    let cq = RioCq::new(&rt, 256).expect("cq");
    let pool = RioBufferPool::new(512, 8).expect("pool");

    let listener = RioTcpAcceptor::bind(
        &rt,
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
    )
    .expect("bind");
    let addr = listener.local_addr().unwrap();

    let result = block_on(timeout(&rt, Duration::from_secs(5), async {
        let rq = RqConfig::default();
        // Both sides of the handshake are awaited; AcceptEx and ConnectEx
        // are in flight at once.
        let (server, client) = futures_join(
            listener.accept(&cq, rq),
            RioTcpStream::connect(&rt, &cq, addr, rq),
        )
        .await;
        let server = server.expect("accept");
        let client = client.expect("connect");

        let srv_rx = server.recv(pool.acquire().unwrap()).expect("recv");
        let mut out = pool.acquire().unwrap();
        out[..7].copy_from_slice(b"composed"[..7].try_into().unwrap());
        out.set_len(7);
        client.send(out).expect("send").await.0.unwrap();

        let (n, got) = srv_rx.await;
        assert_eq!(&got[..n.unwrap()], b"compose");
    }));
    result.expect("composed handshake timed out");
}

/// Minimal join: poll both to completion on this thread.
#[cfg(feature = "tp")]
async fn futures_join<A: Future, B: Future>(a: A, b: B) -> (A::Output, B::Output) {
    // The accept cannot finish before the connect is issued, so issue the
    // connect first and then await both in turn.
    let mut a = Box::pin(a);
    let mut b = Box::pin(b);
    use std::task::Poll;
    let mut ra = None;
    let mut rb = None;
    std::future::poll_fn(move |cx| {
        if ra.is_none()
            && let Poll::Ready(v) = a.as_mut().poll(cx) {
                ra = Some(v);
            }
        if rb.is_none()
            && let Poll::Ready(v) = b.as_mut().poll(cx) {
                rb = Some(v);
            }
        if ra.is_some() && rb.is_some() {
            Poll::Ready((ra.take().unwrap(), rb.take().unwrap()))
        } else {
            Poll::Pending
        }
    })
    .await
}

#[cfg(feature = "tp")]
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
        let _ = tx.send(r.map(Box::new));
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
