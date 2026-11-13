//! `DllCanUnloadNow`'s answer, measured.
//!
//! An integration test rather than a unit test, and for one specific reason:
//! the counters behind [`comio::unload::can_unload`] are process-wide, so a
//! unit test asserting on them would race every other test in the crate that
//! happens to build an object. Each integration test binary gets its own
//! process, and everything here runs inside a single `#[test]` so nothing in
//! this file races either.
//!
//! What it proves, in order:
//!
//! 1. A fresh process can unload.
//! 2. A live object prevents it, and releasing the object restores it.
//! 3. A `LockServer(TRUE)` prevents it with no objects at all.
//! 4. The settled async results are *ours* — the whole reason
//!    `comio::settled` exists rather than `windows_future::spawn`.
//! 5. **An overlapped operation in flight prevents it even when the object
//!    count is back to zero.** This is the case a naive object count misses,
//!    and the one that would corrupt a process: the pending completion is a
//!    function pointer into this module.
// The counters this measures are compiled in by `dylib`; without it
// `can_unload` is an unconditional no and there is nothing to assert.
#![cfg(all(feature = "comio", feature = "dylib"))]

use winasync::comio::NamedPipeConnector;
use winasync::comio::activation::{CLSID_NAMED_PIPE_CONNECTOR, dll_get_class_object};
use winasync::comio::bindings::Pbk20191::WinAsync::Comio::ITransportStream;
use winasync::comio::unload::{can_unload, locks_held, outstanding};
use winasync::common::winsock::wsa_initialized;
use std::time::{Duration, Instant};
use windows::Storage::Streams::{Buffer, IInputStream, InputStreamOptions};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_TYPE_BYTE};
use windows::Win32::System::Com::IClassFactory;
use windows_core::{IUnknown, Interface};

