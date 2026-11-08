//! COM components exposing named pipe, AF_UNIX and Hyper-V socket transports
//! through the same WinRT stream interfaces (`IInputStream`/`IOutputStream`/
//! `IAsyncOperationWithProgress`) as `StreamSocket`, backed by Win32 overlapped
//! IO on the default thread-pool callback environment (`pcbe = NULL`).
//!
//! All three are transports WinRT itself does not expose: `StreamSocket` has no
//! named-pipe, AF_UNIX or AF_HYPERV support, and `HostName` cannot even name
//! those endpoints.
#![allow(dead_code)]

pub mod asyncop;
pub mod buffer;
pub mod hvsock;
pub mod listener;
pub mod pipe;
pub mod stream;
pub mod tpio;
pub mod unix;
pub mod waitop;

// Public transport surface. Nothing outside `comio`'s own tests consumes
// these yet (this binary has no other caller wired up), hence
// `#[allow(unused_imports)]` alongside the module's existing
// `#![allow(dead_code)]` — both are re-export/API-shape warnings, not bugs.
#[allow(unused_imports)]
pub use pipe::{NamedPipeClient, NamedPipeListener};
// The WinRT-native, `StreamSocketListener`-shaped surface (`listener.rs`):
// one shared interface, one concrete type per transport, so each exposes only
// the bind its addressing supports.
#[allow(unused_imports)]
pub use listener::{
    HyperVConnectionListener, IConnectionListener, IConnectionReceivedArgs,
    NamedPipeConnectionListener, UnixConnectionListener,
};
// `stream::from_reg` takes/returns crate-private types (`Arc<tpio::IoReg>`),
// so it stays `pub(crate)` here rather than `pub` (the latter would be a hard
// "private type in public interface" error, E0446) — this re-export just
// gives it a shorter crate-internal path (`comio::from_reg` instead of
// `comio::stream::from_reg`) for `pipe`/`unix`'s own use.
#[allow(unused_imports)]
pub(crate) use stream::from_reg;
#[allow(unused_imports)]
pub use unix::{UnixStreamListener, UnixStreamSocket};
#[allow(unused_imports)]
pub use hvsock::{HvStreamListener, HvStreamSocket};

pub(crate) use crate::common::check::check;

#[cfg(test)]
mod tests {
    use super::pipe::{NamedPipeClient, NamedPipeListener};
    use crate::comio::buffer::new_buffer;
    use crate::common::block_on;
    use std::future::IntoFuture;
    use std::time::Duration;
    use windows::Storage::Streams::{
        DataReader, DataWriter, IBuffer, IInputStream, IOutputStream, InputStreamOptions,
    };
    use windows_core::{Interface, HSTRING};
    use windows_future::IAsyncInfo;

    /// (a) `DataReader`/`DataWriter` interop over our stream: proves
    /// `OverlappedDuplexStream` is a real, drop-in WinRT `IInputStream`/
    /// `IOutputStream` — not just something our own `ReadAsync`/`WriteAsync`
    /// tests happen to exercise correctly. Reuses the exact named-pipe
    /// server/client setup from `pipe::tests::named_pipe_echo`: the server
    /// runs on its own `std::thread` (accepting is a blocking `.await` until
    /// a client connects), the client connects from the test's main thread.
    #[test]
    fn datareader_interop() {
        let name = format!(r"\\.\pipe\comio_datareader_{}", std::process::id());
        let sname = name.clone();
        let server = std::thread::spawn(move || {
            let listener = NamedPipeListener::create(&sname);
            let stream = block_on(listener.accept()).unwrap();
            let input: IInputStream = stream.cast().unwrap();
            // `DataReader::CreateDataReader` wraps any `IInputStream` —
            // including ours — exactly as it would wrap a real
            // `StreamSocket`'s.
            let reader = DataReader::CreateDataReader(&input).unwrap();
            let n = block_on(reader.LoadAsync(5).unwrap().into_future()).unwrap();
            reader.ReadString(n).unwrap()
        });
        // Give the server thread time to reach `accept()`'s pending
        // `ConnectNamedPipe` before the client dials in (same margin as
        // `named_pipe_echo`).
        std::thread::sleep(Duration::from_millis(100));
        let client = block_on(NamedPipeClient::connect(&name)).unwrap();
        let output: IOutputStream = client.cast().unwrap();
        let writer = DataWriter::CreateDataWriter(&output).unwrap();

        // Default `UnicodeEncoding` is UTF-8, so for an ASCII string the byte
        // count `WriteString` returns doubles as the "code unit count"
        // `LoadAsync`/`ReadString` expect on the other side.
        let written = writer.WriteString(&HSTRING::from("hello")).unwrap();
        assert_eq!(written, 5);
        let stored = block_on(writer.StoreAsync().unwrap().into_future()).unwrap();
        assert_eq!(stored, 5);

        let got = server.join().unwrap();
        assert_eq!(got.to_string(), "hello");
    }

