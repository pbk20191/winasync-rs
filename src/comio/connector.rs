//! COM connectors: the client half of [`super::listener`].
//!
//! The listeners are reachable from COM; without these the transports were
//! only half exposed — an external caller could accept a connection but never
//! make one. Same shape as the listeners: one concrete type per transport,
//! each carrying the connect its addressing supports, because a pipe name, a
//! filesystem path and a pair of GUIDs cannot share one signature.
//!
//! There is deliberately no common connector interface. The listeners share
//! one because they share real behaviour — handler registration, the accept
//! chain, close. A connector has exactly one method, and that method is the
//! part that differs, so a shared interface would carry nothing.
//!
//! # No blocked threads
//!
//! Each `ConnectAsync` returns an `IAsyncOperation<IInspectable>` whose
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

// PascalCase members are the COM ABI convention these interfaces are shaped
// to, not Rust naming slips; `#[interface]` rejects an attribute of its own,
// so the allow lives here, as in `listener`.
#![allow(non_snake_case)]

use core::ffi::c_void;
use std::sync::Mutex;
use windows_core::{
    implement, interface, ComObject, HRESULT, HSTRING, IInspectable, IUnknown, IUnknownImpl,
    IUnknown_Vtbl, Interface, Result,
};
use windows_future::{
    AsyncOperationCompletedHandler, AsyncStatus, IAsyncInfo_Impl, IAsyncOperation,
    IAsyncOperation_Impl,
};

use super::tpio::OpHandle;

/// E_POINTER — a null out-parameter from a COM caller.
pub(crate) const E_POINTER: HRESULT = HRESULT(0x80004003u32 as i32);

/// What every connector here shares. GUID frozen — do not change or reuse.
///
/// `ConnectAsync` is deliberately **not** on it. The three take different
/// addresses — a pipe name, a filesystem path, a pair of GUIDs — and the only
/// way to put them on one signature is to erase the address into a variant
/// and reject the wrong shapes at run time, which is exactly the
/// `E_INVALIDARG` design the listeners were just freed from.
///
/// What it does carry is identity: a COM caller holding an unknown object can
/// ask "is this a connector, and of what?" in one `QueryInterface` instead of
/// trying each transport's interface in turn. The answer is also what to put
/// in a log line.
#[interface("6ec93a18-74d2-41b5-9c08-3f5ab0e712d6")]
pub unsafe trait IStreamConnector: IUnknown {
    /// A stable, lowercase transport name: `namedpipe`, `unix`, `hyperv`.
    pub fn Transport(&self, result: *mut core::mem::ManuallyDrop<HSTRING>) -> HRESULT;
}

/// Connects to a named pipe. GUID frozen — do not change or reuse.
#[interface("8d3f21b6-0c47-4e95-a1d8-6b92f05c7e34")]
pub unsafe trait INamedPipeConnector: IUnknown {
    pub fn ConnectAsync(
        &self,
        name: core::mem::ManuallyDrop<HSTRING>,
        result: *mut *mut c_void,
    ) -> HRESULT;
}

/// Connects to an AF_UNIX socket. GUID frozen.
#[interface("2b70e4c9-5a16-43d8-9f02-c7e418a6d35f")]
pub unsafe trait IUnixConnector: IUnknown {
    pub fn ConnectAsync(
        &self,
        path: core::mem::ManuallyDrop<HSTRING>,
        result: *mut *mut c_void,
    ) -> HRESULT;
}

/// Connects to a Hyper-V socket. GUID frozen.
#[interface("f51a8d02-9e3b-4c67-85a4-1d60b7f293ce")]
pub unsafe trait IHyperVConnector: IUnknown {
    pub fn ConnectAsync(
        &self,
        vm_id: *const windows_core::GUID,
        service_id: *const windows_core::GUID,
        result: *mut *mut c_void,
    ) -> HRESULT;
}

/// Hands an `IAsyncOperation` back through a vtable out-parameter.
pub(crate) fn out_operation(op: IAsyncOperation<IInspectable>, result: *mut *mut c_void) -> HRESULT {
    if result.is_null() {
        return E_POINTER;
    }
    // Transfer the reference to the caller.
    unsafe { *result = core::mem::ManuallyDrop::new(op).as_raw() };
    HRESULT(0)
}


