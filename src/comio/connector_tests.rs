use super::*;
// Every interface under test now comes from metadata.
use crate::comio::bindings::Pbk20191::WinAsync::Comio::*;
use crate::comio::listener::{ConnectionHandler, NamedPipeConnectionListener};
use core::ffi::c_void;
use std::sync::mpsc;
use windows_core::{HSTRING, Interface};

/// The AF_UNIX connector, whose result arrives from the `ConnectEx`
/// completion rather than from a blocked thread. Exercises the
/// `ConnectOp` state machine: `SetCompleted` before the result lands,
/// then `GetResults` after.
#[test]
fn unix_connector_completes_from_the_overlapped_callback() {
    use crate::comio::bindings::Pbk20191::WinAsync::Comio::IUnixBind;
    use crate::comio::listener::UnixConnectionListener;

    let path = std::env::temp_dir()
        .join(format!("comio_conn_{}.sock", std::process::id()))
        .to_string_lossy()
        .into_owned();
    let _ = std::fs::remove_file(&path);

    let listener = UnixConnectionListener::new();
    let (accepted_tx, accepted_rx) = mpsc::channel::<()>();
    let handler = ConnectionHandler::new(move |_s, _a| {
        let _ = accepted_tx.send(());
        Ok(())
    });
    {
        let b: IUnixBind = listener.cast().unwrap();
        b.BindAsync(&HSTRING::from(&path)).expect("BindAsync");
    }
    let l: IConnectionListener = listener.cast().unwrap();
    l.ConnectionReceived(&handler).expect("ConnectionReceived");

    let connector = crate::comio::UnixConnector::new();
    let c: IUnixConnector = connector.cast().unwrap();
    let op: IAsyncOperation<ITransportStream> =
        c.ConnectAsync(&HSTRING::from(&path)).expect("ConnectAsync");

    // Register a Completed handler the way a COM caller would, rather
    // than blocking: this is the path that would never fire if the
    // operation were not completed from the callback.
    let (done_tx, done_rx) = mpsc::channel::<AsyncStatus>();
    op.SetCompleted(&AsyncOperationCompletedHandler::new(move |_op, status| {
        let _ = done_tx.send(status);
        Ok(())
    }))
    .expect("SetCompleted");

    let status = done_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("Completed never fired");
    assert_eq!(status, AsyncStatus::Completed);

    let client = op.GetResults().expect("GetResults");
    assert!(client
        .cast::<windows::Storage::Streams::IOutputStream>()
        .is_ok());
    accepted_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("listener never accepted");

    let closable: windows::Foundation::IClosable = l.cast().unwrap();
    closable.Close().unwrap();
    let _ = std::fs::remove_file(&path);
}

/// A full COM round trip: accept through the listener's vtable, connect
/// through the connector's, both sides reached only by `QueryInterface`.
#[test]
fn com_connector_reaches_a_com_listener() {
    let name = format!(r"\\.\pipe\comio_connector_{}", std::process::id());

    let listener = NamedPipeConnectionListener::new();
    let (tx, rx) = mpsc::channel::<usize>();
    let handler = ConnectionHandler::new(move |_s, args| {
        let stream = args.as_ref().unwrap().Stream()?;
        let _ = tx.send(stream.into_raw() as usize);
        Ok(())
    });

    // Bind and register through the listener's vtables.
    let bind: INamedPipeBind = listener.cast().unwrap();
    bind.BindAsync(&HSTRING::from(&name)).expect("BindAsync");
    let l: IConnectionListener = listener.cast().unwrap();
    l.ConnectionReceived(&handler).expect("ConnectionReceived");

    // Connect through the connector's vtable.
    let connector = crate::comio::NamedPipeConnector::new();
    let c: INamedPipeConnector = connector.cast().unwrap();
    let client = c
        .ConnectAsync(&HSTRING::from(&name))
        .expect("ConnectAsync")
        .join()
        .expect("connect result");

    let server_raw = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("no ConnectionReceived");
    let server = unsafe { ITransportStream::from_raw(server_raw as *mut c_void) };

    // Both ends are real duplex streams.
    assert!(server.cast::<windows::Storage::Streams::IInputStream>().is_ok());
    assert!(client.cast::<windows::Storage::Streams::IOutputStream>().is_ok());

    let closable: windows::Foundation::IClosable = l.cast().unwrap();
    closable.Close().unwrap();
}

/// The one interface all three clients share. A caller that holds an
/// `ITransport` learns which transport it has from `Transport`,
/// then asks for that transport's connect interface by `QueryInterface`
/// — the discovery step that replaces a connect member here.
///
/// Nothing is connected: construction and identity are the whole test,
/// so it needs no listener, no pipe and no Hyper-V provider.
#[test]
fn every_client_reports_its_transport_through_the_shared_interface() {
    fn transport_of(c: &ITransport) -> String {
        c.Transport().expect("Transport").to_string()
    }

    let pipe: ITransport = crate::comio::NamedPipeConnector::new().cast().unwrap();
    let unix: ITransport = crate::comio::UnixConnector::new().cast().unwrap();
    let hv: ITransport = crate::comio::HyperVConnector::new().cast().unwrap();

    assert_eq!(transport_of(&pipe), "namedpipe");
    assert_eq!(transport_of(&unix), "unix");
    assert_eq!(transport_of(&hv), "hyperv");

    // The name is not decoration: it says which connect interface the
    // object actually carries, and only that one.
    assert!(pipe.cast::<INamedPipeConnector>().is_ok());
    assert!(pipe.cast::<IUnixConnector>().is_err());
    assert!(unix.cast::<IUnixConnector>().is_ok());
    assert!(unix.cast::<IHyperVConnector>().is_err());
    assert!(hv.cast::<IHyperVConnector>().is_ok());
    assert!(hv.cast::<INamedPipeConnector>().is_err());
}
