//! COM connectors: the client half of [`super::listener`].
//!
//! The listeners are reachable from COM; without these the transports were
//! only half exposed — an external caller could accept a connection but never
//! make one. Same shape as the listeners: one concrete type per transport,
//! each carrying the connect its addressing supports, because a pipe name, a
//! filesystem path and a pair of GUIDs cannot share one signature.
//!
//! The interfaces themselves are no longer declared here. They live in
//! `metadata/winasync.rdl` and are generated into [`super::bindings`],
//! because a hand-written `#[interface]` cannot derive from `IInspectable`
//! and so cannot be a WinRT interface at all. What stays in this module is
//! the part metadata cannot express: the operation object behind every
//! `ConnectAsync`.
//!
//! # No blocked threads
//!
//! Each `ConnectAsync` returns an `IAsyncOperation<ITransportStream>` whose
//! result is delivered **from the overlapped completion**, not by running the
//! connect to completion on a thread. AF_UNIX and AF_HYPERV arm a `ConnectEx`
//! through `tpio::submit` and fill the operation from its callback — the same
//! shape `super::asyncop::IoAsyncOp` uses for reads and writes — so nothing
//! waits while a connect is outstanding, and `IAsyncInfo::Cancel` cancels the
//! real IO.
//!
//! The named pipe is the exception, and only partly: opening the client end
//! is a plain synchronous `CreateFileW`, so the usual case is done inline and
//! returns an already-finished operation, with no thread involved at all.
//! Only `ERROR_PIPE_BUSY` has something to wait for, and that wait is handed
//! to the thread pool rather than to the caller.

// PascalCase members are the COM ABI convention the generated traits are
// shaped to, not Rust naming slips.
#![allow(non_snake_case)]

use crate::comio::unload::ModuleRef;
use std::sync::Mutex;
use windows_core::{ComObject, HRESULT, IUnknownImpl, Result, implement};
use windows_future::{
    AsyncOperationCompletedHandler, AsyncStatus, IAsyncInfo_Impl, IAsyncOperation,
    IAsyncOperation_Impl,
};

use super::bindings::Pbk20191::WinAsync::Comio::ITransportStream;
use super::tpio::OpHandle;

/// An `IAsyncOperation<ITransportStream>` whose result arrives from an overlapped
/// completion rather than from a closure run to completion on a thread.
///
/// The same state machine as `super::asyncop::IoAsyncOp`, which does this for
/// reads and writes; this one is fixed to `IAsyncOperation<ITransportStream>`,
/// the shape a connect returns. `IAsyncInfo::Cancel` cancels the in-flight
/// `ConnectEx`.
struct ConnectState {
    result: Option<Result<ITransportStream>>,
    completed: Option<AsyncOperationCompletedHandler<ITransportStream>>,
    completed_assigned: bool,
    op: Option<OpHandle>,
}

#[implement(IAsyncOperation<ITransportStream>, windows_future::IAsyncInfo)]
struct ConnectOp(Mutex<ConnectState>, ModuleRef);

// SAFETY: the only non-`Send` data (the pending handler) lives behind the
// `Mutex` and every access is serialized by it; the object is agile, so its
// interfaces may be used from the completion callback's thread. Mirrors
// `asyncop::IoAsyncOp`.
unsafe impl Send for ConnectOp {}
unsafe impl Sync for ConnectOp {}

impl ConnectOp_Impl {
    /// Stores the outcome and fires `Completed` if one is already assigned.
    /// Runs on the thread-pool completion thread.
    fn complete(&self, res: Result<ITransportStream>) {
        let (status, completed) = {
            let mut g = self.0.lock().unwrap();
            debug_assert!(g.result.is_none());
            g.result = Some(res);
            (g.status(), g.completed.take())
        };
        // Outside the lock: the handler may re-enter to read Status/GetResults.
        if let Some(completed) = completed {
            let sender: IAsyncOperation<ITransportStream> = self.to_interface();
            let _ = completed.Invoke(&sender, status);
        }
    }
}

