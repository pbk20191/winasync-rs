//! COM components exposing named pipe, AF_UNIX and Hyper-V socket transports
//! through the same WinRT stream interfaces (`IInputStream`/`IOutputStream`/
//! `IAsyncOperationWithProgress`) as `StreamSocket`, backed by Win32 overlapped
//! IO on the default thread-pool callback environment (`pcbe = NULL`).
//!
//! All three are transports WinRT itself does not expose: `StreamSocket` has no
//! named-pipe, AF_UNIX or AF_HYPERV support, and `HostName` cannot even name
//! those endpoints.
//!
//! # What `IInspectable` answers here, and what it does not
//!
//! Every object in this module is reachable as an `IInspectable`: the WinRT
//! interfaces they implement — `IClosable`, `IInputStream`, `IOutputStream`,
//! `IAsyncOperationWithProgress` — all derive from it, so `QueryInterface`
//! succeeds. The ABI really is the WinRT one. What the three `IInspectable`
//! methods *say*, though, is fixed by `windows-rs` and is not the truth about
//! these objects:
//!
//! * `GetIids` reports an empty set. `IInspectable_Vtbl::new` installs a free
//!   function that writes `count = 0`, `values = null` and returns `S_OK`. It
//!   takes no type parameter at all, so there is no point at which an object's
//!   real interface chains could reach it.
//! * `GetRuntimeClassName` reports the empty string. It returns
//!   `RuntimeName::NAME` of the **first** interface named in `#[implement]`,
//!   and `#[interface]` generates `impl RuntimeName for X {}` — taking the
//!   default, `""` — for every interface this crate declares.
//! * `GetTrustLevel` is the one that reaches our code, and the macro hardcodes
//!   it to `0`, `BaseTrust`.
//!
//! All three return `S_OK`, so a consumer doing WinRT type discovery is told
//! "no interfaces, no name, base trust" rather than "I cannot say" — a
//! confident wrong answer instead of an honest refusal. None of it is
//! reachable from here: `#[implement]` parses only a list of interfaces, and
//! the identity vtable it builds is an associated constant on the generated
//! `_Impl` type.
//!
//! That is a gap in what these objects *are*, not a defect to paper over.
//! They are not WinRT runtime classes; they are COM classes that implement
//! WinRT interfaces, activated by CLSID ([`activation`]) and described by a
//! type library ([`typelib`]). A runtime class name is a promise that the
//! name resolves through WinRT metadata, and nothing here publishes any — so
//! inventing a non-empty name would make the claim more *credible* without
//! making it true, which is worse than the empty string. Answering these
//! honestly is part of shipping a `.winmd`, not a step that can usefully come
//! before it. `tests::iinspectable_is_shaped_but_hollow` pins the current
//! answers, so a `windows-rs` upgrade that changes any of them shows up as a
//! failure rather than a silent shift.
#![allow(dead_code)]

pub mod activation;
pub mod asyncop;
pub mod buffer;
pub mod connector;
pub mod connpt;
pub mod dispatch;
pub mod hvsock;
pub mod listener;
pub mod pipe;
pub mod stream;
pub mod typelib;
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
    HyperVConnectionListener, IConnectionListener, IConnectionReceivedArgs, IHyperVListenerBind,
    INamedPipeListenerBind, IUnixListenerBind, NamedPipeConnectionListener, UnixConnectionListener,
};
// The client half's interfaces. The connectors themselves are the existing
// client types — `NamedPipeClient`, `UnixStreamSocket`, `HvStreamSocket` —
// which now implement these, rather than three more types wrapping them.
#[allow(unused_imports)]
pub use connector::{
    IHyperVConnector, INamedPipeConnector, IStreamConnector, IUnixConnector,
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

    /// Pins what `IInspectable` actually answers on our objects, for the
    /// reasons the module docs give.
    ///
    /// This is a characterisation test: it asserts what is, not what should
    /// be, so that the day a `windows-rs` upgrade makes any of it honest —
    /// or changes it some other way — that is a failure to read rather than
    /// a silent shift under a design decision which cited it.
    ///
    /// A real WinRT class goes through the same calls as a control. Without
    /// it, "empty" could just as well mean the test is calling the vtable
    /// wrong; with it, the empty answers are demonstrably the object's.
    #[test]
    fn iinspectable_is_shaped_but_hollow() {
        use windows_core::{IInspectable, IInspectable_Vtbl};

        /// `GetIids` has no safe wrapper in `windows-core`, so go through the
        /// vtable. Returns the count, and whether the array came back null.
        fn get_iids(what: &str, object: &IInspectable) -> (u32, bool) {
            // Seeded with values the callee must overwrite, so a vtable entry
            // that ignored its out-params could not pass for "empty".
            let mut count = 0xDEADu32;
            let mut values = core::ptr::dangling_mut::<windows_core::GUID>();
            let hr = unsafe {
                let raw = object.as_raw();
                let vtbl = *(raw as *const *const IInspectable_Vtbl);
                ((*vtbl).GetIids)(raw, &mut count, &mut values)
            };
            assert!(hr.is_ok(), "{what}: GetIids failed with {hr:?}");
            let null = values.is_null();
            if !null {
                // The array is the caller's to release.
                unsafe {
                    windows::Win32::System::Com::CoTaskMemFree(Some(
                        values as *const core::ffi::c_void,
                    ))
                };
            }
            (count, null)
        }

        let ours: IInspectable = crate::comio::NamedPipeConnectionListener::new()
            .cast()
            .expect("listener QI IInspectable");

        let (count, values_null) = get_iids("listener", &ours);
        assert_eq!(count, 0, "GetIids reported interfaces");
        assert!(values_null, "GetIids returned a non-null array");
        let name = ours.GetRuntimeClassName().expect("GetRuntimeClassName");
        assert!(name.is_empty(), "GetRuntimeClassName said {name:?}");
        assert_eq!(
            ours.GetTrustLevel().expect("GetTrustLevel"),
            0,
            "GetTrustLevel was not BaseTrust"
        );

        // The control: a class the Windows Runtime itself publishes, reached
        // through the identical code path.
        let theirs: IInspectable = crate::comio::buffer::new_buffer(1)
            .expect("Buffer::Create")
            .cast()
            .expect("Buffer QI IInspectable");

        let (count, values_null) = get_iids("control", &theirs);
        assert_ne!(count, 0, "control: a real WinRT class reported no IIDs");
        assert!(!values_null, "control: a real WinRT class returned no array");
        assert!(
            !theirs
                .GetRuntimeClassName()
                .expect("control: GetRuntimeClassName")
                .is_empty(),
            "control: a real WinRT class reported no name"
        );
    }
}
