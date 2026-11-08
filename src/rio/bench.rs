//! A crossover measurement: where RIO overtakes the overlapped engine, on
//! *this* machine.
//!
//! This is not here to show RIO winning. On loopback it mostly will not —
//! that is RIO's documented worst case, because the overlapped path can
//! complete an operation inline (`IoReg::set_skip_completion_on_success`)
//! while RIO must round-trip every result through a queue. The point is to
//! record the shape of the curve so the decision to use either engine rests
//! on a number rather than on a reputation.
//!
//! What is varied is **queue depth**: how many receives are outstanding, and
//! therefore how many completions one drain can amortise. That is the axis
//! the published comparisons disagree along — RIO measured 25x *worse* at
//! depth 1 on one connection, and ahead once concurrency climbs.
//!
//! Run it explicitly; it is `#[ignore]` so it never slows the suite:
//!
//! ```text
//! cargo test --features rio -- --ignored --nocapture rio_vs_overlapped
//! ```

#![cfg(test)]

use super::buffer::RioBufferPool;
use super::cq::RioCq;
use super::rq::RqConfig;
use super::socket::RioUdpSocket;
use crate::common::block_on;
use crate::tp::io::net::UdpSocket;
use crate::tp::Runtime;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};

const PAYLOAD: usize = 64;
const ITERATIONS: usize = 2_000;

fn v4() -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
}

/// Datagrams per second for the overlapped engine at a given queue depth.
fn overlapped_rate(rt: &Runtime, depth: usize) -> f64 {
    let server = UdpSocket::bind(rt, v4()).unwrap();
    let client = UdpSocket::bind(rt, v4()).unwrap();
    let addr = server.local_addr().unwrap();
    // The engine's own fast path, so the comparison is against its best.
    let _ = server.set_skip_completion_on_success();
    let _ = client.set_skip_completion_on_success();

    // Recycled buffers. RIO hands slices back from a registered pool, so
    // allocating a Vec per operation here would measure the allocator as
    // much as the engine.
    let mut free_recv: Vec<Vec<u8>> = (0..depth.max(1)).map(|_| vec![0u8; 2048]).collect();
    let mut free_send: Vec<Vec<u8>> = (0..depth.max(1)).map(|_| vec![7u8; PAYLOAD]).collect();

    let start = Instant::now();
    let mut done = 0usize;
    block_on(async {
        while done < ITERATIONS {
            let batch = depth.min(ITERATIONS - done);
            let mut recvs = Vec::with_capacity(batch);
            for _ in 0..batch {
                recvs.push(server.recv_from(free_recv.pop().unwrap()));
            }
            // Post every send before awaiting any, matching what the RIO
            // side does. Awaiting each send in turn would serialize this
            // engine against a batched one and measure the test, not the
            // engines.
            let mut sends = Vec::with_capacity(batch);
            for _ in 0..batch {
                sends.push(client.send_to(free_send.pop().unwrap(), addr));
            }
            for s in sends {
                let (r, buf) = s.await;
                r.unwrap();
                free_send.push(buf);
            }
            for r in recvs {
                let (_, buf) = r.await;
                free_recv.push(buf);
            }
            done += batch;
        }
    });
    rate(start.elapsed(), ITERATIONS)
}

/// Datagrams per second for RIO at a given queue depth.
fn rio_rate(rt: &Runtime, depth: usize) -> f64 {
    let outstanding = depth.max(1) as u32;
    // sum(max_outstanding) over both sockets must fit the completion queue.
    let cq = RioCq::new(rt, (outstanding + 1) * 4).unwrap();
    let pool = RioBufferPool::new(2048, (outstanding * 2 + 4) as u32).unwrap();
    let rq = RqConfig {
        max_outstanding_recv: outstanding,
        max_outstanding_send: outstanding,
    };
    let server = RioUdpSocket::bind(&cq, v4(), rq).unwrap();
    let client = RioUdpSocket::bind(&cq, v4(), rq).unwrap();
    client.connect(server.local_addr().unwrap()).unwrap();

    let start = Instant::now();
    let mut done = 0usize;
    block_on(async {
        while done < ITERATIONS {
            let batch = depth.min(ITERATIONS - done);
            let mut recvs = Vec::with_capacity(batch);
            for _ in 0..batch {
                recvs.push(server.recv(pool.acquire().unwrap()).unwrap());
            }
            let mut sends = Vec::with_capacity(batch);
            for _ in 0..batch {
                let mut s = pool.acquire().unwrap();
                s[..PAYLOAD].fill(7);
                s.set_len(PAYLOAD);
                // Defer, then release the batch in one crossing — the
                // configuration RIO is designed for.
                sends.push(client.send_deferred(s).unwrap());
            }
            client.commit_sends().unwrap();
            for s in sends {
                let _ = s.await;
            }
            for r in recvs {
                let _ = r.await;
            }
            done += batch;
        }
    });
    rate(start.elapsed(), ITERATIONS)
}

fn rate(elapsed: Duration, n: usize) -> f64 {
    n as f64 / elapsed.as_secs_f64()
}

#[test]
#[ignore = "measurement, not a correctness check; run with --ignored"]
fn rio_vs_overlapped_crossover() {
    let rt = Runtime::new();
    println!();
    println!("{PAYLOAD}-byte datagrams, {ITERATIONS} per run, loopback");
    println!(
        "{:>6}  {:>14}  {:>14}  {:>8}",
        "depth", "overlapped/s", "rio/s", "ratio"
    );

    // Run-to-run spread on loopback is around 10%, so report medians.
    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    }

    let mut crossover = None;
    for depth in [1usize, 2, 4, 8, 16, 32, 64, 128] {
        // One warm run each, so neither engine pays first-call costs.
        let _ = overlapped_rate(&rt, depth);
        let _ = rio_rate(&rt, depth);

        let ov = median((0..3).map(|_| overlapped_rate(&rt, depth)).collect());
        let rio = median((0..3).map(|_| rio_rate(&rt, depth)).collect());
        let ratio = rio / ov;
        println!("{depth:>6}  {ov:>14.0}  {rio:>14.0}  {ratio:>8.2}x");
        if ratio >= 1.0 && crossover.is_none() {
            crossover = Some(depth);
        }
    }

    match crossover {
        Some(d) => println!("\nRIO reaches parity at queue depth {d} on this machine."),
        None => println!(
            "\nRIO did not reach parity at any depth measured here. Expected on \
             loopback: the overlapped path completes inline, RIO cannot."
        ),
    }
}
