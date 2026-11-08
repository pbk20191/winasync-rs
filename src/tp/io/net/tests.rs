    use super::super::super::{block_on, timeout, Runtime};
    use super::*;
    use std::time::Duration;

    #[test]
    fn tcp_echo_self_connect() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
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
            let client = TcpStream::connect(&rt, addr).await.unwrap();
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

            let client = TcpStream::connect(&rt, addr).await.unwrap();
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
            )
            .unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
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
            )
            .unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0)),
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
            )
            .unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
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

            let client = TcpStream::connect(&rt, addr).await.unwrap();
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
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();

            // Server scatters an in-memory header + the whole file in one call.
            let hdr = header.clone();
            let server = rt.spawn(async move {
                let stream = listener.accept().await.unwrap();
                let pk = TransmitPackets::new().bytes(hdr).file(&file, 0, 0);
                let n = stream.transmit_packets(pk, TransmitFlags::default()).await.unwrap();
                stream.shutdown_send().unwrap();
                n
            });

            let client = TcpStream::connect(&rt, addr).await.unwrap();
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
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();
            let client = TcpStream::connect(&rt, addr).await.unwrap();
            let s1 = listener.accept().await.unwrap();
            // DisconnectEx is graceful: it completes once the peer has closed too,
            // so close the server side first (the connection is finished).
            drop(s1);
            client.disconnect(true).await.unwrap();
        }));
        result.expect("disconnect alone timed out");
    }

    #[test]
    fn disconnect_reconnect_client() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();

            // First connection on a fresh client socket.
            let client = TcpStream::connect(&rt, addr).await.unwrap();
            let s1 = listener.accept().await.unwrap();
            client.write(b"one".to_vec()).await.0.unwrap();
            let (n, b) = s1.read(vec![0u8; 8]).await;
            assert_eq!(&b[..n.unwrap()], b"one");

            // Finish the connection (close the peer), then disconnect + reuse the
            // SAME client handle and reconnect — no new socket()/bind()/register.
            drop(s1);
            client.disconnect(true).await.unwrap();
            client.reconnect(addr).await.unwrap();

            let s2 = listener.accept().await.unwrap();
            client.write(b"two".to_vec()).await.0.unwrap();
            let (n, b) = s2.read(vec![0u8; 8]).await;
            assert_eq!(&b[..n.unwrap()], b"two");
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
            )
            .unwrap();
            let addr = listener.local_addr().unwrap();

            let c1 = TcpStream::connect(&rt, addr).await.unwrap();
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

            let c2 = TcpStream::connect(&rt, addr).await.unwrap();
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
            )
            .unwrap();
            server.set_recv_pktinfo(true).unwrap();
            let client = UdpSocket::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
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
    fn tcp_peek_does_not_consume() {
        let rt = Runtime::new();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let listener = TcpListener::bind(
                &rt,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
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
            let client = TcpStream::connect(&rt, addr).await.unwrap();
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
            let client = TcpStream::connect(&rt, addr).await.unwrap();
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
