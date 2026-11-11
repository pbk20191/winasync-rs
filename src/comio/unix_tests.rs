use super::*;
use crate::comio::buffer::{buffer_ptr, new_buffer};
use crate::common::block_on;
use std::future::{Future, IntoFuture};
use std::task::{Context, RawWaker, RawWakerVTable, Waker};
use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
use windows_core::Interface;

/// A `Waker` that does nothing when woken — sufficient for tests that
/// only need one `poll()` to run an async fn's body up to its first
/// `.await` (an async fn's body runs none of its code until first
/// polled) and don't care about being woken again afterward.
fn noop_waker() -> Waker {
    fn clone(_: *const ()) -> RawWaker {
        raw()
    }
    fn no_op(_: *const ()) {}
    fn raw() -> RawWaker {
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, no_op, no_op, no_op);
        RawWaker::new(std::ptr::null(), &VTABLE)
    }
    unsafe { Waker::from_raw(raw()) }
}

/// Polls `fut` exactly once (enough to issue the overlapped op the async
/// fn starts with, since it runs no code until first polled) and then
/// drops it immediately, mid-flight.
fn issue_then_drop<F: Future>(fut: F) {
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let mut fut = Box::pin(fut);
    let _ = fut.as_mut().poll(&mut cx);
    drop(fut);
}

fn assert_no_live_ops(msg: &str) {
    use std::sync::atomic::Ordering;
    for _ in 0..500 {
        if crate::comio::tpio::LIVE_OPS.load(Ordering::Relaxed) == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(crate::comio::tpio::LIVE_OPS.load(Ordering::Relaxed), 0, "{msg}: {}", crate::common::overlapped::live_ops_report());
}

#[test]
fn af_unix_echo() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("comio_unix_{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let p = path.to_string_lossy().into_owned();
    let sp = p.clone();
    let server = std::thread::spawn(move || {
        let l = UnixStreamListener::bind(&sp, None).unwrap();
        let s = block_on(l.accept()).unwrap();
        let input: IInputStream = s.cast().unwrap();
        let output: IOutputStream = s.cast().unwrap();
        let rbuf = new_buffer(16).unwrap();
        let got: IBuffer = block_on(
            input
                .ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), 16, InputStreamOptions(0))
                .unwrap()
                .into_future(),
        )
        .unwrap();
        block_on(output.WriteAsync(&got).unwrap().into_future()).unwrap();
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    let c = block_on(UnixConnector::connect(&p)).unwrap();
    let c_out: IOutputStream = c.cast().unwrap();
    let c_in: IInputStream = c.cast().unwrap();
    let wbuf = new_buffer(4).unwrap();
    unsafe {
        buffer_ptr(&wbuf.clone().into())
            .unwrap()
            .copy_from_nonoverlapping(b"ping".as_ptr(), 4);
    }
    wbuf.SetLength(4).unwrap();
    block_on(
        c_out
            .WriteAsync(&wbuf.cast::<IBuffer>().unwrap())
            .unwrap()
            .into_future(),
    )
    .unwrap();
    let rbuf = new_buffer(4).unwrap();
    let echoed: IBuffer = block_on(
        c_in.ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0))
            .unwrap()
            .into_future(),
    )
    .unwrap();
    unsafe {
        assert_eq!(
            std::slice::from_raw_parts(buffer_ptr(&echoed).unwrap(), 4),
            b"ping"
        );
    }
    server.join().unwrap();
    let _ = std::fs::remove_file(&path);
}

