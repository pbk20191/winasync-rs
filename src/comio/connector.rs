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
//! # The cost of `spawn`
//!
//! Each `ConnectAsync` returns an `IAsyncOperation<IInspectable>` built with
//! `windows-future`'s `spawn`, which runs the connect to completion **on a
//! Windows thread-pool thread**. That thread is blocked for the duration.
//!
//! These are local transports — a named pipe, AF_UNIX, a Hyper-V socket — so
//! a connect normally succeeds or fails promptly, and the pipe client passes
//! its own wait timeout. The exception worth knowing is AF_HYPERV: a connect
//! to a partition that is not listening can sit for the provider's own
//! default, so pass `connect_timeout_ms` when that matters.
//!
//! The non-blocking alternative is the shape `super::asyncop::IoAsyncOp`
//! uses — complete the operation from the overlapped callback instead of
//! waiting on it — but that needs each connect rewritten from an `async fn`
//! into submit-plus-callback form. The interfaces here would not change if
//! that is done later.
// PascalCase members are the COM ABI convention these interfaces are shaped
// to, not Rust naming slips; `#[interface]` rejects an attribute of its own,
// so the allow lives here, as in `listener`.
#![allow(non_snake_case)]

use crate::common::block_on;
use crate::common::hvaddr::HvSocketAddr;
use core::ffi::c_void;
use windows_core::{
    implement, interface, ComObject, Interface, HRESULT, HSTRING, IInspectable, IUnknown,
    IUnknown_Vtbl,
};
use windows_future::IAsyncOperation;

/// E_POINTER — a null out-parameter from a COM caller.
const E_POINTER: HRESULT = HRESULT(0x80004003u32 as i32);

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
fn out_operation(op: IAsyncOperation<IInspectable>, result: *mut *mut c_void) -> HRESULT {
    if result.is_null() {
        return E_POINTER;
    }
    // Transfer the reference to the caller.
    unsafe { *result = core::mem::ManuallyDrop::new(op).as_raw() };
    HRESULT(0)
}

/// Connects to named pipes.
#[implement(INamedPipeConnector)]
pub struct NamedPipeConnector;

impl NamedPipeConnector {
    #[allow(clippy::new_without_default)]
    pub fn new() -> ComObject<NamedPipeConnector> {
        ComObject::new(NamedPipeConnector)
    }

    /// Connects to `name`, resolving to the duplex WinRT stream.
    pub fn connect(&self, name: &HSTRING) -> IAsyncOperation<IInspectable> {
        let name = name.to_string();
        IAsyncOperation::spawn(move || block_on(super::NamedPipeClient::connect(&name)))
    }
}

impl INamedPipeConnector_Impl for NamedPipeConnector_Impl {
    unsafe fn ConnectAsync(
        &self,
        name: core::mem::ManuallyDrop<HSTRING>,
        result: *mut *mut c_void,
    ) -> HRESULT {
        out_operation(self.connect(&name), result)
    }
}

/// Connects to AF_UNIX sockets.
#[implement(IUnixConnector)]
pub struct UnixConnector;

impl UnixConnector {
    #[allow(clippy::new_without_default)]
    pub fn new() -> ComObject<UnixConnector> {
        ComObject::new(UnixConnector)
    }

    /// Connects to the socket at `path`, resolving to the duplex WinRT
    /// stream.
    pub fn connect(&self, path: &HSTRING) -> IAsyncOperation<IInspectable> {
        let path = path.to_string();
        IAsyncOperation::spawn(move || block_on(super::UnixStreamSocket::connect(&path)))
    }
}

impl IUnixConnector_Impl for UnixConnector_Impl {
    unsafe fn ConnectAsync(
        &self,
        path: core::mem::ManuallyDrop<HSTRING>,
        result: *mut *mut c_void,
    ) -> HRESULT {
        out_operation(self.connect(&path), result)
    }
}

/// Connects to Hyper-V sockets.
#[implement(IHyperVConnector)]
pub struct HyperVConnector;

impl HyperVConnector {
    #[allow(clippy::new_without_default)]
    pub fn new() -> ComObject<HyperVConnector> {
        ComObject::new(HyperVConnector)
    }

    /// Connects to `service_id` on the partition `vm_id`, resolving to the
    /// duplex WinRT stream.
    ///
    /// A connect to a partition that is not listening can sit for the
    /// provider's own default; the Rust API's `connect_timeout_ms` is the
    /// way to bound that, and is not exposed here.
    pub fn connect(
        &self,
        vm_id: &windows_core::GUID,
        service_id: &windows_core::GUID,
    ) -> IAsyncOperation<IInspectable> {
        let addr = HvSocketAddr::new(*vm_id, *service_id);
        IAsyncOperation::spawn(move || block_on(super::HvStreamSocket::connect(addr)))
    }
}

impl IHyperVConnector_Impl for HyperVConnector_Impl {
    unsafe fn ConnectAsync(
        &self,
        vm_id: *const windows_core::GUID,
        service_id: *const windows_core::GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        if vm_id.is_null() || service_id.is_null() {
            return E_POINTER;
        }
        out_operation(
            self.connect(unsafe { &*vm_id }, unsafe { &*service_id }),
            result,
        )
    }
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
        let connector = NamedPipeConnector::new();
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
}
