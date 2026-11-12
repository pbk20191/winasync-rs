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

/// The adapter over an AF_UNIX stream: a length-prefixed echo through the
/// combinators, then EOF after the server's `close` half-closes the socket.
#[test]
fn unix_stream_through_the_adapter() {
    use super::super::unix::{UnixListener, UnixStream};
    let rt = Runtime::new();
    let path = std::env::temp_dir()
        .join(format!("tp_unix_io_{}.sock", std::process::id()))
        .to_string_lossy()
        .into_owned();
    let _ = std::fs::remove_file(&path);
    let result = block_on(timeout(&rt, Duration::from_secs(10), async {
        let listener = UnixListener::bind(&rt, &path, None).unwrap();
        let server = rt.spawn(async move {
            let mut io = listener.accept().await.unwrap().into_io();
            let mut buf = vec![0u8; 40_000];
            io.read_exact(&mut buf).await.unwrap();
            io.write_all(&buf).await.unwrap();
            io.close().await.unwrap();
        });

        let (mut r, mut w) = UnixStream::connect(&rt, &path).await.unwrap().into_io().split();
        let payload: Vec<u8> = (0..40_000u32).map(|i| (i % 249) as u8).collect();
        let p2 = payload.clone();
        let writer = rt.spawn(async move {
            w.write_all(&p2).await.unwrap();
            w.flush().await.unwrap();
        });
        let mut echoed = vec![0u8; payload.len()];
        r.read_exact(&mut echoed).await.unwrap();
        assert_eq!(echoed, payload);
        let mut tail = [0u8; 1];
        assert_eq!(r.read(&mut tail).await.unwrap(), 0, "expected EOF after peer close");
        writer.await.unwrap();
        server.await.unwrap();
    }));
    let _ = std::fs::remove_file(&path);
    result.expect("unix adapter echo timed out");
}

/// The adapter over a named pipe, split on both ends. On a message-mode
/// pipe it reads bytes without boundaries: 40 000 bytes written as one
/// message arrive through 16 KiB staging reads. A pipe has no half-close,
/// so EOF comes from dropping the server's halves.
#[test]
fn named_pipe_through_the_adapter() {
    use super::super::pipe::{NamedPipeConnector, NamedPipeListener, NamedPipeOptions};
    let rt = Runtime::new();
    let name = format!(r"\\.\pipe\tp_pipe_io_{}", std::process::id());
    let result = block_on(timeout(&rt, Duration::from_secs(10), async {
        let listener =
            NamedPipeListener::bind(&rt, &name, &NamedPipeOptions::new().message_mode(true)).unwrap();
        let server = rt.spawn(async move {
            let (mut r, mut w) = listener.accept().await.unwrap().into_io().split();
            let mut buf = vec![0u8; 40_000];
            r.read_exact(&mut buf).await.unwrap();
            w.write_all(&buf).await.unwrap();
            w.close().await.unwrap();
            // Both halves gone: the client reads EOF.
        });
        // The instance exists once `accept` has started; as in the pipe tests.
        std::thread::sleep(Duration::from_millis(100));

        let mut io = NamedPipeConnector::connect(&rt, &name).unwrap().into_io();
        let payload: Vec<u8> = (0..40_000u32).map(|i| (i % 247) as u8).collect();
        io.write_all(&payload).await.unwrap();
        let mut echoed = vec![0u8; payload.len()];
        io.read_exact(&mut echoed).await.unwrap();
        assert_eq!(echoed, payload);
        server.await.unwrap();
        let mut tail = [0u8; 1];
        let eof = io.read(&mut tail).await;
        assert!(
            matches!(eof, Ok(0) | Err(_)),
            "expected EOF or a broken pipe after the server dropped its handle, got {eof:?}"
        );
    }));
    result.expect("pipe adapter echo timed out");
}

/// Every stream type wraps, and the halves of each are the ecosystem traits.
#[test]
fn every_stream_wraps() {
    fn assert_read<T: AsyncRead>() {}
    fn assert_write<T: AsyncWrite>() {}
    assert_read::<StreamIo<TcpStream>>();
    assert_write::<StreamIo<TcpStream>>();
    assert_read::<StreamIo<super::super::unix::UnixStream>>();
    assert_read::<StreamIo<super::super::hvsock::HvStream>>();
    assert_read::<StreamIo<super::super::pipe::NamedPipeStream>>();
    assert_read::<StreamIo<super::super::serial::SerialPort>>();
    assert_write::<WriteHalf<super::super::serial::SerialPort>>();
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
    assert_read::<StreamIo<super::super::pipe::NamedPipeStream>>();
    assert_write::<WriteHalf<super::super::unix::UnixStream>>();
}
