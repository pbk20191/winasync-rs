//! Named-pipe transport: `NamedPipeListener`/`NamedPipeClient` produce
//! `OverlappedDuplexStream`s (via [`stream::from_reg`]) over Win32 named
//! pipes, the first end-to-end transport built on the `comio` overlapped-IO
//! engine.

use super::stream;
use super::tpio::{self, IoReg, Issue, OpHandle};
use super::waitop::await_io;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, WaitNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
    PIPE_WAIT,
};
use windows_core::{Error, Result, HRESULT, HSTRING};

/// Server (listener) side of a named pipe. Each `accept()` creates a fresh
/// overlapped pipe instance and waits for a client to connect to it.
pub struct NamedPipeListener {
    name: HSTRING,
}

impl NamedPipeListener {
    pub fn create(name: &str) -> Self {
        Self {
            name: HSTRING::from(name),
        }
    }

    /// Creates a new instance of the named pipe and asynchronously waits for
    /// a client to connect, returning the connected instance as a duplex
    /// WinRT stream.
    pub async fn accept(&self) -> Result<windows_core::IInspectable> {
        let handle = unsafe {
            CreateNamedPipeW(
                &self.name,
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0),
                PIPE_TYPE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                4096,
                0,
                None,
            )
        };
        // `CreateNamedPipeW` returns a bare `HANDLE`, not a `Result` —
        // failure is `INVALID_HANDLE_VALUE` plus `GetLastError()`.
        if handle.is_invalid() {
            return Err(Error::from_thread());
        }
        let reg = match IoReg::register(handle, false) {
            Ok(reg) => reg,
            Err(e) => {
                // `IoReg::register` only takes ownership of `handle` on
                // success; on error the caller still owns (and must close)
                // it.
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err(e);
            }
        };

        await_io(&reg, move |ovl| unsafe {
            // On an overlapped handle, `ConnectNamedPipe` returning `Ok(())`
            // or `ERROR_IO_PENDING` both mean a completion packet is (or
            // will be) queued to the thread pool's IOCP — same as
            // `ReadFile`/`WriteFile` elsewhere in `comio`.
            //
            // `ERROR_PIPE_CONNECTED` is the one documented exception: it
            // means a client connected between `CreateNamedPipeW` and this
            // call, so the "connect" is already done — but *no* completion
            // packet is ever queued for it, unlike every other overlapped
            // call on this handle. We still classify it as `Issue::Failed`
            // so `tpio::submit` balances the `StartThreadpoolIo` it already
            // armed (via `CancelThreadpoolIo`) and invokes the completion
            // callback synchronously right here, instead of leaving the
            // thread pool waiting for a callback that will never arrive.
            // `await_io`'s completion mapping then treats
            // `ERROR_PIPE_CONNECTED` as success rather than as an error, so
            // callers of `accept` see a single, ordinary "connected" outcome
            // regardless of which path produced it.
            match ConnectNamedPipe(handle, Some(ovl)) {
                Ok(()) => Issue::SyncSuccess,
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                Err(e) => Issue::Failed(e.code()),
            }
        })
        .await?;

        Ok(stream::from_reg(reg))
    }
}

