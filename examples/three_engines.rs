//! Three engines, one connection, one buffer.
//!
//! A tiny "file chunk" server: a client asks for `offset len` and gets
//! those bytes of a file back. Each engine does the one thing it is for:
//!
//! * **tp** -- the TCP handshake (`AcceptEx` / `ConnectEx`, which RIO
//!   lacks), the pumps for the other two engines, and the client side.
//! * **rio** -- the connection's data path. Buffers come from one slab
//!   registered with RIO once, so each send and receive names a
//!   descriptor the kernel already pinned.
//! * **ioring** -- the file reads, submitted in a batch and popped by the
//!   serving task itself, **straight into the RIO-registered slice**: the
//!   bytes the ring reads are the bytes RIO sends, with no copy between.
//!
//! The ring and its operations are `!Send`, so the serving task runs on
//! the main thread under `common::block_on`; the client is an ordinary
//! `Send` task on the pool. Both pumps live on the same `tp` pool and only
//! *wake*: the serving task pops the ring, the RIO pump drains the
//! completion queue.
//!
//! ```text
//! cargo run --example three_engines --features tp,rio,ioring
//! ```

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use winasync::common::block_on;
use winasync::ioring::{self, IoRing};
use winasync::rio::{RioBufferPool, RioCq, RioTcpAcceptor, RqConfig};
use winasync::tp;

const CHUNK: usize = 4096;
const FILE_LEN: usize = 1 << 20;

fn main() -> windows_core::Result<()> {
    let rt = tp::Runtime::new();

    // The file: a megabyte whose every byte is its own offset, so a chunk
    // is checkable by the client.
    let path = std::env::temp_dir().join(format!("three_engines_{}.bin", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    std::fs::write(&path, (0..FILE_LEN).map(|i| i as u8).collect::<Vec<u8>>()).unwrap();

    // tp: the listener whose accepted sockets can take RIO queues.
    let acceptor = RioTcpAcceptor::bind(&rt, SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)))?;
    let addr = acceptor.local_addr()?;

    // rio: one completion queue pumped from tp's pool, one registered slab.
    let cq = RioCq::new(&rt, 256)?;
    let pool = RioBufferPool::new(CHUNK, 16)?;

    // ioring: one ring, pumped from tp's pool, owned by this thread.
    let ring = IoRing::with_runtime(&rt, 32)?;
    let file = ioring::File::open(&path)?;

    // The client, on the pool: plain tp TCP. Asks for eight chunks, checks
    // each byte, and reports how many bytes came back.
    let client = rt.spawn({
        let rt = rt.clone();
        async move {
            let stream = tp::TcpStream::connect(&rt, addr, &tp::SocketOptions::new()).await?;
            let mut total = 0usize;
            for i in 0..8u64 {
                let (offset, len) = (i * 100_003 % FILE_LEN as u64, CHUNK);
                stream.write(format!("{offset} {len}\n").into_bytes()).await.0?;
                let mut got = Vec::with_capacity(len);
                while got.len() < len {
                    let (n, buf) = stream.read(vec![0u8; len - got.len()]).await;
                    let n = n?;
                    if n == 0 {
                        break;
                    }
                    got.extend_from_slice(&buf[..n]);
                }
                assert!(
                    got.iter().enumerate().all(|(k, &b)| b == (offset as usize + k) as u8),
                    "chunk {i} came back wrong"
                );
                total += got.len();
            }
            // Closing the socket ends the server's loop.
            drop(stream);
            Ok::<_, windows_core::Error>(total)
        }
    });

    // The server, on this thread: accept with tp's AcceptEx, then serve
    // over RIO, reading with the ring into RIO's own slices.
    let served = block_on(async {
        let stream = acceptor.accept(&cq, RqConfig::default()).await?;
        let mut served = 0usize;
        loop {
            // A request line arrives in a registered slice.
            let (n, req) = stream.recv(pool.acquire().expect("a free slice"))?.await;
            let n = n?;
            if n == 0 {
                break; // the client closed
            }
            let text = String::from_utf8_lossy(&req[..n]);
            let mut words = text.split_whitespace();
            let offset: u64 = words.next().and_then(|w| w.parse().ok()).expect("offset");
            let len: usize = words.next().and_then(|w| w.parse().ok()).expect("len");
            drop(req);

            // The ring reads the chunk straight into a RIO slice (a
            // `RioSlice` is an `IoBufMut` like any other), and RIO sends
            // that same slice. One pinning, no copy.
            let mut slice = pool.acquire().expect("a free slice");
            slice.reset_len();
            let (read, mut slice) = ring.read(&file, slice, offset).await;
            let read = read?.min(len);
            slice.set_len(read);
            let (sent, _slice) = stream.send(slice)?.await;
            served += sent?;
        }
        Ok::<_, windows_core::Error>(served)
    })?;

    let received = block_on(client).expect("client task").expect("client");
    println!("[three_engines] served {served} bytes over RIO from {CHUNK}-byte ring reads; client verified {received}");
    assert_eq!(served, received);

    drop((file, ring, cq, pool));
    let _ = std::fs::remove_file(&path);
    drop(rt);
    Ok(())
}