    /// (b) Cancel storm: issuing (and immediately cancelling) many reads that
    /// can never complete on their own — nothing ever writes to the pipe —
    /// must not leak `comio::tpio`'s operation blocks. Each `ReadAsync` call
    /// issues its overlapped `ReadFile` synchronously before returning the
    /// `IAsyncOperationWithProgress`, so by the time it returns the op is
    /// already in flight; `IAsyncInfo::Cancel` (routed to `OpHandle::cancel`
    /// -> `CancelIoEx`, see `asyncop.rs`/`tpio.rs`) is what actually lets it
    /// complete promptly, since — unlike a plain Rust future — dropping a
    /// WinRT async-operation object does not implicitly cancel it (confirmed
    /// against `windows-future`'s `AsyncFuture`, which has no `Drop` impl).
    /// Without an explicit `Cancel`, the read would simply stay pending
    /// forever (no writer, no timeout), and since the pending op's own
    /// `Arc<IoReg>` clone keeps the pipe handle open, closing our end alone
    /// could never force-abort it either.
    #[test]
    fn cancel_storm() {
        let name = format!(r"\\.\pipe\comio_cancel_storm_{}", std::process::id());
        let sname = name.clone();
        // `IInspectable` (and thus the accepted stream) isn't `Send`, so it
        // can't cross the `JoinHandle::join()` boundary — keep it alive on
        // the server thread itself (parked on `done_rx`) instead of handing
        // it back, so the pipe stays genuinely open (no writer, no EOF)
        // for the whole cancel storm below.
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let listener = NamedPipeListener::create(&sname);
            let _stream = block_on(listener.accept()).unwrap();
            let _ = done_rx.recv();
        });
        std::thread::sleep(Duration::from_millis(100));
        let client = block_on(NamedPipeClient::connect(&name)).unwrap();
        let input: IInputStream = client.cast().unwrap();

        for _ in 0..1000 {
            let buf = new_buffer(16).unwrap();
            let op = input
                .ReadAsync(&buf.cast::<IBuffer>().unwrap(), 16, InputStreamOptions(0))
                .unwrap();
            let info: IAsyncInfo = op.cast().unwrap();
            info.Cancel().unwrap();
            drop(op);
        }

        drop(client);
        let _ = done_tx.send(());
        server.join().unwrap();

        #[cfg(debug_assertions)]
        {
            use std::sync::atomic::Ordering;
            // Cancellation completions are delivered asynchronously by the
            // thread pool; poll for `LIVE_OPS` to settle rather than
            // asserting immediately, mirroring
            // `tpio.rs::cancel_in_flight_read_does_not_leak` and
            // `unix.rs`'s cancel-storm tests. `LIVE_OPS` is process-global,
            // so this also waits out unrelated concurrently running tests.
            for _ in 0..500 {
                if crate::comio::tpio::LIVE_OPS.load(Ordering::Relaxed) == 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                crate::comio::tpio::LIVE_OPS.load(Ordering::Relaxed),
                0,
                "op blocks leaked after read cancel storm"
            );
        }
    }
}
