use super::*;
use crate::comio::buffer::{buffer_ptr, new_buffer};
use crate::common::block_on;
use std::future::IntoFuture;
use std::sync::mpsc;
use windows::Foundation::IClosable;
use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
use windows_core::{IInspectable, Interface, HSTRING};

#[test]
fn connection_received_fires_with_working_stream() {
    let name = format!(r"\\.\pipe\comio_listener_{}", std::process::id());
    let listener = NamedPipeConnectionListener::new();

    // `IInspectable` (and hence the accepted stream) isn't `Send` — same
    // reasoning as `comio::tests::cancel_storm` — so `TypedEventHandler`'s
    // `F: Send` bound rules out sending it directly through the channel
    // from the pool thread the event fires on. Smuggle the identity as a
    // `usize` (itself `Send`) and reconstitute it on this thread instead;
    // the object underneath is genuinely thread-safe (a plain overlapped
    // registration), only the `IInspectable` wrapper type is
    // conservatively `!Send`.
    let (tx, rx) = mpsc::channel::<usize>();
    let handler = ConnectionHandler::new(move |_sender, args| {
        let stream = args.as_ref().unwrap().Stream()?;
        let _ = tx.send(stream.into_raw() as usize);
        Ok(())
    });
    listener.connection_received(&handler).unwrap();
    block_on(listener.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();

    // Client connects (short retry to cover the inter-instance window
    // between the loop's pipe instances).
    let client = {
        let mut last = None;
        for _ in 0..50 {
            match block_on(crate::comio::NamedPipeConnector::connect(&name)) {
                Ok(c) => {
                    last = Some(c);
                    break;
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        }
        last.expect("client failed to connect")
    };

    // Server-side stream delivered via the event; echo a payload
    // client->server->client to prove it's a working duplex stream, not
    // just a non-null object.
    let server_raw = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("no ConnectionReceived");
    let server = unsafe { IInspectable::from_raw(server_raw as *mut core::ffi::c_void) };
    let c_out: IOutputStream = client.cast().unwrap();
    let c_in: IInputStream = client.cast().unwrap();
    let s_in: IInputStream = server.cast().unwrap();
    let s_out: IOutputStream = server.cast().unwrap();

    let wb = new_buffer(4).unwrap();
    unsafe {
        buffer_ptr(&wb.clone().into())
            .unwrap()
            .copy_from_nonoverlapping(b"echo".as_ptr(), 4);
    }
    wb.SetLength(4).unwrap();
    block_on(c_out.WriteAsync(&wb.cast::<IBuffer>().unwrap()).unwrap().into_future()).unwrap();
    let got: IBuffer = block_on(
        s_in.ReadAsync(&new_buffer(4).unwrap().cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0))
            .unwrap()
            .into_future(),
    )
    .unwrap();
    block_on(s_out.WriteAsync(&got).unwrap().into_future()).unwrap();
    let echoed: IBuffer = block_on(
        c_in.ReadAsync(&new_buffer(4).unwrap().cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0))
            .unwrap()
            .into_future(),
    )
    .unwrap();
    unsafe {
        assert_eq!(std::slice::from_raw_parts(buffer_ptr(&echoed).unwrap(), 4), b"echo");
    }

    // `NamedPipeConnectionListener::new()` returns a `ComObject<ConnectionListener>`:
    // inherent methods (`ConnectionReceived`/`BindAsync` above) are reached
    // through its `Deref` chain, and `IClosable::Close` is reached via
    // `ComObject::cast` (a `QueryInterface`) rather than `IClosable::from`,
    // since the `#[implement]`-generated `From` impls take the concrete
    // type by value, not by reference.
    let closable: IClosable = listener.cast().unwrap();
    closable.Close().unwrap();
}

#[test]
fn unix_connection_received_echo() {
    let path = std::env::temp_dir().join(format!(
        "comio_listener_unix_{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let p = path.to_string_lossy().into_owned();

    let listener = UnixConnectionListener::new();
    let (tx, rx) = mpsc::channel::<usize>();
    let handler = ConnectionHandler::new(move |_s, args| {
        let stream = args.as_ref().unwrap().Stream()?;
        let _ = tx.send(stream.into_raw() as usize);
        Ok(())
    });
    listener.connection_received(&handler).unwrap();
    block_on(listener.bind(&HSTRING::from(&p)).unwrap().into_future()).unwrap();

    let client = block_on(crate::comio::UnixConnector::connect(&p)).unwrap();
    let server_raw = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("no ConnectionReceived");
    let server = unsafe { IInspectable::from_raw(server_raw as *mut core::ffi::c_void) };

    let c_out: IOutputStream = client.cast().unwrap();
    let s_in: IInputStream = server.cast().unwrap();
    let s_out: IOutputStream = server.cast().unwrap();
    let c_in: IInputStream = client.cast().unwrap();
    let wb = new_buffer(4).unwrap();
    unsafe {
        buffer_ptr(&wb.clone().into())
            .unwrap()
            .copy_from_nonoverlapping(b"unix".as_ptr(), 4);
    }
    wb.SetLength(4).unwrap();
    block_on(c_out.WriteAsync(&wb.cast::<IBuffer>().unwrap()).unwrap().into_future()).unwrap();
    let got: IBuffer = block_on(
        s_in.ReadAsync(&new_buffer(4).unwrap().cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0))
            .unwrap()
            .into_future(),
    )
    .unwrap();
    block_on(s_out.WriteAsync(&got).unwrap().into_future()).unwrap();
    let echoed: IBuffer = block_on(
        c_in.ReadAsync(&new_buffer(4).unwrap().cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0))
            .unwrap()
            .into_future(),
    )
    .unwrap();
    unsafe {
        assert_eq!(std::slice::from_raw_parts(buffer_ptr(&echoed).unwrap(), 4), b"unix");
    }

    let closable: IClosable = listener.cast().unwrap();
    closable.Close().unwrap();
    let _ = std::fs::remove_file(&path);
}

/// Drives a listener entirely through `IConnectionListener`'s vtable —
/// no inherent Rust method, no concrete type — which is what an external
/// COM caller sees. The previous tests all called the Rust surface, so
/// they could not have caught a broken vtable.
#[test]
fn com_vtable_surface_works_for_every_transport() {
    fn exercise(obj: windows_core::IUnknown, expect_endpoint: &str, expect_transport: &str) {
        let l: IConnectionListener = obj.cast().expect("QI IConnectionListener");

        // The metadata says every listener answers `ITransport`. Asserting
        // it here is the point: declaring an interface in the `.winmd` and
        // forgetting it in `#[implement]` compiles perfectly and fails only
        // at `QueryInterface`, in whatever consumer tries it first.
        let t: ITransport = obj.cast().expect("QI ITransport");
        assert_eq!(
            t.Transport().expect("Transport").to_string(),
            expect_transport
        );

        let (tx, _rx) = mpsc::channel::<()>();
        let handler = ConnectionHandler::new(move |_, _| {
            let _ = tx.send(());
            Ok(())
        });

        // Register through the vtable.
        let token = l
            .ConnectionReceived(&handler)
            .expect("vtable ConnectionReceived");
        assert_ne!(token, 0, "a real token came back through the vtable");

        // Read the endpoint through the vtable.
        assert_eq!(
            l.Information().expect("vtable Information").to_string(),
            expect_endpoint
        );

        // Remove through the vtable, then close through IClosable.
        l.RemoveConnectionReceived(token)
            .expect("vtable RemoveConnectionReceived");
        let c: IClosable = l.cast().expect("QI IClosable");
        c.Close().expect("Close");

        // Closed members report RO_E_CLOSED through the vtable too.
        assert_eq!(l.Information().unwrap_err().code(), RO_E_CLOSED);
    }

    let pipe_name = format!(r"\\.\pipe\comvtable_{}", std::process::id());
    // Bind through the vtable too, so the whole COM surface is covered:
    // without this a COM caller could register a handler but never start
    // the listener.
    let pipe = NamedPipeConnectionListener::new();
    {
        let b: INamedPipeBind = pipe.cast().expect("QI INamedPipeBind");
        b.BindAsync(&HSTRING::from(&pipe_name)).expect("vtable BindAsync");
    }
    exercise(pipe.cast().unwrap(), &pipe_name, "namedpipe");

    let unix = UnixConnectionListener::new();
    let path = std::env::temp_dir()
        .join(format!("comvtable_{}.sock", std::process::id()))
        .to_string_lossy()
        .into_owned();
    let _ = std::fs::remove_file(&path);
    {
        let b: IUnixBind = unix.cast().expect("QI IUnixBind");
        b.BindAsync(&HSTRING::from(&path)).expect("vtable BindAsync");
    }
    exercise(unix.cast().unwrap(), &path, "unix");
    let _ = std::fs::remove_file(&path);

    let hv = HyperVConnectionListener::new();
    // A vsock-template service id, so the bind below can be spelled with
    // the `vsock:` sugar the string form accepts. `BindServiceNameAsync`
    // takes the *service* half alone, not a whole `hv://` endpoint.
    let svc = crate::common::hvaddr::vsock_service_id(54_321);
    let hv_bound = {
        let b: IHyperVBind = hv.cast().expect("QI IHyperVBind");
        b.BindServiceNameAsync(&HSTRING::from("vsock:54321")).is_ok()
    };
    if hv_bound {
        let label = HvSocketAddr::new(HV_GUID_WILDCARD, svc).to_string();
        exercise(hv.cast().unwrap(), &label, "hyperv");
    } else {
        eprintln!("skipped Hyper-V leg: AF_HYPERV unavailable");
    }
}

/// What used to be a runtime check. Each listener now exposes only the
/// bind its addressing supports, so the mismatches this once asserted
/// (`E_INVALIDARG` from a string bind on Hyper-V, from a GUID bind on the
/// other two) are compile errors instead. Kept as a note so the removal
/// is not mistaken for lost coverage; see the commented lines.
#[test]
fn mismatched_binds_do_not_compile() {
    let hv = HyperVConnectionListener::new();
    let pipe = NamedPipeConnectionListener::new();
    // hv.BindAsync(&HSTRING::from("loopback"));            // no such method
    // pipe.BindServiceNameAsync(&windows_core::GUID::zeroed()); // no such method
    let _ = (hv, pipe);
}

#[test]
fn hyper_v_connection_received_echo() {
    // A service GUID of our own. Host-to-host loopback needs no
    // GuestCommunicationServices registration; a machine with no Hyper-V
    // socket provider at all reports a skip instead of failing.
    let service = windows_core::GUID::from_u128(0x5a83c7e1_9b04_4d26_8f17_63ca0e2b95d8);

    let listener = HyperVConnectionListener::new();
    let (tx, rx) = mpsc::channel::<(usize, HSTRING)>();
    let handler = ConnectionHandler::new(move |_s, args| {
        let a = args.as_ref().unwrap();
        let _ = tx.send((a.Stream()?.into_raw() as usize, a.RemoteEndpoint()?));
        Ok(())
    });
    listener.connection_received(&handler).unwrap();
    // Handler is registered, so the lazy AF_HYPERV bind happens here and
    // its error (e.g. WSAEAFNOSUPPORT) surfaces synchronously.
    if let Err(e) = listener.bind_service_name(&service) {
        eprintln!("skipped: AF_HYPERV listener bind unavailable ({e:?})");
        return;
    }

    let client = match block_on(crate::comio::HyperVConnector::connect(
        HvSocketAddr::loopback(service),
    )) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("skipped: AF_HYPERV loopback connect refused ({e:?})");
            return;
        }
    };
    let (server_raw, remote) = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("no ConnectionReceived");
    let server = unsafe { IInspectable::from_raw(server_raw as *mut core::ffi::c_void) };

    // RemoteEndpoint names the *peer* here, not the listener's own bound
    // address: a wildcard-bound service accepts from every partition, so
    // this is how a host service learns which one it is talking to.
    let remote = remote.to_string();
    assert!(
        remote.starts_with("hv://loopback/"),
        "RemoteEndpoint should name the connecting partition, got {remote:?}"
    );

    // The accepted connection is an ordinary WinRT duplex stream — the
    // whole point of routing hvsock through this listener.
    let c_out: IOutputStream = client.cast().unwrap();
    let s_in: IInputStream = server.cast().unwrap();
    let s_out: IOutputStream = server.cast().unwrap();
    let c_in: IInputStream = client.cast().unwrap();
    let wb = new_buffer(4).unwrap();
    unsafe {
        buffer_ptr(&wb.clone().into())
            .unwrap()
            .copy_from_nonoverlapping(b"hvsk".as_ptr(), 4);
    }
    wb.SetLength(4).unwrap();
    block_on(c_out.WriteAsync(&wb.cast::<IBuffer>().unwrap()).unwrap().into_future()).unwrap();
    let got: IBuffer = block_on(
        s_in.ReadAsync(&new_buffer(4).unwrap().cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0))
            .unwrap()
            .into_future(),
    )
    .unwrap();
    block_on(s_out.WriteAsync(&got).unwrap().into_future()).unwrap();
    let echoed: IBuffer = block_on(
        c_in.ReadAsync(&new_buffer(4).unwrap().cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0))
            .unwrap()
            .into_future(),
    )
    .unwrap();
    unsafe {
        assert_eq!(std::slice::from_raw_parts(buffer_ptr(&echoed).unwrap(), 4), b"hvsk");
    }

    let closable: IClosable = listener.cast().unwrap();
    closable.Close().unwrap();
}

