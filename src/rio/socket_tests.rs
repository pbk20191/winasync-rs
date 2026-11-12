// Several of these are tp-only; their imports are shared.
#![allow(unused_imports)]
use super::super::buffer::RioBufferPool;
use super::super::rq::{WSAENOBUFS, WSAENOBUFS_HRESULT};
use super::*;
use crate::common::block_on;
#[cfg(feature = "tp")]
use crate::tp::{timeout, Runtime};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

fn v4() -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
}

/// The whole engine end to end: registered buffers, a request queue, the
/// RIONotify/TP_WAIT pump, and a real datagram.
#[cfg(feature = "tp")]
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
#[cfg(feature = "tp")]
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
#[cfg(feature = "tp")]
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

/// The default-pool pump, with no runtime from this crate: `rio` alone,
/// and the caller writes no drain loop. `block_on` only parks this
/// thread — the completions are dispatched by a default-pool thread.
#[test]
fn default_pool_pump_drives_a_roundtrip() {
    use super::super::cq::RioCq;
    use std::time::Duration;
    let cq = RioCq::with_default_pool(64).expect("default-pool cq");
    let pool = RioBufferPool::new(512, 4).expect("pool");
    let rq = RqConfig {
        max_outstanding_recv: 8,
        max_outstanding_send: 8,
    };
    let server = RioUdpSocket::bind(&cq, v4(), rq).expect("server");
    let client = RioUdpSocket::bind(&cq, v4(), rq).expect("client");
    client.connect(server.local_addr().unwrap()).unwrap();

    let rx = server.recv(pool.acquire().unwrap()).expect("post recv");
    let mut out = pool.acquire().unwrap();
    out[..4].copy_from_slice(b"dflt");
    out.set_len(4);
    let tx = client.send(out).expect("post send");

    // `tp::timeout` is unavailable in an rio-only build and `block_on`
    // parks on INFINITE, so a dead pump would hang the whole test binary
    // with no named failure. Watchdog thread instead.
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let sent = crate::common::block_on(tx).0;
        let (n, slice) = crate::common::block_on(rx);
        let bytes = n.as_ref().map(|&n| slice[..n].to_vec()).unwrap_or_default();
        let _ = done_tx.send((sent, n, bytes));
    });
    let (sent, n, bytes) = done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("default-pool pump never dispatched");
    assert_eq!(sent.unwrap(), 4);
    assert_eq!(n.unwrap(), 4);
    assert_eq!(bytes, b"dflt");

    assert!(cq.pump_error().is_none(), "pump died: {:?}", cq.pump_error());
    drop(server);
    drop(client);
    // Buffers come back only if the completions were dispatched.
    assert_eq!(pool.available(), 4, "every slice returned to the pool");
    drop(cq);
}

/// Waits for every slice of `pool` to be back, which only a dispatched
/// completion can bring about.
fn wait_all_back(pool: &RioBufferPool, count: usize) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while pool.available() != count {
        assert!(std::time::Instant::now() < deadline, "{} of {count} slices back", pool.available());
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A future dropped before its completion, and one dropped after it but
/// never polled: both leave the slot to be emptied by whichever side comes
/// last, and the slice — the only one in the pool — is good for another
/// round trip after each.
#[test]
fn dropped_futures_return_their_slices_and_free_their_slots() {
    use super::super::cq::RioCq;
    let cq = RioCq::with_default_pool(64).expect("default-pool cq");
    let pool = RioBufferPool::new(512, 1).expect("pool");
    let tx_pool = RioBufferPool::new(512, 1).expect("send pool");
    let rq = RqConfig {
        max_outstanding_recv: 8,
        max_outstanding_send: 8,
    };
    let client = RioUdpSocket::bind(&cq, v4(), rq).expect("client");

    // Dropped before any datagram: the completion comes from closesocket.
    let server = RioUdpSocket::bind(&cq, v4(), rq).expect("server");
    let rx = server.recv(pool.acquire().unwrap()).expect("post recv");
    drop(rx);
    assert_eq!(pool.available(), 0, "the slot keeps the slice for the kernel");
    drop(server);
    wait_all_back(&pool, 1);

    // Dropped after the completion landed, without ever being polled.
    let server = RioUdpSocket::bind(&cq, v4(), rq).expect("server");
    client.connect(server.local_addr().unwrap()).unwrap();
    let rx = server.recv(pool.acquire().unwrap()).expect("post recv");
    let mut out = tx_pool.acquire().unwrap();
    out.set_len(3);
    let (sent, _) = block_on(client.send(out).expect("post send"));
    assert_eq!(sent.unwrap(), 3);
    // The receive completes with the send's datagram, moments later.
    // Nothing observable from outside says when it is in; the drop below
    // is correct whether it is or not, and the sleep just makes "after"
    // the likely case.
    std::thread::sleep(Duration::from_millis(50));
    drop(rx);
    wait_all_back(&pool, 1);

    // The same slot, used again, resolves normally.
    let rx = server.recv(pool.acquire().unwrap()).expect("post recv");
    let mut out = tx_pool.acquire().unwrap();
    out[..2].copy_from_slice(b"ok");
    out.set_len(2);
    let (sent, _) = block_on(client.send(out).expect("post send"));
    assert_eq!(sent.unwrap(), 2);
    let (n, slice) = block_on(rx);
    assert_eq!(&slice[..n.unwrap()], b"ok");
    drop(slice);
    assert_eq!(pool.available(), 1);
    assert!(cq.pump_error().is_none(), "pump died: {:?}", cq.pump_error());
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
#[cfg(feature = "tp")]
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
    // Polling mode, so this exercises RIO's own backpressure without
    // needing a runtime from elsewhere.
    let cq = RioCq::polling(64).expect("completion queue");
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
