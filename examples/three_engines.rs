//! Three engines, three transports, one buffer.
//!
//! A tiny "file chunk" server with a control plane and a data plane:
//! the client asks over a local socket for `offset len`, and the bytes
//! arrive as a UDP datagram. Each engine does the one thing it is for:
//!
//! * **tp** -- the control plane, an `AF_UNIX` stream (accept, read and
//!   write through `AcceptEx`/`WSARecv`/`WSASend` on the pool), the pumps
//!   for the other two engines, and the whole client.
//! * **rio** -- the data plane, UDP from one slab registered with RIO
//!   once, so each `RIOSendEx` names a descriptor the kernel already
//!   pinned.
//! * **ioring** -- the file reads, popped by the serving task itself,
//!   **straight into the RIO-registered slice**: the bytes the ring reads
//!   are the bytes RIO sends, with no copy between.
//!
//! The ring and its operations are `!Send`, so the serving task runs on
//! the main thread under `common::block_on`; the client is an ordinary
//! `Send` task on the pool. Both pumps live on the same `tp` pool and
//! only *wake*: the serving task pops the ring, the RIO pump drains the
//! completion queue.
//!
//! ```text
//! cargo run --example three_engines --features tp,rio,ioring
//! ```

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use winasync::common::block_on;
use winasync::ioring::{self, IoRing};
use winasync::rio::{RioBufferPool, RioCq, RioUdpSocket, RqConfig};
use winasync::tp;

const CHUNK: usize = 4096;
const FILE_LEN: usize = 1 << 20;

fn main() -> windows_core::Result<()> {
    let rt = tp::Runtime::new();
    let localhost = |port| SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));

    // The file: a megabyte whose every byte is its own offset, so a chunk
    // is checkable by the client.
    let tag = std::process::id();
    let path = std::env::temp_dir().join(format!("three_engines_{tag}.bin"));
    let path = path.to_string_lossy().into_owned();
    std::fs::write(&path, (0..FILE_LEN).map(|i| i as u8).collect::<Vec<u8>>()).unwrap();

    // tp: the control plane, an AF_UNIX listener on the pool.
    let sock_path = std::env::temp_dir().join(format!("three_engines_{tag}.sock"));
    let sock_path = sock_path.to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&sock_path);
    let control = tp::UnixListener::bind(&rt, &sock_path, None)?;

    // rio: the data plane. One completion queue pumped from tp's pool,
    // one registered slab, one UDP socket.
    let cq = RioCq::new(&rt, 256)?;
    let pool = RioBufferPool::new(CHUNK, 16)?;
    let udp = RioUdpSocket::bind(&cq, localhost(0), RqConfig::default())?;

    // ioring: one ring, pumped from tp's pool, owned by this thread.
    let ring = IoRing::with_runtime(&rt, 32)?;
    let file = ioring::File::open(&path)?;

    // The client, on the pool: asks over the unix socket, receives over
    // plain tp UDP, checks every byte.
    let client = rt.spawn({
        let rt = rt.clone();
        let sock_path = sock_path.clone();
        async move {
            let control = tp::UnixStream::connect(&rt, &sock_path).await?;
            let data = tp::UdpSocket::bind(&rt, localhost(0), &tp::SocketOptions::new())?;
            let port = data.local_addr()?.port();
            let mut total = 0usize;
            for i in 0..8u64 {
                let offset = i * 100_003 % FILE_LEN as u64;
                control.write(format!("{offset} {CHUNK} {port}\n").into_bytes()).await.0?;
                let (got, buf) = data.recv_from(vec![0u8; CHUNK]).await;
                let (n, _from) = got?;
                assert!(
                    buf[..n].iter().enumerate().all(|(k, &b)| b == (offset as usize + k) as u8),
                    "chunk {i} came back wrong"
                );
                total += n;
            }
            // Closing the control socket ends the server's loop.
            drop(control);
            Ok::<_, windows_core::Error>(total)
        }
    });

    // The server, on this thread: requests arrive on the unix stream (tp),
    // the chunk is read by the ring into a RIO slice, and RIO sends that
    // slice as a datagram.
    let served = block_on(async {
        let control = control.accept().await?;
        let mut served = 0usize;
        loop {
            let (n, line) = control.read(vec![0u8; 64]).await;
            let n = n?;
            if n == 0 {
                break; // the client closed
            }
            let text = String::from_utf8_lossy(&line[..n]);
            let mut words = text.split_whitespace().map(|w| w.parse::<u64>().expect("a number"));
            let (offset, len, port) = (words.next().unwrap(), words.next().unwrap() as usize, words.next().unwrap() as u16);

            // The ring reads the chunk straight into a RIO slice (a
            // `RioSlice` is an `IoBufMut` like any other), and RIO sends
            // that same slice. One pinning, no copy.
            let mut slice = pool.acquire().expect("a free slice");
            slice.reset_len();
            let (read, mut slice) = ring.read(&file, slice, offset).await;
            slice.set_len(read?.min(len));
            let (sent, _slice) = udp.send_to(slice, localhost(port))?.await;
            served += sent?.0;
        }
        Ok::<_, windows_core::Error>(served)
    })?;

    let received = block_on(client).expect("client task").expect("client");
    println!(
        "[three_engines] {served} bytes: requests over AF_UNIX (tp), reads by the ring into RIO slices, \
         datagrams by RIO; client verified {received}"
    );
    assert_eq!(served, received);

    drop((file, ring, udp, cq, pool, control));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&sock_path);
    drop(rt);
    Ok(())
}