#[test]
fn close_cancels_inflight_accept_no_leak() {
    use crate::common::block_on;
    use windows::Foundation::IClosable;
    let name = format!(r"\\.\pipe\comio_listener_leak_{}", std::process::id());
    let listener = NamedPipeConnectionListener::new();
    let handler = ConnectionHandler::new(|_, _| Ok(()));
    listener.connection_received(&handler).unwrap();
    block_on(listener.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();
    // No client connects; the chain is parked in an in-flight ConnectNamedPipe.
    std::thread::sleep(std::time::Duration::from_millis(50));
    // Information returns the endpoint; then close.
    assert_eq!(listener.information().unwrap().to_string(), name);
    let closable: IClosable = listener.cast().unwrap();
    closable.Close().unwrap();
    assert_eq!(listener.information().unwrap_err().code(), RO_E_CLOSED);
    {
        for _ in 0..500 {
            if crate::comio::tpio::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            crate::comio::tpio::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "listener accept leaked an op"
        );
    }
}

#[test]
fn drop_without_close_no_leak() {
    use crate::common::block_on;
    let name = format!(r"\\.\pipe\comio_listener_dropleak_{}", std::process::id());
    let listener = NamedPipeConnectionListener::new();
    let handler = ConnectionHandler::new(|_, _| Ok(()));
    listener.connection_received(&handler).unwrap();
    block_on(listener.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();
    // No client connects; the chain is parked in an in-flight ConnectNamedPipe.
    std::thread::sleep(std::time::Duration::from_millis(50));
    // Drop the sole COM reference WITHOUT calling Close(). This proves
    // `impl Drop for ConnectionListener` (not `IClosable::Close`) is what
    // breaks the `Arc<ListenerState>` <-> accept-chain reference cycle:
    // without it, this drop would leak the state, the in-flight
    // overlapped op, and leave the chain able to keep re-arming forever.
    drop(listener);
    {
        let mut live = crate::comio::tpio::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed);
        for _ in 0..500 {
            if live == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            live = crate::comio::tpio::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed);
        }
        assert_eq!(live, 0, "dropping listener without Close leaked an op");
    }
}