/// An `IAsyncOperation<IInspectable>` whose result arrives from an overlapped
/// completion rather than from a closure run to completion on a thread.
///
/// The same state machine as `super::asyncop::IoAsyncOp`, which does this for
/// reads and writes; this one is fixed to `IAsyncOperation<IInspectable>`,
/// the shape a connect returns. `IAsyncInfo::Cancel` cancels the in-flight
/// `ConnectEx`.
struct ConnectState {
    result: Option<Result<IInspectable>>,
    completed: Option<AsyncOperationCompletedHandler<IInspectable>>,
    completed_assigned: bool,
    op: Option<OpHandle>,
}

#[implement(IAsyncOperation<IInspectable>, windows_future::IAsyncInfo)]
struct ConnectOp(Mutex<ConnectState>);

// SAFETY: the only non-`Send` data (the pending handler) lives behind the
// `Mutex` and every access is serialized by it; the object is agile, so its
// interfaces may be used from the completion callback's thread. Mirrors
// `asyncop::IoAsyncOp`.
unsafe impl Send for ConnectOp {}
unsafe impl Sync for ConnectOp {}

impl ConnectOp_Impl {
    /// Stores the outcome and fires `Completed` if one is already assigned.
    /// Runs on the thread-pool completion thread.
    fn complete(&self, res: Result<IInspectable>) {
        let (status, completed) = {
            let mut g = self.0.lock().unwrap();
            debug_assert!(g.result.is_none());
            g.result = Some(res);
            (g.status(), g.completed.take())
        };
        // Outside the lock: the handler may re-enter to read Status/GetResults.
        if let Some(completed) = completed {
            let sender: IAsyncOperation<IInspectable> = self.to_interface();
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

impl IAsyncOperation_Impl<IInspectable> for ConnectOp_Impl {
    fn SetCompleted(
        &self,
        handler: windows_core::Ref<'_, AsyncOperationCompletedHandler<IInspectable>>,
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
            let sender: IAsyncOperation<IInspectable> = self.to_interface();
            let _ = h.Invoke(&sender, status);
        }
        Ok(())
    }

    fn Completed(&self) -> Result<AsyncOperationCompletedHandler<IInspectable>> {
        self.0
            .lock()
            .unwrap()
            .completed
            .clone()
            .ok_or_else(|| windows_core::Error::from_hresult(E_ILLEGAL_METHOD_CALL))
    }

    fn GetResults(&self) -> Result<IInspectable> {
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
    start: impl FnOnce(Box<dyn FnOnce(Result<IInspectable>) + Send>) -> Result<OpHandle>,
) -> IAsyncOperation<IInspectable> {
    let op: ComObject<ConnectOp> = ComObject::new(ConnectOp(Mutex::new(ConnectState {
        result: None,
        completed: None,
        completed_assigned: false,
        op: None,
    })));
    let sink = op.clone();
    match start(Box::new(move |res| sink.complete(res))) {
        Ok(handle) => {
            op.0.lock().unwrap().op = Some(handle);
            op.to_interface()
        }
        // Never armed, so nothing will call back: finish it here.
        Err(e) => IAsyncOperation::ready(Err(e)),
    }
}

/// Fills `IStreamConnector::Transport`.
pub(crate) fn out_transport(
    name: &str,
    result: *mut core::mem::ManuallyDrop<HSTRING>,
) -> HRESULT {
    if result.is_null() {
        return E_POINTER;
    }
    unsafe { *result = core::mem::ManuallyDrop::new(HSTRING::from(name)) };
    HRESULT(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::listener::{
        IConnectionListener, INamedPipeListenerBind, NamedPipeConnectionListener,
    };
    use std::sync::mpsc;
    use windows::Foundation::TypedEventHandler;
    use windows_core::Interface;

    /// The AF_UNIX connector, whose result arrives from the `ConnectEx`
    /// completion rather than from a blocked thread. Exercises the
    /// `ConnectOp` state machine: `SetCompleted` before the result lands,
    /// then `GetResults` after.
    #[test]
    fn unix_connector_completes_from_the_overlapped_callback() {
        use crate::comio::listener::{IUnixListenerBind, UnixConnectionListener};

        let path = std::env::temp_dir()
            .join(format!("comio_conn_{}.sock", std::process::id()))
            .to_string_lossy()
            .into_owned();
        let _ = std::fs::remove_file(&path);

        let listener = UnixConnectionListener::new();
        let (accepted_tx, accepted_rx) = mpsc::channel::<()>();
        let handler = TypedEventHandler::<IInspectable, IInspectable>::new(move |_s, _a| {
            let _ = accepted_tx.send(());
            Ok(())
        });
        {
            let b: IUnixListenerBind = listener.cast().unwrap();
            let mut action = core::ptr::null_mut();
            unsafe {
                b.BindAsync(
                    core::mem::ManuallyDrop::new(HSTRING::from(&path)),
                    &mut action,
                )
                .ok()
                .expect("BindAsync");
                drop(windows_future::IAsyncAction::from_raw(action));
            }
        }
        let l: IConnectionListener = listener.cast().unwrap();
        let mut token = 0i64;
        unsafe {
            l.ConnectionReceived(handler.as_raw(), &mut token)
                .ok()
                .expect("ConnectionReceived");
        }

        let connector = crate::comio::UnixStreamSocket::new();
        let c: IUnixConnector = connector.cast().unwrap();
        let op: IAsyncOperation<IInspectable> = unsafe {
            let mut raw = core::ptr::null_mut();
            c.ConnectAsync(
                core::mem::ManuallyDrop::new(HSTRING::from(&path)),
                &mut raw,
            )
            .ok()
            .expect("ConnectAsync");
            IAsyncOperation::from_raw(raw)
        };

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
        let handler = TypedEventHandler::<IInspectable, IInspectable>::new(move |_s, args| {
            let a = args.as_ref().unwrap();
            let cra: crate::comio::IConnectionReceivedArgs = a.cast().unwrap();
            let raw = unsafe {
                let mut raw = core::ptr::null_mut();
                cra.Stream(&mut raw).ok()?;
                raw as usize
            };
            let _ = tx.send(raw);
            Ok(())
        });

        // Bind and register through the listener's vtables.
        let bind: INamedPipeListenerBind = listener.cast().unwrap();
        let mut action = core::ptr::null_mut();
        unsafe {
            bind.BindAsync(
                core::mem::ManuallyDrop::new(HSTRING::from(&name)),
                &mut action,
            )
            .ok()
            .expect("BindAsync");
            drop(windows_future::IAsyncAction::from_raw(action));
        }
        let l: IConnectionListener = listener.cast().unwrap();
        let mut token = 0i64;
        unsafe {
            l.ConnectionReceived(handler.as_raw(), &mut token)
                .ok()
                .expect("ConnectionReceived");
        }

        // Connect through the connector's vtable.
        let connector = crate::comio::NamedPipeClient::new();
        let c: INamedPipeConnector = connector.cast().unwrap();
        let client: IInspectable = unsafe {
            let mut op = core::ptr::null_mut();
            c.ConnectAsync(
                core::mem::ManuallyDrop::new(HSTRING::from(&name)),
                &mut op,
            )
            .ok()
            .expect("ConnectAsync");
            IAsyncOperation::<IInspectable>::from_raw(op)
        }
        .join()
        .expect("connect result");

        let server_raw = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("no ConnectionReceived");
        let server = unsafe { IInspectable::from_raw(server_raw as *mut c_void) };

        // Both ends are real duplex streams.
        assert!(server.cast::<windows::Storage::Streams::IInputStream>().is_ok());
        assert!(client.cast::<windows::Storage::Streams::IOutputStream>().is_ok());

        let closable: windows::Foundation::IClosable = l.cast().unwrap();
        closable.Close().unwrap();
    }

    /// The one interface all three clients share. A caller that holds an
    /// `IStreamConnector` learns which transport it has from `Transport`,
    /// then asks for that transport's connect interface by `QueryInterface`
    /// — the discovery step that replaces a connect member here.
    ///
    /// Nothing is connected: construction and identity are the whole test,
    /// so it needs no listener, no pipe and no Hyper-V provider.
    #[test]
    fn every_client_reports_its_transport_through_the_shared_interface() {
        fn transport_of(c: &IStreamConnector) -> String {
            let mut s = core::mem::ManuallyDrop::new(HSTRING::new());
            unsafe { c.Transport(&mut s).ok().expect("Transport") };
            core::mem::ManuallyDrop::into_inner(s).to_string()
        }

        let pipe: IStreamConnector = crate::comio::NamedPipeClient::new().cast().unwrap();
        let unix: IStreamConnector = crate::comio::UnixStreamSocket::new().cast().unwrap();
        let hv: IStreamConnector = crate::comio::HvStreamSocket::new().cast().unwrap();

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
