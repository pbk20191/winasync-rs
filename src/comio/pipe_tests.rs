use super::*;
use crate::comio::buffer::{buffer_ptr, new_buffer};
use crate::common::block_on;
use std::future::IntoFuture;
use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_core::Interface;

#[test]
fn named_pipe_echo() {
    let name = format!(r"\\.\pipe\comio_pipe_echo_{}", std::process::id());
    let sname = name.clone();
    let server = std::thread::spawn(move || {
        let listener = NamedPipeListener::create(&sname, &NamedPipeOptions::new()).unwrap();
        let stream = block_on(listener.accept()).unwrap();
        let input: IInputStream = stream.cast().unwrap();
        let output: IOutputStream = stream.cast().unwrap();
        let rbuf = new_buffer(16).unwrap();
        let got: IBuffer = block_on(
            input
                .ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), 16, InputStreamOptions(0))
                .unwrap()
                .into_future(),
        )
        .unwrap();
        let n = got.Length().unwrap();
        block_on(output.WriteAsync(&got).unwrap().into_future()).unwrap();
        n
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    let client = block_on(NamedPipeConnector::connect(&name)).unwrap();
    let c_out: IOutputStream = client.cast().unwrap();
    let c_in: IInputStream = client.cast().unwrap();
    let wbuf = new_buffer(5).unwrap();
    unsafe {
        buffer_ptr(&wbuf.clone().into())
            .unwrap()
            .copy_from_nonoverlapping(b"hello".as_ptr(), 5);
    }
    wbuf.SetLength(5).unwrap();
    block_on(c_out.WriteAsync(&wbuf.cast::<IBuffer>().unwrap()).unwrap().into_future()).unwrap();
    let rbuf = new_buffer(5).unwrap();
    let echoed: IBuffer = block_on(
        c_in.ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), 5, InputStreamOptions(0))
            .unwrap()
            .into_future(),
    )
    .unwrap();
    unsafe {
        assert_eq!(
            std::slice::from_raw_parts(buffer_ptr(&echoed).unwrap(), 5),
            b"hello"
        );
    }
    assert_eq!(server.join().unwrap(), 5);
}

/// Regression test for the trickiest edge case: if a client has already
/// connected to a pipe instance before the server issues
/// `ConnectNamedPipe` on it, `ConnectNamedPipe` reports
/// `ERROR_PIPE_CONNECTED` synchronously instead of `ERROR_IO_PENDING` —
/// and, per its documented behavior, queues no completion packet for
/// it. `await_io` must still resolve successfully (rather than hanging
/// forever waiting for a completion that will never arrive), while
/// still balancing the `StartThreadpoolIo` that was armed before the
/// call.
#[test]
fn accept_when_already_connected() {
    let name = format!(
        r"\\.\pipe\comio_pipe_preconnected_{}",
        std::process::id()
    );
    let listener = NamedPipeListener::create(&name, &NamedPipeOptions::new()).unwrap();
    // Force the server's pipe instance to exist first so the client's
    // `CreateFileW` can succeed, then connect the client *before*
    // `accept` is ever called — guaranteeing `ConnectNamedPipe` will
    // observe `ERROR_PIPE_CONNECTED` rather than pending.
    let handle = unsafe {
        CreateNamedPipeW(
            listener.server.name(),
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0),
            PIPE_TYPE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            4096,
            4096,
            0,
            None,
        )
    };
    assert!(!handle.is_invalid());
    let client = block_on(NamedPipeConnector::connect(&name)).unwrap();
    // Now build a listener bound to the SAME already-connected instance
    // by registering the handle we created above directly, bypassing
    // `CreateNamedPipeW` inside `accept` (which would create a *new*,
    // unconnected instance). This isolates exactly the
    // `ConnectNamedPipe` => `ERROR_PIPE_CONNECTED` path that `accept`'s
    // `await_io` closure must special-case.
    let reg = IoReg::register(unsafe { crate::comio::tpio::IoHandle::from_handle(handle) }).unwrap();
    let server_stream = block_on(await_io(&reg, |ovl| unsafe {
        match ConnectNamedPipe(handle, Some(ovl)) {
            Ok(()) => Issue::SyncSuccess,
            Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
            Err(e) => Issue::Failed(e.code()),
        }
    }))
    .unwrap();
    let _ = server_stream;

    let c_out: IOutputStream = client.cast().unwrap();
    block_on(
        c_out
            .WriteAsync(&new_buffer(0).unwrap().cast::<IBuffer>().unwrap())
            .unwrap()
            .into_future(),
    )
    .unwrap();
}