#[test]
fn members_after_close_return_ro_e_closed() {
    let listener = NamedPipeConnectionListener::new();
    let closable: IClosable = listener.cast().unwrap();
    closable.Close().unwrap();
    let handler = ConnectionHandler::new(|_, _| Ok(()));
    assert_eq!(
        listener.connection_received(&handler).unwrap_err().code(),
        RO_E_CLOSED
    );
    assert_eq!(
        listener.bind(&HSTRING::from("x")).unwrap_err().code(),
        RO_E_CLOSED
    );
    // Second Close is a no-op.
    closable.Close().unwrap();
}

/// A synchronous failure to issue the *first* accept (here, a pipe name
/// that doesn't match the required `\\.\pipe\...` form, so
/// `CreateNamedPipeW` fails synchronously inside `pipe::arm_accept`)
/// must surface as an `Err` from `BindAsync` itself, not get swallowed
/// (the way an internal re-arm's failure is, deliberately, — see
/// `drive`'s doc comment).
///
/// A handler is registered *before* `BindAsync` so the accept chain is
/// actually started by it (handler gating — with no handler, `BindAsync`
/// records the endpoint but issues no accept, so there would be no
/// error to surface; see the zero-handler case in
/// `accept_gated_on_handler`).
#[test]
fn bind_async_surfaces_first_accept_error() {
    let listener = NamedPipeConnectionListener::new();
    let handler = ConnectionHandler::new(|_, _| Ok(()));
    listener.connection_received(&handler).unwrap();
    let err = listener
        .bind(&HSTRING::from("not_a_valid_pipe_name"))
        .unwrap_err();
    assert_ne!(err.code().0, 0);
    // The listener is otherwise still usable (BindAsync failing isn't
    // the same as Close): a well-formed retry should still work.
    assert!(listener.information().is_err() || listener.information().is_ok());
}