impl ConnectState {
    fn status(&self) -> AsyncStatus {
        match &self.result {
            None => AsyncStatus::Started,
            Some(Ok(_)) => AsyncStatus::Completed,
            Some(Err(_)) => AsyncStatus::Error,
        }
    }
}

impl IAsyncInfo_Impl for ConnectOp_Impl {
    fn Id(&self) -> Result<u32> {
        Ok(1)
    }
    fn Status(&self) -> Result<AsyncStatus> {
        Ok(self.0.lock().unwrap().status())
    }
    fn ErrorCode(&self) -> Result<HRESULT> {
        Ok(match &self.0.lock().unwrap().result {
            Some(Err(e)) => e.code(),
            _ => HRESULT(0),
        })
    }
    fn Cancel(&self) -> Result<()> {
        if let Some(op) = self.0.lock().unwrap().op.as_ref() {
            op.cancel();
        }
        Ok(())
    }
    fn Close(&self) -> Result<()> {
        Ok(())
    }
}

impl IAsyncOperation_Impl<ITransportStream> for ConnectOp_Impl {
    fn SetCompleted(
        &self,
        handler: windows_core::Ref<'_, AsyncOperationCompletedHandler<ITransportStream>>,
    ) -> Result<()> {
        let (fire, status) = {
            let mut g = self.0.lock().unwrap();
            if g.completed_assigned {
                return Err(windows_core::Error::from_hresult(E_ILLEGAL_DELEGATE_ASSIGNMENT));
            }
            g.completed_assigned = true;
            let h = handler.ok()?.clone();
            // Already finished: fire now rather than storing a handler that
            // nothing will ever call.
            if g.result.is_some() {
                (Some(h), g.status())
            } else {
                g.completed = Some(h);
                (None, g.status())
            }
        };
        if let Some(h) = fire {
            let sender: IAsyncOperation<ITransportStream> = self.to_interface();
            let _ = h.Invoke(&sender, status);
        }
        Ok(())
    }

    fn Completed(&self) -> Result<AsyncOperationCompletedHandler<ITransportStream>> {
        self.0
            .lock()
            .unwrap()
            .completed
            .clone()
            .ok_or_else(|| windows_core::Error::from_hresult(E_ILLEGAL_METHOD_CALL))
    }

    fn GetResults(&self) -> Result<ITransportStream> {
        match &self.0.lock().unwrap().result {
            Some(r) => r.clone(),
            None => Err(windows_core::Error::from_hresult(E_ILLEGAL_METHOD_CALL)),
        }
    }
}

/// E_ILLEGAL_METHOD_CALL — results read before completion.
const E_ILLEGAL_METHOD_CALL: HRESULT = HRESULT(0x8000000Eu32 as i32);
/// E_ILLEGAL_DELEGATE_ASSIGNMENT — `Completed` set twice.
const E_ILLEGAL_DELEGATE_ASSIGNMENT: HRESULT = HRESULT(0x80000018u32 as i32);

/// Builds an operation and arms `start`, which must deliver exactly one
/// result through the callback it is given.
pub(crate) fn connect_op(
    start: impl FnOnce(Box<dyn FnOnce(Result<ITransportStream>) + Send>) -> Result<OpHandle>,
) -> IAsyncOperation<ITransportStream> {
    let op: ComObject<ConnectOp> = ComObject::new(ConnectOp(
        Mutex::new(ConnectState {
            result: None,
            completed: None,
            completed_assigned: false,
            op: None,
        }),
        ModuleRef::new(),
    ));
    let sink = op.clone();
    match start(Box::new(move |res| sink.complete(res))) {
        Ok(handle) => {
            op.0.lock().unwrap().op = Some(handle);
            op.to_interface()
        }
        // Never armed, so nothing will call back: finish it here. Through
        // `op` rather than `IAsyncOperation::ready`, so the object handed
        // back is one this module counts -- see `comio::unload`.
        Err(e) => {
            op.complete(Err(e));
            op.to_interface()
        }
    }
}

#[cfg(test)]
mod tests {
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
}