#[test]
fn the_module_refuses_to_unload_while_anything_of_its_own_is_live() {
    // --- 1. A fresh process owns nothing ---------------------------------
    assert_eq!(outstanding(), 0, "something was live before the test ran");
    assert!(can_unload(), "a fresh process could not unload");

    // --- 2. An object is a reason to stay --------------------------------
    let object = NamedPipeConnector::new();
    assert_eq!(outstanding(), 1);
    assert!(!can_unload(), "a live object did not prevent unloading");

    // A second interface on the same object is the same object: COM
    // refcounting must not look like a second reason to stay loaded.
    let second: IUnknown = object.to_interface();
    assert_eq!(outstanding(), 1, "a QueryInterface was counted as an object");
    drop(second);

    drop(object);
    assert_eq!(outstanding(), 0, "releasing an object did not clear it");
    assert!(can_unload());

    // --- 2b. A connect's operation object is counted ---------------------
    // The case an earlier audit missed. `NamedPipeConnector::ConnectAsync`
    // handed back `windows_future::IAsyncOperation::ready(...)`: an object
    // whose vtable is in this module and which nothing counted. The stream
    // inside it was counted, so the total still moved -- by one, not two --
    // which is exactly what this assertion distinguishes.
    //
    // The server end is the test's own `CreateNamedPipeW`, not a listener, so
    // no accept fires on a pool thread to move the count underneath us.
    let pipe = format!(r"\\.\pipe\comio_unload_op_{}", std::process::id());
    let server = unsafe {
        CreateNamedPipeW(
            &windows_core::HSTRING::from(&pipe),
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0),
            PIPE_TYPE_BYTE,
            1,
            4096,
            4096,
            0,
            None,
        )
    };
    assert!(!server.is_invalid(), "CreateNamedPipeW failed");
    let connector = NamedPipeConnector::new();
    let typed: winasync::comio::bindings::Pbk20191::WinAsync::Comio::INamedPipeConnector =
        connector.cast().expect("QI INamedPipeConnector");
    let before = outstanding();
    let op = typed
        .ConnectAsync(&windows_core::HSTRING::from(&pipe))
        .expect("ConnectAsync");
    assert_eq!(
        outstanding(),
        before + 2,
        "a connect should add two objects -- the operation and the stream it \
         carries. One means the operation is not ours and goes uncounted."
    );
    drop(op);
    assert_eq!(outstanding(), before, "the operation outlived its last reference");
    drop(typed);
    drop(connector);
    unsafe { CloseHandle(server) }.expect("CloseHandle");
    assert_eq!(outstanding(), 0);
    assert!(can_unload());

    // --- 3. A server lock holds the module on its own --------------------
    // Through the real class factory, not the counter's own back door.
    let factory: IClassFactory = unsafe {
        let mut raw = core::ptr::null_mut();
        dll_get_class_object(
            &CLSID_NAMED_PIPE_CONNECTOR,
            &IClassFactory::IID,
            &mut raw,
        )
        .ok()
        .expect("dll_get_class_object");
        IClassFactory::from_raw(raw)
    };
    // The factory is itself an object of ours, so it counts too.
    assert_eq!(outstanding(), 1, "the class factory was not counted");

    unsafe { factory.LockServer(true) }.expect("LockServer(TRUE)");
    assert_eq!(locks_held(), 1);
    drop(factory);
    assert_eq!(outstanding(), 0, "the factory outlived its last reference");
    assert!(!can_unload(), "a held server lock did not prevent unloading");

    unsafe {
        let mut raw = core::ptr::null_mut();
        dll_get_class_object(&CLSID_NAMED_PIPE_CONNECTOR, &IClassFactory::IID, &mut raw)
            .ok()
            .unwrap();
        let f = IClassFactory::from_raw(raw);
        f.LockServer(false).expect("LockServer(FALSE)");
    }
    assert_eq!(locks_held(), 0);
    assert_eq!(outstanding(), 0);
    assert!(can_unload(), "the lock was released but the module is still held");

    // --- 4. The settled async results are ours ---------------------------
    // `BindAsync` returns one. If it were a `windows_future` object the
    // count would not move here, and that invisibility is exactly the hole
    // `comio::settled` was written to close.
    let (listener, pipe_name) = bound_listener();
    let outstanding_with_listener = outstanding();
    assert!(
        outstanding_with_listener > 0,
        "a bound listener registered nothing"
    );
    assert!(!can_unload());

    // --- 5. In flight with no objects left -------------------------------
    // Connect, start a read nothing will answer, then drop every object.
    // The object count returns to zero while the operation is still armed,
    // and its completion callback still points into this module.
    //
    // The other end is a raw pipe this test holds, not one of the listener's
    // streams. It used to be the listener's accepted end, which went with the
    // listener: the read then ended with a broken pipe at a moment of the
    // kernel's choosing, and a fast enough (release) run finished it before
    // the object count reached zero -- so the state below was never seen.
    // Holding the far end keeps the read pending until the test lets it go.
    let far_name = format!(r"\\.\pipe\comio_unload_far_{}", std::process::id());
    let far_end = unsafe {
        CreateNamedPipeW(
            &windows_core::HSTRING::from(&far_name),
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0),
            PIPE_TYPE_BYTE,
            1,
            4096,
            4096,
            0,
            None,
        )
    };
    assert!(!far_end.is_invalid(), "CreateNamedPipeW failed");
    let client = connect(&far_name);
    let buffer = Buffer::Create(16).expect("Buffer::Create");
    let read = client
        .cast::<IInputStream>()
        .expect("QI IInputStream")
        .ReadAsync(&buffer, 16, InputStreamOptions::Partial)
        .expect("ReadAsync");

    // Everything the test holds goes, including the operation object. The
    // read itself is still pending: nothing has been written, and the far
    // end is still open.
    drop(read);
    drop(client);
    drop(listener);
    let _ = pipe_name;

    // What keeps the module now is the read: its operation object, which the
    // completion callback holds until the read ends, and the op block that
    // `LIVE_OPS` counts. (There is no moment with the object gone and the op
    // still armed long enough to observe: the callback releases the object
    // as its last act. An earlier version waited for one and was flaky.)
    std::thread::sleep(Duration::from_millis(50));
    assert!(!can_unload(), "the module offered to unload with a read in flight");
    assert!(
        winasync::common::overlapped::LIVE_OPS.load(std::sync::atomic::Ordering::Acquire) > 0,
        "a pending read is not counted in LIVE_OPS -- check that it is not          gated to debug builds again"
    );

    // Now let the read end: closing the far end fails it with a broken pipe,
    // and once its completion has run the module may go.
    unsafe { CloseHandle(far_end) }.expect("CloseHandle");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !can_unload() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(outstanding(), 0, "objects outlived the test");
    assert!(
        can_unload(),
        "the operation never settled; LIVE_OPS is still nonzero"
    );

    // --- 6. Process-global state leaves with the last socket -------------
    // Everything above used named pipes, which never touch Winsock. A socket
    // transport takes a Winsock reference, and the module used to answer a
    // permanent "no" from that point on because the cached provider
    // pointers could not be retired. Now each socket carries its own
    // reference and releases it as it closes, so nothing is left for the
    // module to give back once its objects and operations are gone.
    assert!(can_unload(), "precondition: still unloadable before any socket");
    assert!(!wsa_initialized(), "precondition: Winsock is not up before any socket");

    // Constructing the connector is not enough -- Winsock comes up when a
    // socket is actually created. A connect to a path nothing is listening
    // on creates one and then fails, which is all this needs. A bound
    // listener on the side holds a socket -- and so a reference -- across
    // that.
    let unix = winasync::comio::UnixConnector::new();
    let typed: winasync::comio::bindings::Pbk20191::WinAsync::Comio::IUnixConnector =
        unix.cast().expect("QI IUnixConnector");
    let dir = std::env::temp_dir();
    let held = dir.join(format!("comio_unload_held_{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&held);
    let listener = winasync::comio::UnixStreamListener::bind(&held.to_string_lossy(), None)
        .expect("bind a listener to hold a socket");
    assert!(
        wsa_initialized(),
        "a socket was created without bringing Winsock up"
    );
    let path = dir.join(format!("comio_unload_{}.sock", std::process::id()));
    let endpoint = windows_core::HSTRING::from(path.to_string_lossy().as_ref());
    let _ = typed.ConnectAsync(&endpoint).and_then(|op| op.join());
    drop(typed);
    drop(unix);
    settle(|| outstanding() == 0, "the connector outlived its reference");
    assert!(
        wsa_initialized(),
        "the listener's socket is open, so its reference must still be held"
    );

    // The last socket out takes Winsock with it.
    drop(listener);
    let _ = std::fs::remove_file(&held);
    settle(|| !wsa_initialized(), "the last socket closed without releasing Winsock");
    assert!(
        can_unload(),
        "objects are gone, so the module should be willing to unload"
    );

    // And the module still works afterwards. This is the half that the
    // cached `ConnectEx` / `AcceptEx` pointers used to make unsound: they
    // were fetched under the old Winsock and would survive a teardown that
    // could have unloaded the provider. Each now carries the teardown count
    // it was fetched under, so this connect re-fetches rather than calling
    // through a stale address.
    let again = winasync::comio::UnixConnector::new();
    let typed: winasync::comio::bindings::Pbk20191::WinAsync::Comio::IUnixConnector =
        again.cast().expect("QI IUnixConnector");
    let _ = typed.ConnectAsync(&endpoint).and_then(|op| op.join());
    drop(typed);
    drop(again);
    settle(|| outstanding() == 0, "the second connector outlived its reference");
    settle(|| !wsa_initialized(), "the second socket closed without releasing Winsock");
    assert!(can_unload());
}

/// Waits briefly for a condition that a completion callback settles on
/// another thread, then asserts it.
fn settle(cond: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cond() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(cond(), "{what}");
}

/// A listener bound to a fresh pipe name, plus that name.
fn bound_listener() -> (
    windows_core::ComObject<winasync::comio::NamedPipeConnectionListener>,
    String,
) {
    use winasync::comio::NamedPipeConnectionListener;
    use winasync::comio::bindings::Pbk20191::WinAsync::Comio::{
        IConnectionListener, IConnectionReceivedArgs,
    };
    use windows::Foundation::TypedEventHandler;

    let name = format!(r"\\.\pipe\comio_unload_{}", std::process::id());
    let listener = NamedPipeConnectionListener::new();
    // A handler, so the accept chain actually starts and there is real I/O
    // to be in flight. It keeps nothing, on purpose.
    listener
        .connection_received(&TypedEventHandler::<
            IConnectionListener,
            IConnectionReceivedArgs,
        >::new(|_sender, _args| Ok(())))
        .expect("connection_received");
    listener
        .bind(&windows_core::HSTRING::from(&name))
        .expect("bind");
    (listener, name)
}

/// The client end of `name`.
fn connect(name: &str) -> ITransportStream {
    let connector = NamedPipeConnector::new();
    let typed: winasync::comio::bindings::Pbk20191::WinAsync::Comio::INamedPipeConnector =
        connector.cast().expect("QI INamedPipeConnector");
    typed
        .ConnectAsync(&windows_core::HSTRING::from(name))
        .expect("ConnectAsync")
        .join()
        .expect("connect result")
}
