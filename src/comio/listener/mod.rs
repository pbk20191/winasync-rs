//! WinRT-native StreamSocketListener-shaped listener over comio transports.
#![allow(dead_code)]
// This whole file is a COM/WinRT surface: PascalCase members are the ABI
// convention it is shaped to, not Rust naming slips. The `#[interface]` macro
// rejects an attribute of its own, so the allow lives here.
#![allow(non_snake_case)]

use crate::comio::unload::ModuleRef;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use windows::Foundation::{IClosable_Impl, TypedEventHandler};
use windows_core::{ComObject, Error, HRESULT, HSTRING, Result, implement};
use windows_future::IAsyncAction;

use super::bindings;
use super::bindings::Pbk20191::WinAsync::Comio::{
    ConnectionReceivedArgs as ConnectionReceivedArgsClass, IConnectionListener,
    IConnectionReceivedArgs,
    IConnectionListener_Impl, IConnectionReceivedArgs_Impl, ITransport_Impl, IHyperVBind_Impl,
    IListenerSecurity_Impl, INamedPipeBind_Impl, INamedPipeListenerSettings_Impl,
    ITransportStream, IUnixBind_Impl,
};
use crate::common::pipe::{NamedPipeOptions, PipeServer};
use windows::Win32::System::Pipes::PIPE_UNLIMITED_INSTANCES;
use crate::common::security::{PeerIdentity, SecurityDescriptor};
// Queried for by the tests below, which check that each listener exposes only
// the bind its addressing supports.
#[cfg(test)]
use super::bindings::Pbk20191::WinAsync::Comio::{IHyperVBind, INamedPipeBind, ITransport, IUnixBind};
use super::tpio::OpHandle;
use crate::common::hvaddr::{HvSocketAddr, HV_GUID_WILDCARD};

/// RO_E_CLOSED — returned by members of a closed listener/stream.
pub(crate) const RO_E_CLOSED: HRESULT = HRESULT(0x80000013u32 as i32);

/// E_POINTER — a null out-parameter from a COM caller.
const E_POINTER: HRESULT = HRESULT(0x80004003u32 as i32);

/// E_ILLEGAL_METHOD_CALL — a member called in a state it does not apply to.
pub(crate) const E_ILLEGAL_METHOD_CALL: HRESULT = HRESULT(0x8000000Eu32 as i32);

// The interfaces are no longer declared here: they live in
// `metadata/winasync.rdl` and are generated into `super::bindings`.
// A hand-written `#[interface]` derives from `IUnknown` and cannot derive
// from `IInspectable` at all, so it could never have been a WinRT interface
// — see that file's header for the mechanical reason.

/// The handler every listener raises `ConnectionReceived` with.
///
/// The sender is the *interface*, not any one listener class, which is what
/// lets all three share one event in metadata.
pub(crate) type ConnectionHandler =
    TypedEventHandler<IConnectionListener, IConnectionReceivedArgs>;

#[implement(bindings::Pbk20191::WinAsync::Comio::ConnectionReceivedArgs)]
struct ConnectionReceivedArgs {
    stream: ITransportStream,
    endpoint: HSTRING,
    peer: PeerIdentity,
    _module: ModuleRef,
}

impl IConnectionReceivedArgs_Impl for ConnectionReceivedArgs_Impl {
    fn Stream(&self) -> Result<ITransportStream> {
        Ok(self.stream.clone())
    }

    fn RemoteEndpoint(&self) -> Result<HSTRING> {
        Ok(self.endpoint.clone())
    }

    fn RemoteProcessId(&self) -> Result<u32> {
        Ok(self.peer.process_id)
    }

    fn RemoteUserSid(&self) -> Result<HSTRING> {
        Ok(HSTRING::from(&self.peer.user_sid))
    }
}

/// One accepted connection, as a transport hands it back.
///
/// `remote` is the *peer's* endpoint when the transport can name one. Only
/// AF_HYPERV can today: a Hyper-V peer carries the partition it came from,
/// which is the thing a host service authorizes on. A named pipe or an
/// AF_UNIX client has no address to report, so those pass `None` and the
/// event falls back to the listener's own bound endpoint, as before.
///
/// `peer` is the other half of that: who connected, for the transports that
/// have no address but do have a process and an account (named pipes,
/// AF_UNIX). Read on the accept path, while the connection is fresh.
pub(crate) struct Accepted {
    pub(crate) stream: ITransportStream,
    pub(crate) remote: Option<HSTRING>,
    pub(crate) peer: PeerIdentity,
}

impl Accepted {
    /// A connection from a local transport: no peer address, but a peer
    /// process and account.
    pub(crate) fn local(stream: ITransportStream, peer: PeerIdentity) -> Self {
        Self {
            stream,
            remote: None,
            peer,
        }
    }
}

/// Constructs the `Pbk20191.WinAsync.Comio.ConnectionReceivedArgs` the event carries.
pub(crate) fn new_args(
    stream: ITransportStream,
    endpoint: HSTRING,
    peer: PeerIdentity,
) -> ConnectionReceivedArgsClass {
    ConnectionReceivedArgs {
        stream,
        endpoint,
        peer,
        _module: ModuleRef::new(),
    }
    .into()
}

#[cfg(test)]
mod tests;

/// Who may connect (`IListenerSecurity`), who did (`RemoteProcessId` /
/// `RemoteUserSid`), and the pipe-name policy, through the ABI a projection
/// uses.
#[cfg(test)]
mod security_tests;

#[cfg(test)]
mod chain_tests;

/// `INamedPipeListenerSettings`, `IMessageStream` and the connector's
/// timeout, through the ABI a projection uses.
#[cfg(test)]
mod pipe_settings_tests;

mod chain;
mod classes;
mod source;
mod state;
use chain::*;
pub use classes::*;
pub(crate) use source::*;
pub(crate) use state::*;