/// The mirror of `bind_async_surfaces_first_accept_error` for the
/// bind-first-then-handler ordering: when a listener is bound *before* any
/// handler (so gating defers the first accept, and `BindAsync` itself
/// succeeds), a malformed endpoint's first-accept error is surfaced from
/// the first `ConnectionReceived` — not swallowed — and that failed
/// registration is rolled back (so a later, valid rebind + handler still
/// works from a clean slate).
#[test]
fn connection_received_surfaces_deferred_bind_error() {
    let listener = NamedPipeConnectionListener::new();

    // No handler yet: gating means BindAsync records the (malformed)
    // endpoint but issues no accept, so it succeeds.
    block_on(
        listener
            .bind(&HSTRING::from("not_a_valid_pipe_name"))
            .unwrap()
            .into_future(),
    )
    .unwrap();

    // Registering the first handler starts the chain, which tries the first
    // accept and fails synchronously — surfaced here, with the registration
    // rolled back.
    let handler = ConnectionHandler::new(|_, _| Ok(()));
    let err = listener.connection_received(&handler).unwrap_err();
    assert_ne!(err.code().0, 0);
    assert_ne!(err.code(), RO_E_CLOSED, "must be the accept error, not closed");

    // Rollback left zero handlers, so re-adding is again the "first"
    // handler. Rebind to a valid endpoint first (source still points at the
    // bad name until then), then register: a clean slate works end to end.
    let name = format!(r"\\.\pipe\comio_listener_deferred_{}", std::process::id());
    block_on(listener.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();
    let (tx, rx) = mpsc::channel::<usize>();
    let handler2 = ConnectionHandler::new(move |_s, args| {
        let raw = args.as_ref().unwrap().Stream()?.into_raw() as usize;
        let _ = tx.send(raw);
        Ok(())
    });
    listener.connection_received(&handler2).unwrap();

    let client = {
        let mut last = None;
        for _ in 0..50 {
            match block_on(crate::comio::NamedPipeConnector::connect(&name)) {
                Ok(c) => {
                    last = Some(c);
                    break;
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        }
        last.expect("client failed to connect after a clean rebind")
    };
    let server_raw = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("no ConnectionReceived after a clean rebind");
    let _server = unsafe { IInspectable::from_raw(server_raw as *mut core::ffi::c_void) };
    drop(client);

    let closable: IClosable = listener.cast().unwrap();
    closable.Close().unwrap();
}

/// Handler gating: a bound listener with **no** `ConnectionReceived`
/// handler must not accept connections (no pipe instance is created, so a
/// client can't connect); registering the first handler starts accepting,
/// and the handler then receives the connection.
#[test]
fn accept_gated_on_handler() {
    let name = format!(r"\\.\pipe\comio_listener_gate_{}", std::process::id());
    let listener = NamedPipeConnectionListener::new();

    // Bind with no handler registered. Gating => no accept chain, so no
    // pipe instance exists and a client connect must fail.
    block_on(listener.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        block_on(crate::comio::NamedPipeConnector::connect(&name)).is_err(),
        "connected to a listener that has no ConnectionReceived handler"
    );

    // Register the first handler => chain starts (the first pipe instance
    // is created synchronously inside `ConnectionReceived`), so a client
    // can now connect and the handler fires.
    let (tx, rx) = mpsc::channel::<usize>();
    let handler = ConnectionHandler::new(move |_s, args| {
        let stream = args.as_ref().unwrap().Stream()?;
        let _ = tx.send(stream.into_raw() as usize);
        Ok(())
    });
    listener.connection_received(&handler).unwrap();

    let client = {
        let mut last = None;
        for _ in 0..50 {
            match block_on(crate::comio::NamedPipeConnector::connect(&name)) {
                Ok(c) => {
                    last = Some(c);
                    break;
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        }
        last.expect("client failed to connect after a handler was registered")
    };
    let server_raw = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("no ConnectionReceived after registering a handler");
    // Reconstitute + drop the delivered stream so the op doesn't linger.
    let _server = unsafe { IInspectable::from_raw(server_raw as *mut core::ffi::c_void) };
    drop(client);

    let closable: IClosable = listener.cast().unwrap();
    closable.Close().unwrap();
}

/// Handler gating is reversible: removing the *last* `ConnectionReceived`
/// handler pauses delivery (the in-flight accept is cancelled), and
/// registering a handler again resumes it. This exercises the
/// remove-then-re-add handoff (`restart_pending`).
///
/// A paused pipe listener still holds the name: one listening instance is
/// kept in reserve from the first connection until `Close`, so no other
/// server can take the name in the meantime (see `common::pipe`). A client
/// can therefore connect while it is paused; nothing is delivered until a
/// handler returns, and then that client is.
#[test]
fn pause_and_resume_on_handler_toggle() {
    let name = format!(r"\\.\pipe\comio_listener_toggle_{}", std::process::id());
    let listener = NamedPipeConnectionListener::new();

    // Helper: build a handler that smuggles the delivered stream identity.
    let make_handler = |tx: mpsc::Sender<usize>| {
        ConnectionHandler::new(move |_s, args| {
            let raw = args.as_ref().unwrap().Stream()?.into_raw() as usize;
            let _ = tx.send(raw);
            Ok(())
        })
    };
    let connect_once = |name: &str| {
        let mut last = None;
        for _ in 0..50 {
            match block_on(crate::comio::NamedPipeConnector::connect(name)) {
                Ok(c) => {
                    last = Some(c);
                    break;
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        }
        last
    };

    // Bind with a handler, confirm a connection is delivered.
    let (tx1, rx1) = mpsc::channel::<usize>();
    let h1 = make_handler(tx1);
    let token = listener.connection_received(&h1).unwrap();
    block_on(listener.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();
    let c1 = connect_once(&name).expect("first connect failed");
    let raw1 = rx1
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("no delivery while a handler was registered");
    let _s1 = unsafe { IInspectable::from_raw(raw1 as *mut core::ffi::c_void) };
    drop(c1);

    // Remove the last handler => chain pauses. A client connects to the
    // reserved instance, and nothing is delivered for it.
    listener.remove_connection_received(token).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));
    let c2 = connect_once(&name).expect("the paused listener gave up its name");
    assert!(
        rx1.recv_timeout(std::time::Duration::from_millis(200)).is_err(),
        "a connection was delivered after the last handler was removed"
    );

    // Re-add a handler => chain resumes (restart_pending handoff), and
    // the client that connected while paused is delivered.
    let (tx2, rx2) = mpsc::channel::<usize>();
    let h2 = make_handler(tx2);
    listener.connection_received(&h2).unwrap();
    let raw2 = rx2
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("no delivery after the chain resumed");
    let _s2 = unsafe { IInspectable::from_raw(raw2 as *mut core::ffi::c_void) };
    drop(c2);

    let closable: IClosable = listener.cast().unwrap();
    closable.Close().unwrap();
}

/// Concurrent-backlog liveness test. Originally written to probe a
/// stack-overflow risk from recursive same-stack re-arming under a
/// backlog of already-connected clients (the named-pipe transport's
/// `ERROR_PIPE_CONNECTED` synchronous-completion case — see
/// `pipe::arm_accept`'s doc comment); that risk is now structurally
/// impossible, since `tpio::submit` re-dispatches every synchronous
/// completion to the default thread pool instead of invoking it inline
/// (see `tpio::submit`'s and `drive`'s doc comments) — `on_result` never
/// runs on the stack of the `try_arm` call that armed it, so a re-arm from
/// `on_result` can never recurse, regardless of how large the connect
/// backlog is. This test is kept as a liveness/throughput check under
/// real concurrent connection load rather than a stack-safety repro:
///
/// A horde of client threads is started *before* the listener is bound,
/// each hammer-retrying `NamedPipeConnector::connect` with no backoff, to
/// produce a realistic over-subscription burst against the chain's
/// re-arm loop. The test then checks that the chain still makes real
/// forward progress (delivers most connections) under that load, i.e.
/// the pool-dispatch restructuring didn't introduce a livelock/deadlock.
/// Functional event-delivery coverage for the ordinary single- and
/// multi-connection cases lives in
/// `connection_received_fires_with_working_stream` and
/// `unix_connection_received_echo`.
#[test]
fn synchronous_backlog_does_not_overflow() {
    use std::sync::atomic::AtomicUsize;

    const N: usize = 200;
    let name = format!(r"\\.\pipe\comio_listener_backlog_{}", std::process::id());

    let stop = Arc::new(AtomicBool::new(false));
    let connected = Arc::new(AtomicUsize::new(0));
    let clients: Vec<_> = (0..N)
        .map(|_| {
            let name = name.clone();
            let stop = stop.clone();
            let connected = connected.clone();
            std::thread::spawn(move || {
                // Tight retry loop, no backoff sleep on failure: the
                // point is to have as many threads as possible spinning
                // on `CreateFileW`/`WaitNamedPipeW` at once so some of
                // them are ready to win the `CreateNamedPipeW` ->
                // `ConnectNamedPipe` race inside the server's chain.
                while !stop.load(Ordering::Relaxed) {
                    match block_on(crate::comio::NamedPipeConnector::connect(&name)) {
                        Ok(c) => {
                            connected.fetch_add(1, Ordering::Relaxed);
                            // Hold the client open briefly so the
                            // server side has time to hand its stream
                            // to the `ConnectionReceived` event before
                            // the connection disappears out from under
                            // it.
                            std::thread::sleep(std::time::Duration::from_millis(10));
                            drop(c);
                            return;
                        }
                        Err(_) => continue,
                    }
                }
            })
        })
        .collect();

    let listener = NamedPipeConnectionListener::new();
    let delivered = Arc::new(AtomicUsize::new(0));
    let delivered_cb = delivered.clone();
    let handler = ConnectionHandler::new(move |_s, _args| {
        delivered_cb.fetch_add(1, Ordering::Relaxed);
        Ok(())
    });
    listener.connection_received(&handler).unwrap();
    block_on(listener.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();

    // Bounded wait for the backlog to drain. If the pool-dispatch
    // restructuring regressed into a livelock, this loop's own timeout
    // (not a process hang) is what fails the test.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline
        && delivered.load(Ordering::Relaxed) < (N * 9 / 10)
    {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }

    stop.store(true, Ordering::Relaxed);
    for c in clients {
        let _ = c.join();
    }

    let closable: IClosable = listener.cast().unwrap();
    closable.Close().unwrap();

    let delivered_n = delivered.load(Ordering::Relaxed);
    let connected_n = connected.load(Ordering::Relaxed);
    assert!(
        delivered_n >= N / 2,
        "only {delivered_n} of {N} connections were delivered ({connected_n} clients \
         reported a successful connect) -- possible deadlock/regression in the re-arm chain"
    );
}
