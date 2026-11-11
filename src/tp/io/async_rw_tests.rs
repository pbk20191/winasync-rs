use super::super::super::{block_on, timeout, Runtime};
use super::super::net::{SocketOptions, TcpListener, TcpStream};
use super::*;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

#[test]
fn async_rw_echo_over_traits() {
    let rt = Runtime::new();
    let result = block_on(timeout(&rt, Duration::from_secs(10), async {
        let listener = TcpListener::bind(
            &rt,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            &SocketOptions::new(),
        )
        .unwrap();
        let addr = listener.local_addr().unwrap();

        // Server: read_exact a length-prefixed payload, echo it back via
        // write_all, then close the write half.
        let server = rt.spawn(async move {
            let mut io = listener.accept().await.unwrap().into_io();
            let mut len = [0u8; 4];
            io.read_exact(&mut len).await.unwrap();
            let len = u32::from_be_bytes(len) as usize;
            let mut payload = vec![0u8; len];
            io.read_exact(&mut payload).await.unwrap();
            io.write_all(&payload).await.unwrap();
            io.close().await.unwrap();
            len
        });

        let mut io = TcpStream::connect(
            &rt,
            addr,
            &SocketOptions::new(),
        ).await.unwrap().into_io();
        let payload: Vec<u8> = (0..50_000u32).map(|i| (i % 253) as u8).collect();
        io.write_all(&(payload.len() as u32).to_be_bytes()).await.unwrap();
        io.write_all(&payload).await.unwrap();
        io.flush().await.unwrap();

        // Read the echo back with read_exact; the adapter's short-read
        // remainder handling is exercised by the staging-cap-sized chunks.
        let mut echoed = vec![0u8; payload.len()];
        io.read_exact(&mut echoed).await.unwrap();
        assert_eq!(echoed, payload);

        // peer_addr is reachable through the adapter's get_ref.
        assert!(io.get_ref().peer_addr().is_ok());

        // After the server closes its write half, the next read is EOF.
        let mut tail = [0u8; 1];
        let got = io.read(&mut tail).await.unwrap();
        assert_eq!(got, 0, "expected EOF after peer close");

        server.await.unwrap()
    }));
    assert_eq!(result.expect("async_rw echo timed out"), 50_000);
}

#[test]
fn split_full_duplex() {
    let rt = Runtime::new();
    let result = block_on(timeout(&rt, Duration::from_secs(10), async {
        let listener = TcpListener::bind(
            &rt,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            &SocketOptions::new(),
        )
        .unwrap();
        let addr = listener.local_addr().unwrap();

        // Echo server over the combined adapter.
        let server = rt.spawn(async move {
            let mut io = listener.accept().await.unwrap().into_io();
            let mut buf = vec![0u8; 10_000];
            io.read_exact(&mut buf).await.unwrap();
            io.write_all(&buf).await.unwrap();
            io.close().await.unwrap();
        });

        // Client: split, then write (on a spawned task) and read
        // concurrently — impossible without split, since both need &mut.
        let (mut r, mut w) = TcpStream::connect(
            &rt,
            addr,
            &SocketOptions::new(),
        ).await.unwrap().into_io().split();
        let payload: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let p2 = payload.clone();
        let writer = rt.spawn(async move {
            w.write_all(&p2).await.unwrap();
            w.close().await.unwrap();
        });

        let mut echoed = vec![0u8; payload.len()];
        r.read_exact(&mut echoed).await.unwrap();
        writer.await.unwrap();
        server.await.unwrap();
        echoed == payload
    }));
    assert!(result.expect("split full-duplex timed out"));
}

#[cfg(feature = "futures-io")]
#[test]
fn implements_futures_io_traits() {
    fn assert_read<T: futures_io::AsyncRead>() {}
    fn assert_write<T: futures_io::AsyncWrite>() {}
    assert_read::<TcpStreamIo>();
    assert_write::<TcpStreamIo>();
    assert_read::<ReadHalf>();
    assert_write::<WriteHalf>();
}
