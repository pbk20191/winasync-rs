    use super::super::super::{block_on, timeout, Runtime};
    use super::*;
use crate::tp::{FileRange, TransmitSequence};
    use std::time::Duration;

    #[test]
    fn tcp_echo_self_connect() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();
            assert_ne!(addr.port(), 0);

            // Echo server: accept one connection, echo until the peer
            // half-closes, then half-close back.
            let server = rt.spawn(async move {
                let stream = listener.accept().await.unwrap();
                // Peer address is available post-SO_UPDATE_ACCEPT_CONTEXT.
                assert!(stream.peer_addr().is_ok());
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
            let client = TcpStream::connect(&rt, addr, &SocketOptions::new()).await.unwrap();
            // The client's peer address is the listener's address.
            assert_eq!(client.peer_addr().unwrap().port(), addr.port());
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
    fn tcp_echo_ipv6() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();
            assert!(matches!(addr, SocketAddr::V6(_)), "expected an IPv6 bind");
            assert_ne!(addr.port(), 0);

            let server = rt.spawn(async move {
                let stream = listener.accept().await.unwrap();
                let (n, b) = stream.read(vec![0u8; 64]).await;
                let n = n.unwrap();
                stream.write(b[..n].to_vec()).await.0.unwrap();
            });

            let client = TcpStream::connect(&rt, addr, &SocketOptions::new()).await.unwrap();
            assert!(matches!(client.peer_addr().unwrap(), SocketAddr::V6(_)));
            client.write(b"v6ping".to_vec()).await.0.unwrap();
            let (n, b) = client.read(vec![0u8; 64]).await;
            assert_eq!(&b[..n.unwrap()], b"v6ping");
            server.await.unwrap();
        }));
        result.expect("ipv6 echo test timed out");
    }

    #[test]
    fn udp_send_recv_roundtrip() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let server = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let server_addr = server.local_addr().unwrap();
            let client_addr = client.local_addr().unwrap();

            // Client -> server.
            let (w, _) = client.send_to(b"ping".to_vec(), server_addr).await;
            assert_eq!(w.unwrap(), 4);

            let (r, b) = server.recv_from(vec![0u8; 64]).await;
            let (n, from) = r.unwrap();
            assert_eq!(&b[..n], b"ping");
            // The datagram's source is the client's bound address.
            assert_eq!(from.port(), client_addr.port());

            // Server echoes back to the reported source.
            let (w, _) = server.send_to(b"pong".to_vec(), from).await;
            assert_eq!(w.unwrap(), 4);
            let (r, b) = client.recv_from(vec![0u8; 64]).await;
            let (n, _) = r.unwrap();
            assert_eq!(&b[..n], b"pong");
        }));
        result.expect("udp roundtrip timed out");
    }

    #[test]
    fn udp_ipv6_roundtrip() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let server = UdpSocket::bind(
                &rt,
                SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let server_addr = server.local_addr().unwrap();
            assert!(matches!(server_addr, SocketAddr::V6(_)));

            client.send_to(b"v6dgram".to_vec(), server_addr).await.0.unwrap();
            let (r, b) = server.recv_from(vec![0u8; 64]).await;
            let (n, from) = r.unwrap();
            assert_eq!(&b[..n], b"v6dgram");
            assert!(matches!(from, SocketAddr::V6(_)));
        }));
        result.expect("udp ipv6 roundtrip timed out");
    }

    #[test]
    fn udp_connected_send_recv() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let server = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let server_addr = server.local_addr().unwrap();
            let client_addr = client.local_addr().unwrap();

            // Pin each socket to the other so send/recv work without addresses.
            client.connect(server_addr).unwrap();
            server.connect(client_addr).unwrap();

            client.send(b"hi".to_vec()).await.0.unwrap();
            let (n, b) = server.recv(vec![0u8; 64]).await;
            assert_eq!(&b[..n.unwrap()], b"hi");

            server.send(b"yo".to_vec()).await.0.unwrap();
            let (n, b) = client.recv(vec![0u8; 64]).await;
            assert_eq!(&b[..n.unwrap()], b"yo");
        }));
        result.expect("udp connected send/recv timed out");
    }

    #[test]
    fn transmit_file_over_tcp() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let path = {
                let mut p = std::env::temp_dir();
                p.push(format!("tp_transmit_{}.bin", std::process::id()));
                p.to_string_lossy().into_owned()
            };
            let content: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
            let file = crate::tp::File::create(&rt, &path).unwrap();
            let (w, _) = file.write_at(content.clone(), 0).await;
            assert_eq!(w.unwrap(), content.len());

            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();

            // Server streams the whole file over the socket, then half-closes.
            let server = rt.spawn(async move {
                let stream = listener.accept().await.unwrap();
                let n = stream.transmit_file(&file, 0, 0, TransmitFlags::default()).await.unwrap();
                stream.shutdown_send().unwrap();
                n
            });

            let client = TcpStream::connect(&rt, addr, &SocketOptions::new()).await.unwrap();
            let mut got = Vec::new();
            loop {
                let (n, b) = client.read(vec![0u8; 8192]).await;
                let n = n.unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&b[..n]);
            }
            let sent = server.await.unwrap();
            (sent, got, content, path)
        }));
        let (sent, got, content, path) = result.expect("transmit_file timed out");
        assert_eq!(sent, content.len());
        assert_eq!(got, content);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn transmit_packets_mem_and_file() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let path = {
                let mut p = std::env::temp_dir();
                p.push(format!("tp_packets_{}.bin", std::process::id()));
                p.to_string_lossy().into_owned()
            };
            let body: Vec<u8> = (0..8000u32).map(|i| (i % 251) as u8).collect();
            let file = crate::tp::File::create(&rt, &path).unwrap();
            file.write_at(body.clone(), 0).await.0.unwrap();
            let header = b"HDR:".to_vec();

            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();

            // Server scatters an in-memory header + the whole file in one call.
            let hdr = header.clone();
            let server = rt.spawn(async move {
                let stream = listener.accept().await.unwrap();
                let pk = hdr.then(FileRange::new(&file, 0, 0));
                let n = stream.transmit_packets(pk, TransmitFlags::default()).await.unwrap();
                stream.shutdown_send().unwrap();
                n
            });

            let client = TcpStream::connect(&rt, addr, &SocketOptions::new()).await.unwrap();
            let mut got = Vec::new();
            loop {
                let (n, b) = client.read(vec![0u8; 8192]).await;
                let n = n.unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&b[..n]);
            }
            let sent = server.await.unwrap();
            (sent, got, header, body, path)
        }));
        let (sent, got, header, body, path) = result.expect("transmit_packets timed out");
        let mut expected = header;
        expected.extend_from_slice(&body);
        assert_eq!(got, expected, "scattered header+file bytes mismatch");
        assert_eq!(sent, expected.len());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn disconnect_only_completes() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(5), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();
            let client = TcpStream::connect(&rt, addr, &SocketOptions::new()).await.unwrap();
            let s1 = listener.accept().await.unwrap();
            // DisconnectEx is graceful: it completes once the peer has closed too,
            // so close the server side first (the connection is finished).
            drop(s1);
            client.disconnect(true).await.unwrap();
        }));
        result.expect("disconnect alone timed out");
    }

    /// Disconnect a client socket with `DisconnectEx(TF_REUSE_SOCKET)` and
    /// connect the *same* socket again.
    ///
    /// The second connection goes to a second listener, on purpose. The
    /// reused socket keeps its local binding, so reconnecting to the first
    /// listener recreates the identical four-tuple the first connection just
    /// tore down -- and that tuple is in TIME_WAIT. It failed rarely that way
    /// (60 isolated runs and 15 full suites were clean) until the release
    /// suite caught it on 2026-11-10:
    ///
    /// ```text
    /// reconnect to 127.0.0.1:55616 from Some(127.0.0.1:55618) failed:
    ///     HRESULT(0x80070034)
    /// ```
    ///
    /// `0x80070034` is `ERROR_DUP_NAME`, which is how `WSAEADDRINUSE` arrives
    /// through the overlapped path: `ConnectEx` fails with
    /// `STATUS_ADDRESS_ALREADY_EXISTS`, and `GetOverlappedResult` maps that
    /// NTSTATUS to the Win32 error, not to the Winsock one. A different remote
    /// port is a different tuple, and what the test is about -- the same
    /// handle, with no new `socket()`/`bind()`/registration -- is unchanged.
    #[test]
    fn disconnect_reconnect_client() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let loopback = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
            let listener = TcpListener::bind(&rt, loopback, &SocketOptions::new()).unwrap();
            let addr = listener.local_addr().unwrap();
            let second = TcpListener::bind(&rt, loopback, &SocketOptions::new()).unwrap();
            let addr2 = second.local_addr().unwrap();

            // First connection on a fresh client socket.
            let client = TcpStream::connect(&rt, addr, &SocketOptions::new())
                .await
                .expect("first connect");
            let local = client.local_addr().ok();
            let s1 = listener.accept().await.expect("first accept");
            client.write(b"one".to_vec()).await.0.expect("first write");
            let (n, b) = s1.read(vec![0u8; 8]).await;
            assert_eq!(&b[..n.expect("first read")], b"one");

            // Finish the connection (close the peer), then disconnect + reuse the
            // SAME client handle and reconnect -- no new socket()/bind()/register.
            drop(s1);
            client
                .disconnect(true)
                .await
                .expect("graceful DisconnectEx after the peer closed");
            client.reconnect(addr2).await.unwrap_or_else(|e| {
                panic!("reconnect to {addr2} from {local:?} failed: {e:?}")
            });

            let s2 = second.accept().await.expect("second accept");
            client.write(b"two".to_vec()).await.0.expect("second write");
            let (n, b) = s2.read(vec![0u8; 8]).await;
            assert_eq!(&b[..n.expect("second read")], b"two");
        }));
        result.expect("disconnect/reconnect timed out");
    }

    #[test]
    fn accept_into_server_reuse() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();

            let c1 = TcpStream::connect(&rt, addr, &SocketOptions::new()).await.unwrap();
            let s1 = listener.accept().await.unwrap();
            c1.write(b"a".to_vec()).await.0.unwrap();
            let (n, b) = s1.read(vec![0u8; 4]).await;
            assert_eq!(&b[..n.unwrap()], b"a");

            // Finish the connection (close the client), then the server disconnects
            // its accepted socket for reuse and hands it to the next accept instead
            // of creating a new one.
            drop(c1);
            s1.disconnect(true).await.unwrap();
            let reusable = s1.into_reusable();

            let c2 = TcpStream::connect(&rt, addr, &SocketOptions::new()).await.unwrap();
            let s2 = listener.accept_into(reusable).await.unwrap();
            c2.write(b"b".to_vec()).await.0.unwrap();
            let (n, b) = s2.read(vec![0u8; 4]).await;
            assert_eq!(&b[..n.unwrap()], b"b");
        }));
        result.expect("accept_into reuse timed out");
    }

    #[test]
    fn udp_recv_msg_pktinfo() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let server = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            server.set_recv_pktinfo(true).unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let server_addr = server.local_addr().unwrap();

            client.send_msg(b"msg!".to_vec(), server_addr, Vec::new()).await.0.unwrap();
            let (r, b) = server.recv_msg(vec![0u8; 64], 128).await;
            let rm = r.unwrap();
            assert_eq!(&b[..rm.bytes], b"msg!");
            assert!(matches!(rm.source, SocketAddr::V4(_)));
            // IP_PKTINFO reports the local (loopback) address the datagram hit.
            assert_eq!(parse_ip_pktinfo(&rm.control), Some(Ipv4Addr::LOCALHOST));
        }));
        result.expect("recv_msg pktinfo timed out");
    }

    #[test]
    fn skip_completion_on_success_echoes_and_leaks_nothing() {
        use crate::common::overlapped::INLINE_COMPLETIONS;
        let rt = Runtime::new();
        let inline_before = INLINE_COMPLETIONS.load(std::sync::atomic::Ordering::Relaxed);
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();
            let server = rt.spawn(async move {
                let stream = listener.accept().await.unwrap();
                stream.set_skip_completion_on_success().unwrap();
                let mut echoed = 0usize;
                // Echo until the peer half-closes; several round trips, so the
                // sync-success path (data already buffered) is the one
                // actually taken most times.
                loop {
                    let (n, b) = stream.read(vec![0u8; 64]).await;
                    let n = n.unwrap();
                    if n == 0 {
                        break;
                    }
                    stream.write(b[..n].to_vec()).await.0.unwrap();
                    echoed += n;
                }
                echoed
            });

            let client = TcpStream::connect(&rt, addr, &SocketOptions::new()).await.unwrap();
            client.set_skip_completion_on_success().unwrap();
            for i in 0..16u8 {
                let msg = vec![i; 8];
                client.write(msg.clone()).await.0.unwrap();
                let (n, b) = client.read(vec![0u8; 64]).await;
                assert_eq!(&b[..n.unwrap()], &msg[..]);
            }
            // Vectored and peek go through the same classifier.
            client
                .write_vectored(vec![b"ab".to_vec(), b"cd".to_vec()])
                .await
                .0
                .unwrap();
            let (n, b) = client.read(vec![0u8; 8]).await;
            assert_eq!(&b[..n.unwrap()], b"abcd");

            client.shutdown_send().unwrap();
            assert_eq!(server.await.unwrap(), 16 * 8 + 4);
        }));
        result.expect("skip-completion echo timed out");
        // Without this the test would pass just as well with the mode inert:
        // waiting for a packet produces the same bytes, only slower.
        {
            let inlined =
                INLINE_COMPLETIONS.load(std::sync::atomic::Ordering::Relaxed) - inline_before;
            assert!(
                inlined > 0,
                "skip-completion mode never took the inline path — the flag is not in effect"
            );
        }
        drop(rt);
        // `LIVE_OPS` is process-global, so the only value this test can
        // assert on is the quiescent one: a snapshot taken at entry counts
        // whatever concurrently running tests happened to have in flight
        // then, and comparing against it fails when they finish first.
        // Waiting for zero is what every other leak check here does.
        for _ in 0..500 {
            if super::super::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(super::super::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed), 0, "{}: {}", "op blocks leaked on the skip-completion path", crate::common::overlapped::live_ops_report());
    }

    #[test]
    fn accept_with_data_gets_first_segment() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();
            let server = rt.spawn(async move {
                let (stream, data) = listener.accept_with_data(64).await.unwrap();
                // The accepted socket is fully usable; the data came for free.
                assert!(stream.peer_addr().is_ok());
                data
            });

            let client = TcpStream::connect(&rt, addr, &SocketOptions::new()).await.unwrap();
            client.write(b"GET / HTTP/1.1".to_vec()).await.0.unwrap();
            let data = server.await.unwrap();
            assert_eq!(&data, b"GET / HTTP/1.1");
        }));
        result.expect("accept_with_data timed out");
    }

    /// USO must actually segment: one send of 5 x 300 bytes has to arrive as
    /// five separate 300-byte datagrams, not one 1500-byte one. Asserting
    /// only that `set_send_segment_size` returns Ok would pass with the
    /// feature doing nothing.
    #[test]
    fn uso_segments_one_send_into_many_datagrams() {
        const SEG: usize = 300;
        const COUNT: usize = 5;
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let server = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let server_addr = server.local_addr().unwrap();

            assert_eq!(client.send_segment_size().unwrap(), None, "off by default");
            if let Err(e) = client.set_send_segment_size(Some(SEG as u16)) {
                eprintln!("skipped: USO unsupported here ({e:?})");
                return;
            }
            // The SDK's wrappers are a Get/Set pair; read it back.
            assert_eq!(client.send_segment_size().unwrap(), Some(SEG as u16));

            // Each segment is filled with its own index, so a mis-split shows
            // up as mixed content rather than just a wrong length.
            let mut big = Vec::with_capacity(SEG * COUNT);
            for i in 0..COUNT {
                big.extend(std::iter::repeat_n(b'a' + i as u8, SEG));
            }
            let (n, _) = client.send_to(big, server_addr).await;
            assert_eq!(n.unwrap(), SEG * COUNT, "the send itself is one call");

            for i in 0..COUNT {
                let (r, buf) = server.recv_from(vec![0u8; 2048]).await;
                let (n, _from) = r.unwrap();
                assert_eq!(n, SEG, "segment {i} should arrive on its own");
                assert!(
                    buf[..n].iter().all(|&b| b == b'a' + i as u8),
                    "segment {i} has mixed content"
                );
            }
        }));
        result.expect("USO segmentation timed out");
    }

    /// The per-send control-message form, which leaves the socket option
    /// alone.
    #[test]
    fn uso_per_send_control_message_segments() {
        const SEG: usize = 200;
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let server = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let server_addr = server.local_addr().unwrap();

            let ctrl = cmsg_send_segment_size(SEG as u16);
            let (n, _) = client
                .send_msg(vec![7u8; SEG * 3], server_addr, ctrl)
                .await;
            match n {
                Ok(n) => assert_eq!(n, SEG * 3),
                Err(e) => {
                    eprintln!("skipped: per-send USO unsupported here ({e:?})");
                    return;
                }
            }
            for _ in 0..3 {
                let (r, _) = server.recv_from(vec![0u8; 2048]).await;
                assert_eq!(r.unwrap().0, SEG);
            }
        }));
        result.expect("per-send USO timed out");
    }

    /// URO: the option round-trips, and *if* the stack coalesces, the segment
    /// size comes back in a `UDP_COALESCED_INFO` control message. Whether
    /// loopback coalesces at all is the stack's call, so the test reports
    /// which happened rather than demanding one.
    #[test]
    fn uro_option_roundtrips_and_reports_coalescing() {
        const SEG: usize = 100;
        const COUNT: usize = 4;
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let server = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let server_addr = server.local_addr().unwrap();

            assert_eq!(server.recv_coalesced_size().unwrap(), None, "off by default");
            if let Err(e) = server.set_recv_coalesced_size(Some((SEG * COUNT) as u32)) {
                eprintln!("skipped: URO unsupported here ({e:?})");
                return;
            }
            assert_eq!(
                server.recv_coalesced_size().unwrap(),
                Some((SEG * COUNT) as u32),
                "Get/Set pair must agree"
            );

            for i in 0..COUNT {
                client
                    .send_to(vec![b'a' + i as u8; SEG], server_addr)
                    .await
                    .0
                    .unwrap();
            }

            let (msg, _buf) = server.recv_msg(vec![0u8; 4096], 512).await;
            let msg = msg.unwrap();
            match parse_udp_coalesced_info(&msg.control) {
                Some(seg) => {
                    // Coalesced: the buffer holds several datagrams of `seg`.
                    assert_eq!(seg as usize, SEG);
                    assert!(msg.bytes > SEG, "coalescing should deliver more than one");
                }
                None => {
                    // Not coalesced: a single datagram, which is legal.
                    assert_eq!(msg.bytes, SEG);
                    eprintln!("note: loopback delivered one datagram, no coalescing");
                }
            }
        }));
        result.expect("URO roundtrip timed out");
    }

    /// Joins a group and receives on it, all on this host: TTL 0 keeps the
    /// datagram from leaving the machine while still delivering it locally.
    /// Multicast needs a capable interface, so a machine without one reports
    /// a skip rather than failing.
    #[test]
    fn multicast_v4_join_and_receive() {
        let rt = Runtime::new();
        let group: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 99);
        let result = block_on(timeout(&rt, Duration::from_secs(5), async {
            // SO_REUSEADDR at bind time — several receivers may share the port.
            let rx = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)),
                &SocketOptions::new().reuse_address(true),
            )
            .unwrap();
            let port = rx.local_addr().unwrap().port();
            if let Err(e) = rx.join_multicast_v4(group, Ipv4Addr::UNSPECIFIED) {
                eprintln!("skipped: no multicast-capable interface ({e:?})");
                return;
            }
            // Winsock's IP_MULTICAST_LOOP is the *receive* path, so it belongs
            // on the receiver — the opposite of BSD, where it would go on the
            // sender.
            rx.set_multicast_loop_v4(true).unwrap();

            let tx = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            // TTL 0: delivered on this host, never forwarded off it.
            tx.set_multicast_ttl_v4(0).unwrap();

            let dest = SocketAddr::V4(SocketAddrV4::new(group, port));
            tx.send_to(b"mcast".to_vec(), dest).await.0.unwrap();

            let (r, buf) = rx.recv_from(vec![0u8; 64]).await;
            let (n, _from) = r.unwrap();
            assert_eq!(&buf[..n], b"mcast");

            rx.leave_multicast_v4(group, Ipv4Addr::UNSPECIFIED).unwrap();
        }));
        if result.is_err() {
            eprintln!("skipped: multicast not delivered on this host");
        }
    }

    /// The source-filtering options accept their arguments and round-trip
    /// join/leave. Whether a given source is actually filtered needs two
    /// hosts, so this covers the plumbing only.
    #[test]
    fn multicast_source_filters_are_accepted() {
        let rt = Runtime::new();
        let group: Ipv4Addr = Ipv4Addr::new(232, 1, 2, 3); // SSM range
        let source: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);
        let sock = UdpSocket::bind(
            &rt,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)),
            &SocketOptions::new().reuse_address(true),
        )
        .unwrap();
        match sock.join_multicast_source_v4(group, source, Ipv4Addr::UNSPECIFIED) {
            Ok(()) => {
                sock.leave_multicast_source_v4(group, source, Ipv4Addr::UNSPECIFIED)
                    .unwrap();
            }
            Err(e) => eprintln!("skipped: SSM unavailable here ({e:?})"),
        }
        // Blocking applies to an any-source join, so join first.
        let asm: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 100);
        if sock.join_multicast_v4(asm, Ipv4Addr::UNSPECIFIED).is_ok() {
            match sock.block_multicast_source_v4(asm, source, Ipv4Addr::UNSPECIFIED) {
                Ok(()) => sock
                    .unblock_multicast_source_v4(asm, source, Ipv4Addr::UNSPECIFIED)
                    .unwrap(),
                Err(e) => eprintln!("skipped: source blocking unavailable ({e:?})"),
            }
            let _ = sock.leave_multicast_v4(asm, Ipv4Addr::UNSPECIFIED);
        }
    }

    #[test]
    fn cmsg_build_and_iterate_roundtrip() {
        // Two messages back to back must both come out, with payloads intact.
        let mut buf = build_cmsg(1, 2, &[0xAAu8; 3]);
        buf.extend(build_cmsg(17, 4, &0x1234_5678u32.to_ne_bytes()));
        let got: Vec<(i32, i32, Vec<u8>)> = iter_cmsgs(&buf)
            .map(|(l, t, d)| (l, t, d.to_vec()))
            .collect();
        assert_eq!(got.len(), 2, "a second control message must not be lost");
        assert_eq!(got[0], (1, 2, vec![0xAAu8; 3]));
        assert_eq!(got[1], (17, 4, 0x1234_5678u32.to_ne_bytes().to_vec()));
        // Losing only the trailing *padding* keeps both messages: the second
        // one's content still fits.
        assert_eq!(iter_cmsgs(&buf[..buf.len() - 2]).count(), 2);
        // Cutting into the second message's payload drops it, rather than
        // reading past the end of the buffer.
        assert_eq!(iter_cmsgs(&buf[..buf.len() - 6]).count(), 1);
        // A buffer too short for even one header yields nothing.
        assert_eq!(iter_cmsgs(&buf[..8]).count(), 0);
    }

    /// Pooled slab memory straight through the overlapped engine — the
    /// point of the buffer trait. No `Vec` anywhere on the IO path.
    #[test]
    fn pooled_buffers_go_through_tcp_and_udp() {
        use crate::common::buffer::SlabPool;
        let rt = Runtime::new();
        let pool = SlabPool::new(256, 4).unwrap();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            // TCP
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();
            let server = rt.spawn(async move {
                let s = listener.accept().await.unwrap();
                let (n, b) = s.read(vec![0u8; 64]).await;
                let n = n.unwrap();
                assert_eq!(&b[..n], b"slab");
            });
            let client = TcpStream::connect(&rt, addr, &SocketOptions::new()).await.unwrap();
            let mut out = pool.acquire().unwrap();
            out[..4].copy_from_slice(b"slab");
            out.set_len(4);
            let (n, out) = client.write(out).await;
            assert_eq!(n.unwrap(), 4);
            server.await.unwrap();
            drop(out);

            // UDP, receiving into a pooled slice this time.
            let rx = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let tx = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let rx_addr = rx.local_addr().unwrap();
            let mut dgram = pool.acquire().unwrap();
            dgram[..3].copy_from_slice(b"pop");
            dgram.set_len(3);
            tx.send_to(dgram, rx_addr).await.0.unwrap();

            let landing = pool.acquire().unwrap();
            let (r, landing) = rx.recv_from(landing).await;
            let (n, _from) = r.unwrap();
            assert_eq!(&landing[..n], b"pop");
        }));
        result.expect("pooled buffer roundtrip timed out");
        assert_eq!(pool.available(), 4, "every slice returned to the pool");
    }

    /// Vectored IO over pooled slices: scatter/gather no longer means
    /// `Vec<Vec<u8>>`, so the buffer array can come from one slab.
    #[test]
    fn pooled_slices_go_through_vectored_io() {
        use crate::common::buffer::SlabPool;
        let rt = Runtime::new();
        let pool = SlabPool::new(64, 6).unwrap();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let server = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let server_addr = server.local_addr().unwrap();

            // Gather three pooled slices into one datagram.
            let mut parts = Vec::new();
            for chunk in [b"aa", b"bb", b"cc"] {
                let mut b = pool.acquire().unwrap();
                b[..2].copy_from_slice(chunk);
                b.set_len(2);
                parts.push(b);
            }
            let (w, parts) = client.send_to_vectored(parts, server_addr).await;
            assert_eq!(w.unwrap(), 6);
            drop(parts);

            // Scatter it back into two pooled slices of different sizes.
            let mut a = pool.acquire().unwrap();
            a.set_len(4);
            let landing = vec![a, pool.acquire().unwrap()];
            let (r, landing) = server.recv_from_vectored(landing).await;
            let (n, _from) = r.unwrap();
            let mut got = Vec::new();
            for b in &landing {
                got.extend_from_slice(b);
            }
            got.truncate(n);
            assert_eq!(got, b"aabbcc");
        }));
        result.expect("pooled vectored roundtrip timed out");
        assert_eq!(pool.available(), 6, "every slice returned to the pool");
    }

    #[test]
    fn udp_vectored_roundtrip() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let server = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let server_addr = server.local_addr().unwrap();
            let client_port = client.local_addr().unwrap().port();

            // Three buffers gather into ONE datagram...
            let (w, _) = client
                .send_to_vectored(
                    vec![b"ab".to_vec(), b"cd".to_vec(), b"ef".to_vec()],
                    server_addr,
                )
                .await;
            assert_eq!(w.unwrap(), 6);

            // ...and scatter into two on receive.
            let (r, bufs) = server.recv_from_vectored(vec![vec![0u8; 3], vec![0u8; 8]]).await;
            let (n, from) = r.unwrap();
            let mut got = Vec::new();
            for b in &bufs {
                got.extend_from_slice(b);
            }
            got.truncate(n);
            assert_eq!(got, b"abcdef");
            assert_eq!(from.port(), client_port);
        }));
        result.expect("udp vectored roundtrip timed out");
    }

    #[test]
    fn tcp_peek_does_not_consume() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();
            let server = rt.spawn(async move {
                let s = listener.accept().await.unwrap();
                // Peek first, then read: both must see the same bytes.
                let (n1, b1) = s.peek(vec![0u8; 8]).await;
                let peeked = b1[..n1.unwrap()].to_vec();
                let (n2, b2) = s.read(vec![0u8; 8]).await;
                let read = b2[..n2.unwrap()].to_vec();
                (peeked, read)
            });
            let client = TcpStream::connect(&rt, addr, &SocketOptions::new()).await.unwrap();
            client.write(b"peekme".to_vec()).await.0.unwrap();
            let (peeked, read) = server.await.unwrap();
            assert_eq!(peeked, b"peekme");
            assert_eq!(read, b"peekme", "peek must not consume");
        }));
        result.expect("peek test timed out");
    }

    #[test]
    fn tcp_vectored_roundtrip() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();

            // Server scatter-reads one stream into two differently-sized buffers.
            let server = rt.spawn(async move {
                let s = listener.accept().await.unwrap();
                let (n, bufs) = s.read_vectored(vec![vec![0u8; 4], vec![0u8; 5]]).await;
                let n = n.unwrap();
                let mut got = Vec::new();
                for b in &bufs {
                    got.extend_from_slice(b);
                }
                got.truncate(n);
                got
            });

            // Client gather-writes three buffers as one contiguous send.
            let client = TcpStream::connect(&rt, addr, &SocketOptions::new()).await.unwrap();
            let (w, bufs) = client
                .write_vectored(vec![b"abc".to_vec(), b"def".to_vec(), b"ghi".to_vec()])
                .await;
            assert_eq!(w.unwrap(), 9);
            assert_eq!(bufs.len(), 3, "buffers are handed back");

            assert_eq!(server.await.unwrap(), b"abcdefghi");
        }));
        result.expect("vectored roundtrip timed out");
    }

    #[test]
    fn cancel_storm_accept_and_read() {
        let rt = Runtime::new();
        block_on(timeout(&rt, Duration::from_secs(30), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            )
            .unwrap();
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
            let client = TcpStream::connect(&rt, addr, &SocketOptions::new()).await.unwrap();
            let server = server_task.await.unwrap();
            for _ in 0..1000 {
                drop(client.read(vec![0u8; 1024]));
                drop(server.read(vec![0u8; 1024]));
            }
        }))
        .expect("cancel storm timed out");
        drop(rt);
        {
            for _ in 0..500 {
                if super::super::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(super::super::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed), 0, "{}: {}", "op blocks leaked", crate::common::overlapped::live_ops_report());
        }
    }

    const SIO_KEEPALIVE_VALS: u32 = 0x9800_0004;
    const SIO_ADDRESS_LIST_QUERY: u32 = 0x4800_0016;
    const SIO_ADDRESS_LIST_CHANGE: u32 = 0x2800_0017;

    /// An ioctl that finishes at once still reports its output's length:
    /// the completion packet says 0 bytes, and only `lpcbBytesReturned`
    /// has the real count.
    #[test]
    fn ioctl_output_survives_an_immediate_completion() {
        let rt = Runtime::new();
        let udp = UdpSocket::bind(
            &rt,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            &SocketOptions::new(),
        )
            .unwrap();
        // Spare capacity, not zeroed bytes: the length ends where the output
        // does.
        let (n, _, out) = block_on(udp.ioctl(SIO_ADDRESS_LIST_QUERY, Vec::new(), Vec::with_capacity(4096)));
        assert_eq!(n.unwrap(), out.len());
        // SOCKET_ADDRESS_LIST: an i32 count, then the entries.
        assert!(out.len() > 4, "no output from SIO_ADDRESS_LIST_QUERY: {out:?}");
        let count = i32::from_ne_bytes(out[..4].try_into().unwrap());
        assert!(count >= 1, "no addresses listed");
    }

    /// An input-only ioctl on a connected stream.
    #[test]
    fn ioctl_sets_keepalive() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener =
                TcpListener::bind(
                    &rt,
                    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                    &SocketOptions::new(),
                )
                    .unwrap();
            let addr = listener.local_addr().unwrap();
            let server = rt.spawn(async move { listener.accept().await.unwrap() });
            let client = TcpStream::connect(&rt, addr, &SocketOptions::new()).await.unwrap();
            let _server = server.await.unwrap();
            let vals: Vec<u8> = [1u32, 10_000, 1_000].iter().flat_map(|v| v.to_ne_bytes()).collect();
            client.ioctl(SIO_KEEPALIVE_VALS, vals, Vec::new()).await.0
        }));
        assert_eq!(result.expect("timed out").unwrap(), 0);
    }

    /// A code that waits (here for the machine's addresses to change) stays
    /// pending, and dropping its future cancels it without leaking the op.
    #[test]
    fn a_waiting_ioctl_is_cancelled_by_drop() {
        let rt = Runtime::new();
        let udp = UdpSocket::bind(
            &rt,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            &SocketOptions::new(),
        )
            .unwrap();
        let waited = block_on(timeout(
            &rt,
            Duration::from_millis(200),
            udp.ioctl(SIO_ADDRESS_LIST_CHANGE, Vec::new(), Vec::new()),
        ));
        assert!(waited.is_err(), "SIO_ADDRESS_LIST_CHANGE completed: {:?}", waited.map(|r| r.0));
        drop(udp);
        for _ in 0..500 {
            if super::super::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(super::super::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed), 0, "{}: {}", "the cancelled ioctl leaked its op", crate::common::overlapped::live_ops_report());
    }

    /// Borrowed buffers over TCP and connected UDP, and a dropped TCP read
    /// that leaves late data for the next read instead of writing it into a
    /// buffer the caller has taken back.
    /// Sockets the standard library made, taken over: a listener, its accepted
    /// stream, a client, and a UDP pair.
    /// More clients connect at once than any sane backlog is short of:
    /// with the `winsock.h` `SOMAXCONN` of 5 the sixth SYN is dropped and
    /// retransmitted 500 ms later. Every connect must finish well inside
    /// that.
    #[test]
    fn listen_backlog_takes_a_burst_of_connects() {
        let rt = Runtime::new();
        let listener = TcpListener::bind(&rt, "127.0.0.1:0".parse().unwrap(), &SocketOptions::default()).unwrap();
        let addr = listener.local_addr().unwrap();
        let clients: Vec<_> = (0..32)
            .map(|_| {
                std::thread::spawn(move || {
                    let t = std::time::Instant::now();
                    let s = std::net::TcpStream::connect(addr).unwrap();
                    (t.elapsed(), s)
                })
            })
            .collect();
        let mut slowest = std::time::Duration::ZERO;
        let mut keep = Vec::new();
        for c in clients {
            let (took, s) = c.join().unwrap();
            slowest = slowest.max(took);
            keep.push(s);
        }
        assert!(slowest < std::time::Duration::from_millis(300), "slowest connect {slowest:?}");
        for _ in 0..32 {
            block_on(listener.accept()).unwrap();
        }
    }

    #[test]
    fn std_sockets_are_taken_over() {
        use std::io::Write;
        let rt = Runtime::new();
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = std_listener.local_addr().unwrap();
        let listener = TcpListener::from_std(&rt, std_listener).unwrap();
        let mut std_client = std::net::TcpStream::connect(addr).unwrap();
        let server = block_on(listener.accept()).unwrap();
        std_client.write_all(b"from std").unwrap();
        let (n, buf) = block_on(server.read(vec![0u8; 16]));
        assert_eq!(&buf[..n.unwrap()], b"from std");
        let client = TcpStream::from_std(&rt, std_client).unwrap();
        block_on(client.write(b"back".to_vec())).0.unwrap();
        let (n, buf) = block_on(server.read(vec![0u8; 16]));
        assert_eq!(&buf[..n.unwrap()], b"back");
        // The borrowed handle is tied to the stream's lifetime.
        let _borrowed = std::os::windows::io::AsSocket::as_socket(&client);

        let a = UdpSocket::from_std(&rt, std::net::UdpSocket::bind("127.0.0.1:0").unwrap()).unwrap();
        let b = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let (n, _) = block_on(a.send_to(b"dgram".to_vec(), b.local_addr().unwrap()));
        assert_eq!(n.unwrap(), 5);
        let mut got = [0u8; 8];
        let (n, _) = b.recv_from(&mut got).unwrap();
        assert_eq!(&got[..n], b"dgram");
    }

    #[test]
    fn unchecked_socket_ops_take_borrowed_buffers() {
        let rt = Runtime::new();
        let listener =
            TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &SocketOptions::new(),
            ).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || block_on(listener.accept()).unwrap());
        let client = block_on(TcpStream::connect(&rt, addr, &SocketOptions::new())).unwrap();
        let server = server.join().unwrap();

        let mut buf = [0u8; 16];
        // SAFETY: dropped by the timeout, never forgotten.
        let timed_out =
            block_on(timeout(&rt, Duration::from_millis(100), unsafe { server.read_unchecked(&mut buf[..], crate::tp::io_event().unwrap()) }))
                .is_err();
        assert!(timed_out);
        buf.fill(0xAA);
        let hello = *b"hello";
        // SAFETY: awaited to completion.
        block_on(unsafe { client.write_unchecked(&hello[..], crate::tp::io_event().unwrap()) }).0.unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(buf.iter().all(|&b| b == 0xAA), "the kernel wrote after the drop");
        // SAFETY: awaited to completion.
        let (n, _) = block_on(unsafe { server.read_unchecked(&mut buf[..], crate::tp::io_event().unwrap()) });
        assert_eq!(&buf[..n.unwrap()], b"hello");

        let a = UdpSocket::bind(
            &rt,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            &SocketOptions::new(),
        ).unwrap();
        let b = UdpSocket::bind(
            &rt,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            &SocketOptions::new(),
        ).unwrap();
        a.connect(b.local_addr().unwrap()).unwrap();
        b.connect(a.local_addr().unwrap()).unwrap();
        let mut got = [0u8; 8];
        // SAFETY: both awaited to completion.
        block_on(unsafe { a.send_unchecked(&b"dgram"[..], crate::tp::io_event().unwrap()) }).0.unwrap();
        let (n, _) = block_on(unsafe { b.recv_unchecked(&mut got[..], crate::tp::io_event().unwrap()) });
        assert_eq!(&got[..n.unwrap()], b"dgram");
    }
