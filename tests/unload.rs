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
#![cfg(feature = "comio")]

use winasync::comio::NamedPipeConnector;
use winasync::comio::activation::{CLSID_NAMED_PIPE_CONNECTOR, DllGetClassObject};
use winasync::comio::bindings::Pbk20191::WinAsync::Comio::ITransportStream;
use winasync::comio::unload::{can_unload, locks_held, outstanding};
use std::time::{Duration, Instant};
use windows::Storage::Streams::{Buffer, IInputStream, IOutputStream, InputStreamOptions};
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

    // --- 3. A server lock holds the module on its own --------------------
    // Through the real class factory, not the counter's own back door.
    let factory: IClassFactory = unsafe {
        let mut raw = core::ptr::null_mut();
        DllGetClassObject(
            &CLSID_NAMED_PIPE_CONNECTOR,
            &IClassFactory::IID,
            &mut raw,
        )
        .ok()
        .expect("DllGetClassObject");
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
        DllGetClassObject(&CLSID_NAMED_PIPE_CONNECTOR, &IClassFactory::IID, &mut raw)
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
    let client = connect(&pipe_name);
    let buffer = Buffer::Create(16).expect("Buffer::Create");
    let read = client
        .cast::<IInputStream>()
        .expect("QI IInputStream")
        .ReadAsync(&buffer, 16, InputStreamOptions::Partial)
        .expect("ReadAsync");

    // Everything the test holds goes, including the operation object. The
    // read itself is still pending: nothing has been written.
    drop(read);
    drop(client);
    drop(listener);

    // The accepted server end lives inside the listener's state, which the
    // drop above released, so the pipe's other end is gone and the read will
    // complete with a broken-pipe error rather than hanging. Either way it
    // is the *operation* keeping the module loaded now, not an object.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_zero_objects_with_io_live = false;
    while Instant::now() < deadline {
        if outstanding() == 0 {
            if !can_unload() {
                saw_zero_objects_with_io_live = true;
            }
            if can_unload() {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(outstanding(), 0, "objects outlived the test");
    assert!(
        can_unload(),
        "the operation never settled; LIVE_OPS is still nonzero"
    );
    assert!(
        saw_zero_objects_with_io_live,
        "never observed the state this test exists for: zero objects and an \
         operation still in flight. Either the read completed before the \
         first poll, or LIVE_OPS is not being counted — check that it is not \
         gated to debug builds again"
    );

    // --- 6. Process-global state is one-way ------------------------------
    // Everything above used named pipes, which never touch Winsock. The
    // moment a socket transport exists the module has raised the process's
    // Winsock reference and has no safe way to lower it, so the answer
    // becomes a permanent no — see `comio::unload` for why a matching
    // `WSACleanup` would be worse than the leak.
    assert!(can_unload(), "precondition: still unloadable before any socket");

    // Constructing the connector is not enough — `wsa_init` runs when a
    // socket is actually created. A connect to a path nothing is listening
    // on creates one and then fails, which is all this needs.
    let unix = winasync::comio::UnixConnector::new();
    let typed: winasync::comio::bindings::Pbk20191::WinAsync::Comio::IUnixConnector =
        unix.cast().expect("QI IUnixConnector");
    let path = std::env::temp_dir().join(format!("comio_unload_{}.sock", std::process::id()));
    let _ = typed
        .ConnectAsync(&windows_core::HSTRING::from(path.to_string_lossy().as_ref()))
        .and_then(|op| op.join());
    drop(typed);
    drop(unix);
    assert_eq!(outstanding(), 0, "the connector outlived its reference");
    assert!(
        !can_unload(),
        "the module offered to unload after taking a Winsock reference it \
         cannot give back"
    );
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

/// Silences the unused-import warning when the write half is not exercised.
#[allow(dead_code)]
fn _assert_output_stream_is_reachable(s: &ITransportStream) -> IOutputStream {
    s.cast().unwrap()
}
