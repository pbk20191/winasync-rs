//! What a comio stream operation costs: `WriteAsync`/`ReadAsync` round
//! trips over a loopback named pipe, through the WinRT surface, awaited as
//! futures. Per round trip that is four operations -- two `IoAsyncOp`
//! objects, two op blocks, and whatever each keeps alive for the kernel --
//! so the figure is the per-operation overhead of this layer rather than of
//! the pipe.
//!
//! Run it explicitly; it is `#[ignore]` so it never slows the suite:
//!
//! ```text
//! cargo test --features comio -- --ignored --nocapture comio_pipe_round_trips
//! ```

#![cfg(test)]

use super::buffer::{buffer_ptr, new_buffer};
use crate::common::block_on;
use std::time::Instant;
use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
use windows_core::Interface;

const PAYLOAD: u32 = 64;
const ROUND_TRIPS: usize = 20_000;

#[test]
#[ignore = "measurement, not a correctness check; run with --ignored"]
fn comio_pipe_round_trips() {
    let (server, client) = super::stream::tests::pipe_pair();
    let (s_in, s_out): (IInputStream, IOutputStream) = (server.cast().unwrap(), server.cast().unwrap());
    let (c_in, c_out): (IInputStream, IOutputStream) = (client.cast().unwrap(), client.cast().unwrap());

    let payload: IBuffer = new_buffer(PAYLOAD).unwrap().cast().unwrap();
    unsafe { std::ptr::write_bytes(buffer_ptr(&payload).unwrap(), 0x5a, PAYLOAD as usize) };
    payload.SetLength(PAYLOAD).unwrap();
    let scratch_s: IBuffer = new_buffer(PAYLOAD).unwrap().cast().unwrap();
    let scratch_c: IBuffer = new_buffer(PAYLOAD).unwrap().cast().unwrap();

    // Both ends driven from one task, one operation at a time: the
    // interfaces are not `Send`, and what is measured is each operation's
    // own cost, not overlap between them.
    let mut warm = 0usize;
    let mut started = Instant::now();
    block_on(async {
        for i in 0..ROUND_TRIPS {
            if i == ROUND_TRIPS / 10 {
                warm = i;
                started = Instant::now();
            }
            c_out.WriteAsync(&payload).unwrap().into_future().await.unwrap();
            let got = s_in
                .ReadAsync(&scratch_s, PAYLOAD, InputStreamOptions(0))
                .unwrap()
                .into_future()
                .await
                .unwrap();
            s_out.WriteAsync(&got).unwrap().into_future().await.unwrap();
            let got = c_in
                .ReadAsync(&scratch_c, PAYLOAD, InputStreamOptions(0))
                .unwrap()
                .into_future()
                .await
                .unwrap();
            assert_eq!(got.Length().unwrap(), PAYLOAD);
        }
    });
    let elapsed = started.elapsed();
    let measured = ROUND_TRIPS - warm;
    println!(
        "comio named-pipe round trips ({PAYLOAD}-byte, {measured} measured): {:.0}/s, {:.2} us each, {:.2} us per operation",
        measured as f64 / elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1e6 / measured as f64,
        elapsed.as_secs_f64() * 1e6 / (measured * 4) as f64,
    );
}
