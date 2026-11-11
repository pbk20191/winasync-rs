//! What a comio stream operation costs: `WriteAsync`/`ReadAsync` round
//! trips over a loopback named pipe, through the WinRT surface, awaited as
//! futures. Per round trip that is four operations -- two `IoAsyncOp`
//! objects, two op blocks, and whatever each keeps alive for the kernel --
//! so the figure is the per-operation overhead of this layer rather than of
//! the pipe.
//!
//! Two rows: as the streams are (a synchronous success completed on the
//! issuing thread), and with every completion through the port, which is
//! what they did before and what the listeners' accepts still do. The
//! difference is the pool hand-off, and it is most of an operation.
//!
//! Run it explicitly; it is `#[ignore]` so it never slows the suite:
//!
//! ```text
//! cargo test --features comio,tp -- --ignored --nocapture comio_pipe_round_trips
//! ```

#![cfg(test)]

use super::buffer::{buffer_ptr, new_buffer};
use crate::common::block_on;
use std::time::Instant;
use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
use windows_core::Interface;

const PAYLOAD: u32 = 64;
const ROUND_TRIPS: usize = 20_000;

/// A connected overlapped pipe pair as comio streams, optionally with
/// `FILE_SKIP_COMPLETION_PORT_ON_SUCCESS` on both ends, so an operation
/// the kernel finishes synchronously is completed on the issuing thread
/// instead of through the pool.
fn pipe_pair(skip: bool) -> (
    super::bindings::Pbk20191::WinAsync::Comio::ITransportStream,
    super::bindings::Pbk20191::WinAsync::Comio::ITransportStream,
) {
    use super::stream::{from_reg_with, Peer};
    use super::tpio::{IoHandle, IoReg};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ,
        FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_WAIT};
    let name = format!(r"\\.\pipe\comio_bench_{}_{skip}", std::process::id());
    let wname = windows_core::HSTRING::from(&name);
    let server = unsafe {
        CreateNamedPipeW(
            &wname,
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0),
            PIPE_TYPE_BYTE | PIPE_WAIT,
            1,
            4096,
            4096,
            0,
            None,
        )
    };
    assert!(!server.is_invalid());
    let client = unsafe {
        CreateFileW(
            &wname,
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            None,
        )
    }
    .unwrap();
    let s = IoReg::register(unsafe { IoHandle::from_handle(server) }).unwrap();
    let c = IoReg::register(unsafe { IoHandle::from_handle(client) }).unwrap();
    (
        from_reg_with(s, Peer::Opaque, false, skip),
        from_reg_with(c, Peer::Opaque, false, skip),
    )
}

/// What every stream does now: a synchronous success is completed on the
/// issuing thread. On loopback nearly every write, and every read that
/// finds data, finishes synchronously, so the pool thread -- and the hop
/// back from it -- drops out of the path.
#[test]
#[ignore = "measurement, not a correctness check; run with --ignored"]
fn comio_pipe_round_trips() {
    round_trips(true);
}

/// The baseline: every completion through the port and a pool thread, as
/// every stream did before 2026-11-12.
#[test]
#[ignore = "measurement, not a correctness check; run with --ignored"]
fn comio_pipe_round_trips_through_the_port() {
    round_trips(false);
}

fn round_trips(skip: bool) {
    let (server, client) = pipe_pair(skip);
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
        "comio named-pipe round trips ({PAYLOAD}-byte, {measured} measured, skip={skip}): {:.0}/s, {:.2} us each, {:.2} us per operation",
        measured as f64 / elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1e6 / measured as f64,
        elapsed.as_secs_f64() * 1e6 / (measured * 4) as f64,
    );
}