/// Callback-based counterpart to [`NamedPipeListener::accept`], used by
/// `listener.rs`'s self-re-arming, executor-free accept chain via
/// [`tpio::submit`] directly instead of the `Future`-based `accept()`/
/// `await_io`. Creates a fresh pipe instance, registers it, and issues the
/// overlapped `ConnectNamedPipe` — the same flags/classification as
/// `accept()` above, including the `ERROR_PIPE_CONNECTED` special case.
///
/// `on_result` is invoked exactly once: normally on a thread-pool worker
/// thread when the kernel completion arrives, but — per `tpio::submit`'s
/// documented `Issue::Failed` behavior — it may instead run synchronously,
/// on the calling thread, *before this function returns*, whenever
/// `ConnectNamedPipe` reports `ERROR_PIPE_CONNECTED` (a client connected
/// between `CreateNamedPipeW` and this call) or any other synchronous
/// failure. Callers that re-arm from within `on_result` must account for
/// that reentrancy.
///
/// A synchronous failure to create/register the pipe instance itself is
/// returned as `Err` directly, without ever invoking `on_result` — mirroring
/// `accept()`'s own synchronous-failure behavior — so a bad pipe name (for
/// instance) surfaces to the immediate caller rather than through the
/// callback.
pub(crate) fn arm_accept(
    name: &str,
    on_result: Box<dyn FnOnce(Result<super::listener::Accepted>) + Send>,
) -> Result<OpHandle> {
    let wname = HSTRING::from(name);
    let handle = unsafe {
        CreateNamedPipeW(
            &wname,
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0),
            PIPE_TYPE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            4096,
            4096,
            0,
            None,
        )
    };
    if handle.is_invalid() {
        return Err(Error::from_thread());
    }
    let reg = match IoReg::register(handle, false) {
        Ok(reg) => reg,
        Err(e) => {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(e);
        }
    };
    let reg_for_complete = reg.clone();
    let op = unsafe {
        tpio::submit(
            &reg,
            Box::new(move |ioresult: u32, _bytes: usize| {
                let res = if ioresult == 0 || ioresult == ERROR_PIPE_CONNECTED.0 {
                    Ok(stream::from_reg(reg_for_complete))
                } else {
                    Err(Error::from_hresult(HRESULT::from_win32(ioresult)))
                };
                on_result(res.map(super::listener::Accepted::unnamed));
            }),
            move |ovl| match ConnectNamedPipe(handle, Some(ovl)) {
                Ok(()) => Issue::SyncSuccess,
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                Err(e) => Issue::Failed(e.code()),
            },
        )
    };
    Ok(op)
}

/// Client side of a named pipe.
pub struct NamedPipeClient;

impl NamedPipeClient {
    /// Opens the client end of an existing named pipe. If every server
    /// instance is momentarily busy (`ERROR_PIPE_BUSY`), waits via
    /// `WaitNamedPipeW` for one to free up and retries once.
    pub async fn connect(name: &str) -> Result<windows_core::IInspectable> {
        let wname = HSTRING::from(name);
        let handle = match Self::open(&wname) {
            Ok(h) => h,
            Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() => {
                // A single retry is enough for tests/typical use; a caller
                // needing a longer wait or a bespoke retry policy would
                // build on `open`/`WaitNamedPipeW` directly.
                unsafe { WaitNamedPipeW(&wname, 5000) }.ok()?;
                Self::open(&wname)?
            }
            Err(e) => return Err(e),
        };
        let reg = match IoReg::register(handle, false) {
            Ok(reg) => reg,
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err(e);
            }
        };
        Ok(stream::from_reg(reg))
    }

    /// Opening the client end is a plain synchronous `CreateFileW` — only
    /// the server's `accept` needs an overlapped `ConnectNamedPipe`.
    fn open(wname: &HSTRING) -> Result<HANDLE> {
        unsafe {
            CreateFileW(
                wname,
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::buffer::{buffer_ptr, new_buffer};
    use crate::common::block_on;
    use std::future::IntoFuture;
    use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
    use windows_core::Interface;

    #[test]
    fn named_pipe_echo() {
        let name = format!(r"\\.\pipe\comio_pipe_echo_{}", std::process::id());
        let sname = name.clone();
        let server = std::thread::spawn(move || {
            let listener = NamedPipeListener::create(&sname);
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
        let client = block_on(NamedPipeClient::connect(&name)).unwrap();
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
        let listener = NamedPipeListener::create(&name);
        // Force the server's pipe instance to exist first so the client's
        // `CreateFileW` can succeed, then connect the client *before*
        // `accept` is ever called — guaranteeing `ConnectNamedPipe` will
        // observe `ERROR_PIPE_CONNECTED` rather than pending.
        let handle = unsafe {
            CreateNamedPipeW(
                &listener.name,
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
        let client = block_on(NamedPipeClient::connect(&name)).unwrap();
        // Now build a listener bound to the SAME already-connected instance
        // by registering the handle we created above directly, bypassing
        // `CreateNamedPipeW` inside `accept` (which would create a *new*,
        // unconnected instance). This isolates exactly the
        // `ConnectNamedPipe` => `ERROR_PIPE_CONNECTED` path that `accept`'s
        // `await_io` closure must special-case.
        let reg = IoReg::register(handle, false).unwrap();
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
}