/// Exercises the `WSARecv` path (`stream::ReadAsync`'s socket branch)
/// with a non-trivial, multi-chunk payload, rather than `af_unix_echo`'s
/// single 4-byte round trip. This is realistic coverage for the
/// `WSARecv`-`lpFlags` write-after-free fix in `IoAsyncOp::start_read`
/// (`flags` is now a kernel-gated `Box<u32>` keep-alive instead of a
/// stack local): every chunk here drives a real overlapped `WSARecv`
/// whose provider writes back through that pointer at completion.
#[test]
fn af_unix_echo_large_payload() {
    const TOTAL: usize = 5000;
    const CHUNK: usize = 1024;

    let dir = std::env::temp_dir();
    let path = dir.join(format!("comio_unix_large_{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let p = path.to_string_lossy().into_owned();
    let sp = p.clone();

    let payload: Vec<u8> = (0..TOTAL).map(|i| (i % 256) as u8).collect();
    let payload_for_server = payload.clone();

    let server = std::thread::spawn(move || {
        let l = UnixStreamListener::bind(&sp, None).unwrap();
        let s = block_on(l.accept()).unwrap();
        let input: IInputStream = s.cast().unwrap();
        let output: IOutputStream = s.cast().unwrap();

        // Read exactly TOTAL bytes. A single overlapped recv is not
        // guaranteed to fill a several-KB request in one completion, so
        // loop `ReadAsync` (each iteration a fresh `WSARecv`) until the
        // whole payload has arrived.
        let mut received = Vec::with_capacity(TOTAL);
        while received.len() < TOTAL {
            let want = (TOTAL - received.len()).min(CHUNK) as u32;
            let rbuf = new_buffer(want).unwrap();
            let got: IBuffer = block_on(
                input
                    .ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), want, InputStreamOptions(0))
                    .unwrap()
                    .into_future(),
            )
            .unwrap();
            let len = got.Length().unwrap() as usize;
            assert!(len > 0, "peer closed early");
            unsafe {
                let ptr = buffer_ptr(&got).unwrap();
                received.extend_from_slice(std::slice::from_raw_parts(ptr, len));
            }
        }
        assert_eq!(received, payload_for_server);

        // Echo it back, also in chunks, exercising the `WSASend` side
        // with the same multi-chunk shape.
        let mut sent = 0usize;
        while sent < TOTAL {
            let chunk_len = (TOTAL - sent).min(CHUNK);
            let wbuf = new_buffer(chunk_len as u32).unwrap();
            let wib: IBuffer = wbuf.clone().into();
            unsafe {
                buffer_ptr(&wib).unwrap().copy_from_nonoverlapping(
                    received[sent..sent + chunk_len].as_ptr(),
                    chunk_len,
                );
            }
            wbuf.SetLength(chunk_len as u32).unwrap();
            let wrote = block_on(output.WriteAsync(&wib).unwrap().into_future()).unwrap();
            sent += wrote as usize;
        }
    });

    std::thread::sleep(std::time::Duration::from_millis(100));
    let c = block_on(UnixConnector::connect(&p)).unwrap();
    let c_out: IOutputStream = c.cast().unwrap();
    let c_in: IInputStream = c.cast().unwrap();

    let mut sent = 0usize;
    while sent < TOTAL {
        let chunk_len = (TOTAL - sent).min(CHUNK);
        let wbuf = new_buffer(chunk_len as u32).unwrap();
        let wib: IBuffer = wbuf.clone().into();
        unsafe {
            buffer_ptr(&wib)
                .unwrap()
                .copy_from_nonoverlapping(payload[sent..sent + chunk_len].as_ptr(), chunk_len);
        }
        wbuf.SetLength(chunk_len as u32).unwrap();
        let wrote = block_on(c_out.WriteAsync(&wib).unwrap().into_future()).unwrap();
        sent += wrote as usize;
    }

    let mut echoed = Vec::with_capacity(TOTAL);
    while echoed.len() < TOTAL {
        let want = (TOTAL - echoed.len()).min(CHUNK) as u32;
        let rbuf = new_buffer(want).unwrap();
        let got: IBuffer = block_on(
            c_in.ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), want, InputStreamOptions(0))
                .unwrap()
                .into_future(),
        )
        .unwrap();
        let len = got.Length().unwrap() as usize;
        assert!(len > 0, "server closed early");
        unsafe {
            let ptr = buffer_ptr(&got).unwrap();
            echoed.extend_from_slice(std::slice::from_raw_parts(ptr, len));
        }
    }
    assert_eq!(echoed, payload);

    server.join().unwrap();
    let _ = std::fs::remove_file(&path);
}

/// Regression test for the write-after-free this change fixes:
/// `AcceptEx`'s output address buffer used to be a plain local in
/// `accept()`'s async fn frame, freed as soon as the future dropped —
/// but `await_io`'s (now `await_io_owning`'s) `Drop` only issues
/// `CancelIoEx` and returns immediately, without waiting for the real
/// `ERROR_OPERATION_ABORTED` completion, so the kernel could still write
/// into that freed memory afterward. Repeatedly starts `accept()` with
/// no client ever connecting, polls it exactly once (to actually issue
/// the `AcceptEx`), and drops it mid-flight — the same shape as a
/// `select!`/timeout abandoning the future. With the fix, the address
/// buffer now lives on the kernel-completion-gated `Op` block (via
/// `await_io_owning`) instead of the async-fn frame, so it survives
/// until the real completion regardless of the future's `Drop`. This
/// asserts the ops don't leak (`LIVE_OPS` settles back to 0), which
/// mirrors `tp/io/net.rs::cancel_storm_accept_and_read` and
/// `tpio.rs::cancel_in_flight_read_does_not_leak`'s leak-detection
/// pattern; it does not (and cannot, without a race detector) directly
/// observe the UAF itself, so the actual safety guarantee rests on the
/// `await_io_owning` contract (payload moved into the kernel-gated
/// `on_complete` closure) being upheld here, not on this test alone.
#[test]
fn accept_cancel_storm_does_not_leak() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("comio_unix_accept_cancel_{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let p = path.to_string_lossy().into_owned();
    let listener = UnixStreamListener::bind(&p, None).unwrap();

    for _ in 0..500 {
        issue_then_drop(listener.accept());
    }
    drop(listener);

    assert_no_live_ops("op blocks leaked after accept() cancel storm");
    let _ = std::fs::remove_file(&path);
}

/// As `accept_cancel_storm_does_not_leak`, for `connect()`'s
/// `ConnectEx` input `SOCKADDR_UN` (lower certainty than the accept
/// buffer per the task brief — it's a provider *input*, so may be read
/// synchronously rather than deferred — but gated identically for
/// consistency with the module's invariant, so exercised identically
/// here). No `accept()` is ever called on `listener`, so connections
/// simply queue in the listen backlog rather than completing, giving
/// `connect()`'s op a real window to be cancelled mid-flight.
#[test]
fn connect_cancel_storm_does_not_leak() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("comio_unix_connect_cancel_{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let p = path.to_string_lossy().into_owned();
    let listener = UnixStreamListener::bind(&p, None).unwrap();

    for _ in 0..200 {
        issue_then_drop(UnixConnector::connect(&p));
    }
    drop(listener);

    assert_no_live_ops("op blocks leaked after connect() cancel storm");
    let _ = std::fs::remove_file(&path);
}
