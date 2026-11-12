use super::pipe::{NamedPipeConnector, NamedPipeListener, NamedPipeOptions};
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
/// `TransportStream` is a real, drop-in WinRT `IInputStream`/
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
        let listener = NamedPipeListener::create(&sname, &NamedPipeOptions::new()).unwrap();
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
    let client = block_on(NamedPipeConnector::connect(&name)).unwrap();
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
        let listener = NamedPipeListener::create(&sname, &NamedPipeOptions::new()).unwrap();
        let _stream = block_on(listener.accept()).unwrap();
        let _ = done_rx.recv();
    });
    std::thread::sleep(Duration::from_millis(100));
    let client = block_on(NamedPipeConnector::connect(&name)).unwrap();
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
        assert_eq!(crate::comio::tpio::LIVE_OPS.load(Ordering::Relaxed), 0, "{}: {}", "op blocks leaked after read cancel storm", crate::common::overlapped::live_ops_report());
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
/// None of it is hollow any more: the generated class types in
/// [`bindings`] carry a `RuntimeName`, so `GetRuntimeClassName` is
/// checked against the string the metadata defines rather than against
/// `""`; and `GetIids`, which upstream's macro hard-codes empty with no
/// seam, is answered by the forked macro in `tools/implement`. The name
/// stays, for the history it points at.
///
/// A real WinRT class goes through the same calls as a control. Without
/// it, "empty" could just as well mean the test is calling the vtable
/// wrong; with it, the empty answer is demonstrably the object's.
#[test]
fn iinspectable_is_shaped_but_hollow() {
    use windows_core::{IInspectable, IInspectable_Vtbl};

    /// `GetIids` has no safe wrapper in `windows-core`, so go through the
    /// vtable. Returns the count, and the IIDs (sorted).
    fn get_iids(what: &str, object: &IInspectable) -> (u32, Vec<windows_core::GUID>) {
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
        let mut iids = Vec::new();
        if !values.is_null() {
            iids.extend_from_slice(unsafe { std::slice::from_raw_parts(values, count as usize) });
            // The array is the caller's to release.
            unsafe {
                windows::Win32::System::Com::CoTaskMemFree(Some(
                    values as *const core::ffi::c_void,
                ))
            };
        }
        iids.sort_by_key(|g| g.to_u128());
        (count, iids)
    }

    let ours: IInspectable = crate::comio::NamedPipeConnectionListener::new()
        .cast()
        .expect("listener QI IInspectable");

    // Since the forked macro (`tools/implement`): every interface named in
    // the listener's `#[implement]`, each once.
    let (count, iids) = get_iids("listener", &ours);
    use crate::comio::bindings::Pbk20191::WinAsync::Comio as c;
    use windows_core::Interface;
    for (what, iid) in [
        ("ITransport", c::ITransport::IID),
        ("INamedPipeBind", c::INamedPipeBind::IID),
        ("IListenerSecurity", c::IListenerSecurity::IID),
        ("INamedPipeListenerSettings", c::INamedPipeListenerSettings::IID),
        ("IClosable", windows::Foundation::IClosable::IID),
    ] {
        assert!(iids.contains(&iid), "GetIids left out {what}: {iids:?}");
    }
    assert_eq!(count as usize, iids.len());
    let mut unique = iids.clone();
    unique.dedup();
    assert_eq!(unique.len(), iids.len(), "GetIids repeated an IID: {iids:?}");
    // Not merely non-empty: the exact name, and the same constant the
    // `activation::CLASSES` keys activation off, so a rename cannot land
    // in one world and not the other.
    let name = ours.GetRuntimeClassName().expect("GetRuntimeClassName");
    assert_eq!(
        name.to_string(),
        <crate::comio::bindings::Pbk20191::WinAsync::Comio::NamedPipeConnectionListener as windows_core::RuntimeName>::NAME,
        "GetRuntimeClassName said {name:?}"
    );
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

    let (count, iids) = get_iids("control", &theirs);
    assert_ne!(count, 0, "control: a real WinRT class reported no IIDs");
    assert!(!iids.is_empty(), "control: a real WinRT class returned no array");
    assert!(
        !theirs
            .GetRuntimeClassName()
            .expect("control: GetRuntimeClassName")
            .is_empty(),
        "control: a real WinRT class reported no name"
    );
}

/// `#[implement(..., Export = prefix)]` (the forked macro's second change):
/// the exported getter hands out the class's vtable, and a call through it
/// with the object's own interface pointer reaches the object. This is the
/// table a statically linked caller uses so that LTO can inline the method
/// (`tools/lto-probe`); here only its identity is checked.
#[winasync_implement::implement(windows::Foundation::IClosable, Export = winasync_test_Closable)]
struct ExportedClosable;

static EXPORTED_CLOSED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl windows::Foundation::IClosable_Impl for ExportedClosable_Impl {
    fn Close(&self) -> windows_core::Result<()> {
        EXPORTED_CLOSED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn export_hands_out_the_vtable_the_object_carries() {
    use windows::Foundation::IClosable;
    let obj: IClosable = ExportedClosable.into();
    let table = winasync_test_Closable_IClosable_vtable();
    assert!(!table.is_null());
    assert_eq!(table, winasync_test_Closable_IClosable_vtable(), "the getter is stable");

    // Through the exported table, with the interface pointer the vtable
    // call would take.
    let hr = unsafe { ((*table).Close)(obj.as_raw()) };
    assert!(hr.is_ok(), "{hr:?}");
    assert_eq!(EXPORTED_CLOSED.load(std::sync::atomic::Ordering::SeqCst), 1);

    // And the ordinary way reaches the same implementation.
    obj.Close().unwrap();
    assert_eq!(EXPORTED_CLOSED.load(std::sync::atomic::Ordering::SeqCst), 2);
}
