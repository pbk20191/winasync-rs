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

/// How a listener issues one accept.
///
/// A trait rather than an enum because each transport is now its own COM
/// type: the shared accept chain only ever needs "arm one accept", and
/// keeping that the whole contract is what lets the three listeners share
/// every other line of this module.
///
/// No transport does kernel work when it is bound. The named-pipe source
/// holds only the name until its first `arm` (which creates the first
/// instance, and from the first connection on keeps one in reserve); the socket
/// sources hold the address and bind/listen *lazily* on the first `arm`, then
/// reuse the one listen socket. Deferring that bind is what makes handler
/// gating complete: with no handler the chain never arms, so no listen socket
/// exists and a client cannot connect into a backlog. The cost is that a bind
/// error — a stale AF_UNIX socket file, `WSAEAFNOSUPPORT` where the Hyper-V
/// provider is absent — surfaces when the chain first starts rather than from
/// the bind call.
///
/// Held behind an `Arc` in `ListenerState::source` and cloned out before use,
/// so `try_arm` can drop that lock before arming.
/// Where a listener's connections come from: one of the three transports.
pub(crate) enum AcceptSource {
    /// The pipe's name and its instances. Dropping it -- on `Close`, or
    /// when a rebind replaces it -- closes the instance held in reserve,
    /// which is what gives the name up.
    Pipe(Arc<PipeServer>),
    Unix(UnixSource),
    HyperV(HyperVSource),
}

impl AcceptSource {
    /// Issues one accept through the transport's callback-based primitive
    /// (`pipe::arm_accept` / `*StreamListener::arm_accept`), each built
    /// directly on `tpio::submit` — no `Future`, no executor.
    ///
    /// `Ok(None)` means nothing could be armed yet -- a named pipe at its
    /// instance limit -- and `on_result` will be given
    /// [`super::pipe::SLOT_FREED`] once something can be.
    fn arm(&self, on_result: AcceptSink) -> Result<Option<OpHandle>> {
        match self {
            AcceptSource::Pipe(server) => super::pipe::arm_accept(server, on_result),
            AcceptSource::Unix(source) => {
                let mut guard = source.listener.lock().unwrap();
                if guard.is_none() {
                    *guard = Some(super::UnixStreamListener::bind(
                        &source.path,
                        source.security.as_ref(),
                    )?);
                }
                guard.as_ref().unwrap().arm_accept(on_result).map(Some)
            }
            AcceptSource::HyperV(source) => {
                let mut guard = source.listener.lock().unwrap();
                if guard.is_none() {
                    *guard = Some(super::HvStreamListener::bind(source.addr)?);
                }
                guard.as_ref().unwrap().arm_accept(on_result).map(Some)
            }
        }
    }
}

pub(crate) struct UnixSource {
    path: String,
    security: Option<SecurityDescriptor>,
    /// `None` until the first `arm` binds the persistent listen socket.
    listener: Mutex<Option<super::UnixStreamListener>>,
}

pub(crate) struct HyperVSource {
    addr: HvSocketAddr,
    listener: Mutex<Option<super::HvStreamListener>>,
}

/// Where an armed accept delivers: the listener's chain, which fires the
/// event and re-arms (see [`drive`]). One per armed accept; delivered
/// exactly once, by the completion or by the slot waiter that parked it.
pub(crate) struct AcceptSink {
    state: Arc<ListenerState>,
}

/// Shared, `Send + Sync` state behind a [`ConnectionListener`]: held both by
/// the COM object itself and by the closures driving its self-re-arming
/// accept chain (see [`drive`]), so `Close` can flip `closed`/cancel the
/// in-flight accept out from under it, and the chain can tell it has been
/// asked to stop.
///
/// # Handler gating
/// The accept chain runs only while the listener is *wanted running*
/// ([`want_running`]): not closed, bound (`source` is `Some`), *and* at least
/// one `ConnectionReceived` handler is registered. Binding alone no longer
/// accepts connections — without a consumer, an accepted stream would just be
/// dropped. The chain is (re)started by whichever of `BindAsync` /
/// `ConnectionReceived` completes that condition, and paused — the in-flight
/// accept cancelled — when the last handler is removed.
///
/// The whole chain is driven by one non-recursive loop ([`drive`]); `try_arm`
/// issues a single accept and never manages lifecycle itself, so there is no
/// `drive`/`arm`/stop mutual recursion. Two atomics coordinate starts:
/// * `running` — "a chain is logically active"; CAS'd `false -> true` exactly
///   once per chain start and back to `false` exactly once when it stops.
/// * `restart_pending` — set when a would-be starter loses the `running` CAS to
///   a chain that is already (or concurrently) stopping; the stopping chain
///   consumes it on release and restarts, closing the add-races-stop window
///   *without* restarting on every failure (so a persistent hard accept error
///   can never hot-spin the pool). The set-then-recheck handshake across these
///   two atomics uses `SeqCst` so a start request can never be lost.
struct ListenerState {
    closed: AtomicBool,
    event: windows_core::Event<ConnectionHandler>,
    endpoint: Mutex<Option<HSTRING>>,
    /// Set once by `BindAsync`, read (cloned) by every `try_arm` call.
    source: Mutex<Option<Arc<AcceptSource>>>,
    /// The in-flight accept's `OpHandle`. `tpio::submit` guarantees every
    /// completion — synchronous or asynchronous — is delivered on a
    /// thread-pool thread (see `tpio::submit`'s doc comment), so `try_arm`
    /// never recurses/re-enters on the caller's own stack and a plain
    /// `Option<OpHandle>` (no sequence tag) is sufficient here.
    current_op: Mutex<Option<OpHandle>>,
    /// Live `ConnectionReceived` tokens — `windows_core::Event` exposes no
    /// count, so handler presence (the gating predicate) is tracked here.
    handlers: Mutex<HashSet<i64>>,
    /// Who may connect, set through `IListenerSecurity` before the bind that
    /// uses it. `None` is the transport's default.
    security: Mutex<Option<SecurityDescriptor>>,
    /// How a named pipe's instances are made, set through
    /// `INamedPipeListenerSettings` before the bind. Its `security` is unused:
    /// that comes from the field above when the bind combines the two.
    pipe_options: Mutex<NamedPipeOptions>,
    /// Whether an accept chain is logically active. See the type-level
    /// "Handler gating" note.
    running: AtomicBool,
    /// A start was requested while `running` was still held by a stopping
    /// chain; the chain restarts once on release. See the "Handler gating" note.
    restart_pending: AtomicBool,
}

impl ListenerState {
    fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            event: windows_core::Event::new(),
            endpoint: Mutex::new(None),
            source: Mutex::new(None),
            current_op: Mutex::new(None),
            handlers: Mutex::new(HashSet::new()),
            security: Mutex::new(None),
            pipe_options: Mutex::new(NamedPipeOptions::new()),
            running: AtomicBool::new(false),
            restart_pending: AtomicBool::new(false),
        }
    }
}

/// Shared behaviour of every listener here, held by each transport's COM type.
///
/// Not itself a COM object: the three listeners differ only in how they are
/// addressed, so everything else — handler registration, the self-re-arming
/// accept chain, teardown — lives here once and each type delegates to it.
pub(crate) struct ListenerCore {
    state: Arc<ListenerState>,
}

impl ListenerCore {
    fn new() -> Self {
        Self { state: Arc::new(ListenerState::new()) }
    }

    /// Registers `handler`, invoked on a thread-pool thread per accepted
    /// connection; returns a token for `remove_connection_received`.
    ///
    /// Registering the *first* handler starts the accept chain if the
    /// listener is already bound (handler gating — see [`ListenerState`]): a
    /// listener with no consumer does not accept connections.
    ///
    /// If starting the chain fails synchronously — a malformed endpoint bound
    /// before any handler existed, or a stale AF_UNIX socket file — that
    /// error surfaces here and the registration is rolled back, leaving the
    /// listener exactly as it was. Bind-after-handler ordering surfaces the
    /// same error from the bind instead.
    fn connection_received(
        &self,
        handler: &ConnectionHandler,
    ) -> Result<i64> {
        add_handler(&self.state, handler)
    }

    /// Removes a handler. Removing the last one pauses the chain and cancels
    /// the in-flight accept.
    fn remove_connection_received(&self, token: i64) -> Result<()> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }
        self.state.event.remove(token);
        let last = {
            let mut h = self.state.handlers.lock().unwrap();
            h.remove(&token);
            h.is_empty()
        };
        if last {
            // No consumers left: cancel the in-flight accept. Its `on_result`
            // observes the cancellation and releases the chain.
            if let Some(op) = self.state.current_op.lock().unwrap().take() {
                op.cancel();
            }
        }
        Ok(())
    }

    /// The endpoint this listener is bound to, or `RO_E_CLOSED` once closed.
    fn information(&self) -> Result<HSTRING> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }
        Ok(self
            .state
            .endpoint
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_default())
    }

    fn close(&self) -> Result<()> {
        self.state.closed.store(true, Ordering::Release);
        if let Some(op) = self.state.current_op.lock().unwrap().take() {
            op.cancel();
        }
        self.state.event.clear();
        self.state.handlers.lock().unwrap().clear();
        // Let go of the endpoint: a pipe source holds a listening instance in
        // reserve, and the name stays ours until that is closed.
        self.state.source.lock().unwrap().take();
        Ok(())
    }

    /// The descriptor set through `IListenerSecurity`, as SDDL; empty for
    /// the default.
    fn security_descriptor(&self) -> Result<HSTRING> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }
        Ok(self
            .state
            .security
            .lock()
            .unwrap()
            .as_ref()
            .map(|sd| HSTRING::from(sd.sddl()))
            .unwrap_or_default())
    }

    /// Sets who may connect. Only before the first bind: what is already
    /// listening keeps the descriptor it was created with, and a setting that
    /// silently applied to some instances and not others would be worse than
    /// none. Malformed SDDL is `E_INVALIDARG`.
    fn set_security_descriptor(&self, sddl: &HSTRING) -> Result<()> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }
        if self.state.source.lock().unwrap().is_some() {
            return Err(Error::new(
                E_ILLEGAL_METHOD_CALL,
                "set SecurityDescriptor before binding",
            ));
        }
        let parsed = if sddl.is_empty() {
            None
        } else {
            Some(SecurityDescriptor::from_sddl(&sddl.to_string()).map_err(|e| {
                Error::new(
                    windows::Win32::Foundation::E_INVALIDARG,
                    format!("not a security descriptor: {}", e.message()),
                )
            })?)
        };
        *self.state.security.lock().unwrap() = parsed;
        Ok(())
    }

    /// The descriptor a bind should use.
    fn security(&self) -> Option<SecurityDescriptor> {
        self.state.security.lock().unwrap().clone()
    }

    /// The named-pipe settings, as `INamedPipeListenerSettings` reads them.
    fn pipe_options(&self) -> Result<NamedPipeOptions> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }
        Ok(self.state.pipe_options.lock().unwrap().clone())
    }

    /// Changes one named-pipe setting. Only before the first bind, for the
    /// reason [`set_security_descriptor`](Self::set_security_descriptor)
    /// gives.
    fn update_pipe_options(
        &self,
        update: impl FnOnce(NamedPipeOptions) -> Result<NamedPipeOptions>,
    ) -> Result<()> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }
        if self.state.source.lock().unwrap().is_some() {
            return Err(Error::new(
                E_ILLEGAL_METHOD_CALL,
                "change named-pipe settings before binding",
            ));
        }
        let mut options = self.state.pipe_options.lock().unwrap();
        *options = update(options.clone())?;
        Ok(())
    }

    /// Shared tail of every bind: swap in the accept source and (re)start the
    /// chain if a consumer is already registered.
    ///
    /// Rebinding while a chain is running is not supported: the `current_op`
    /// handoff assumes one active chain, and a concurrent rebind could cancel
    /// the wrong op and orphan an in-flight accept. Serialize binds.
    ///
    /// Note an error-surfacing asymmetry on *rebind*: a fresh bind issues the
    /// first accept inline and surfaces its error, but a rebind over a
    /// running chain cannot claim `running` synchronously, so the new
    /// source's first accept happens on the restart path and its error is not
    /// returned from here. Rebind after an explicit `Close`, or onto a fresh
    /// listener, if you need that error synchronously.
    fn install_source(&self, source: Arc<AcceptSource>, label: HSTRING) -> Result<IAsyncAction> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }
        // A second bind replacing an in-flight chain must not orphan an
        // uncancellable accept: cancel whatever is armed before installing.
        if let Some(op) = self.state.current_op.lock().unwrap().take() {
            op.cancel();
        }
        *self.state.source.lock().unwrap() = Some(source);
        *self.state.endpoint.lock().unwrap() = Some(label);

        // Start accepting only if a consumer is already registered; otherwise
        // the chain starts on the first `ConnectionReceived`.
        ensure_running(self.state.clone())?;
        Ok(super::settled::action_done())
    }
}

impl Drop for ListenerCore {
    /// Breaks the `Arc<ListenerState>` <-> accept-chain reference cycle: the
    /// in-flight accept's completion closure (captured by the `Op` block
    /// `tpio::submit` allocates, held alive by the kernel until the real
    /// completion fires) captures a clone of `state`, so without this the
    /// last COM reference going away would leak the state, the pending
    /// overlapped op, and — if a client ever connected — resume firing events
    /// on a listener nobody references any more. The chain captures
    /// `Arc<ListenerState>`, never the COM pointer, so this is not part of
    /// the COM refcount cycle and is guaranteed to run once the refcount
    /// reaches zero even if `Close` was never called. Idempotent with an
    /// explicit `Close`.
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// Generates one transport's COM type: the shared members on
/// [`IConnectionListener`], `Close` on `IClosable`, and inherent Rust
/// equivalents so Rust callers need no `cast`.
///
/// Every body delegates to [`ListenerCore`]; only the impl blocks repeat,
/// because `#[implement]` generates a per-type trait that cannot be blanket
/// implemented. The `_Impl` name is passed in rather than pasted, to avoid a
/// proc-macro dependency for one concatenation.
macro_rules! listener_impls {
    ($name:ident, $impl_name:ident) => {
        impl $name {
            /// Creates the listener. Nothing is created on disk or in the
            /// kernel until it is bound *and* has a handler.
            #[allow(clippy::new_without_default)]
            pub fn new() -> ComObject<$name> {
                ComObject::new($name {
                    core: ListenerCore::new(),
                    _module: ModuleRef::new(),
                })
            }

            /// Typed Rust equivalent of the vtable's `ConnectionReceived`.
            ///
            /// These three are `snake_case` while the binds below keep their
            /// WinRT-mirroring names, because only these three also exist on
            /// [`IConnectionListener`]: `ComObject` derefs through the
            /// generated `_Impl` type, so same-named inherent methods would
            /// be shadowed by the raw vtable ones. Separate names keep the
            /// typed surface and the ABI surface from colliding.
            pub fn connection_received(&self, handler: &ConnectionHandler) -> Result<i64> {
                self.core.connection_received(handler)
            }

            /// Typed Rust equivalent of the vtable's
            /// `RemoveConnectionReceived`.
            pub fn remove_connection_received(&self, token: i64) -> Result<()> {
                self.core.remove_connection_received(token)
            }

            /// Typed Rust equivalent of the vtable's `Information`.
            pub fn information(&self) -> Result<HSTRING> {
                self.core.information()
            }
        }

        // The generated binding borrows the handler as a `Ref`, carries the
        // token as the return value and the failure as an `Err`, so every
        // null check and hand-rolled `HRESULT` that used to live here is now
        // the projection's problem rather than ours.
        impl IConnectionListener_Impl for $impl_name {
            fn ConnectionReceived(
                &self,
                handler: windows_core::Ref<'_, ConnectionHandler>,
            ) -> Result<i64> {
                self.core.connection_received(handler.ok()?)
            }

            fn RemoveConnectionReceived(&self, token: i64) -> Result<()> {
                self.core.remove_connection_received(token)
            }

            fn Information(&self) -> Result<HSTRING> {
                self.core.information()
            }
        }

        impl IClosable_Impl for $impl_name {
            fn Close(&self) -> Result<()> {
                self.core.close()
            }
        }
    };
}

/// A listener that accepts named-pipe connections.
///
/// Binding creates nothing on its own: the accept chain — and with it any
/// pipe instance — starts only once the listener is both bound and has a
/// `ConnectionReceived` handler.
#[implement(
    bindings::Pbk20191::WinAsync::Comio::NamedPipeConnectionListener,
    bindings::Pbk20191::WinAsync::Comio::ITransport,
    bindings::Pbk20191::WinAsync::Comio::INamedPipeBind,
    bindings::Pbk20191::WinAsync::Comio::IListenerSecurity,
    bindings::Pbk20191::WinAsync::Comio::INamedPipeListenerSettings,
    windows::Foundation::IClosable
)]
pub struct NamedPipeConnectionListener {
    core: ListenerCore,
    _module: ModuleRef,
}

/// A listener that accepts AF_UNIX (Unix domain socket) connections.
///
/// The listen socket is created on the first accept, so binding alone leaves
/// nothing on disk.
#[implement(
    bindings::Pbk20191::WinAsync::Comio::UnixConnectionListener,
    bindings::Pbk20191::WinAsync::Comio::ITransport,
    bindings::Pbk20191::WinAsync::Comio::IUnixBind,
    bindings::Pbk20191::WinAsync::Comio::IListenerSecurity,
    windows::Foundation::IClosable
)]
pub struct UnixConnectionListener {
    core: ListenerCore,
    _module: ModuleRef,
}

/// A listener that accepts Hyper-V socket (AF_HYPERV) connections.
///
/// The listen socket is created on the first accept, as for AF_UNIX.
#[implement(
    bindings::Pbk20191::WinAsync::Comio::HyperVConnectionListener,
    bindings::Pbk20191::WinAsync::Comio::ITransport,
    bindings::Pbk20191::WinAsync::Comio::IHyperVBind,
    windows::Foundation::IClosable
)]
pub struct HyperVConnectionListener {
    core: ListenerCore,
    _module: ModuleRef,
}

// Which transport this listener is, answered from the same constant its
// connector answers with -- so the two halves of one transport cannot
// disagree about what it is called. The metadata puts this on `ITransport`
// rather than on anything named for a role, because binding, connecting and
// listening are three things one transport does.
impl ITransport_Impl for NamedPipeConnectionListener_Impl {
    fn Transport(&self) -> Result<HSTRING> {
        Ok(HSTRING::from(super::pipe::TRANSPORT))
    }
}

impl ITransport_Impl for UnixConnectionListener_Impl {
    fn Transport(&self) -> Result<HSTRING> {
        Ok(HSTRING::from(super::unix::TRANSPORT))
    }
}

impl ITransport_Impl for HyperVConnectionListener_Impl {
    fn Transport(&self) -> Result<HSTRING> {
        Ok(HSTRING::from(super::hvsock::TRANSPORT))
    }
}

// Each listener's own bind, as metadata declares it. One interface per
// transport rather than one shared bind: the addressing genuinely differs,
// and this is what lets each listener offer only the bind it has.
impl INamedPipeBind_Impl for NamedPipeConnectionListener_Impl {
    fn BindAsync(&self, name: &HSTRING) -> Result<IAsyncAction> {
        self.bind(name)
    }
}

impl IUnixBind_Impl for UnixConnectionListener_Impl {
    fn BindAsync(&self, path: &HSTRING) -> Result<IAsyncAction> {
        self.bind(path)
    }
}

// Who may connect, on the two local transports. Not on the Hyper-V listener:
// who may reach a VM socket is configured with the VM.
impl IListenerSecurity_Impl for NamedPipeConnectionListener_Impl {
    fn SecurityDescriptor(&self) -> Result<HSTRING> {
        self.core.security_descriptor()
    }

    fn SetSecurityDescriptor(&self, sddl: &HSTRING) -> Result<()> {
        self.core.set_security_descriptor(sddl)
    }
}

// The metadata spells "no limit" as 0; the pipe API spells it 255. Both are
// accepted nowhere else, so the conversion lives here and only here.
impl INamedPipeListenerSettings_Impl for NamedPipeConnectionListener_Impl {
    fn MessageMode(&self) -> Result<bool> {
        Ok(self.core.pipe_options()?.message_mode)
    }

    fn SetMessageMode(&self, value: bool) -> Result<()> {
        self.core.update_pipe_options(|o| Ok(o.message_mode(value)))
    }

    fn AcceptRemoteClients(&self) -> Result<bool> {
        Ok(self.core.pipe_options()?.accept_remote_clients)
    }

    fn SetAcceptRemoteClients(&self, value: bool) -> Result<()> {
        self.core.update_pipe_options(|o| Ok(o.accept_remote_clients(value)))
    }

    fn MaxInstances(&self) -> Result<u32> {
        Ok(match self.core.pipe_options()?.max_instances {
            PIPE_UNLIMITED_INSTANCES => 0,
            n => n,
        })
    }

    fn SetMaxInstances(&self, value: u32) -> Result<()> {
        self.core.update_pipe_options(|o| match value {
            0 => Ok(o.max_instances(PIPE_UNLIMITED_INSTANCES)),
            1..PIPE_UNLIMITED_INSTANCES => Ok(o.max_instances(value)),
            _ => Err(Error::new(
                windows::Win32::Foundation::E_INVALIDARG,
                "MaxInstances is 1 to 254, or 0 for no limit",
            )),
        })
    }

    fn InBufferSize(&self) -> Result<u32> {
        Ok(self.core.pipe_options()?.in_buffer)
    }

    fn SetInBufferSize(&self, value: u32) -> Result<()> {
        self.core
            .update_pipe_options(|o| Ok(NamedPipeOptions { in_buffer: value, ..o }))
    }

    fn OutBufferSize(&self) -> Result<u32> {
        Ok(self.core.pipe_options()?.out_buffer)
    }

    fn SetOutBufferSize(&self, value: u32) -> Result<()> {
        self.core
            .update_pipe_options(|o| Ok(NamedPipeOptions { out_buffer: value, ..o }))
    }
}

impl IListenerSecurity_Impl for UnixConnectionListener_Impl {
    fn SecurityDescriptor(&self) -> Result<HSTRING> {
        self.core.security_descriptor()
    }

    fn SetSecurityDescriptor(&self, sddl: &HSTRING) -> Result<()> {
        self.core.set_security_descriptor(sddl)
    }
}

// The string forms, which are what the metadata declares — see the RDL for
// why this endpoint crosses a projection as text rather than as the pair of
// GUIDs the transport takes. A malformed endpoint is `E_INVALIDARG`: the
// caller passed something that is not an address, which is exactly that.
impl IHyperVBind_Impl for HyperVConnectionListener_Impl {
    fn BindEndpointAsync(&self, endpoint: &HSTRING) -> Result<IAsyncAction> {
        let addr: HvSocketAddr = endpoint.to_string().parse().map_err(invalid_endpoint)?;
        self.bind_endpoint(&addr.vm_id, &addr.service_id)
    }

    fn BindServiceNameAsync(&self, serviceName: &HSTRING) -> Result<IAsyncAction> {
        let service = crate::common::hvaddr::service_id_from_str(&serviceName.to_string())
            .map_err(invalid_endpoint)?;
        self.bind_service_name(&service)
    }
}

/// E_INVALIDARG, carrying what the string should have looked like.
fn invalid_endpoint(e: crate::common::hvaddr::HvAddrParseError) -> Error {
    Error::new(windows::Win32::Foundation::E_INVALIDARG, e.to_string())
}

// The struct declarations stay out of the macro: `#[implement]` is a
// proc-macro and does not survive being handed a `macro_rules` metavariable
// as an interface name. Only the bodies, which are identical, are generated.
listener_impls!(NamedPipeConnectionListener, NamedPipeConnectionListener_Impl);
listener_impls!(UnixConnectionListener, UnixConnectionListener_Impl);
listener_impls!(HyperVConnectionListener, HyperVConnectionListener_Impl);

impl NamedPipeConnectionListener {
    /// Binds to a pipe name such as `\\.\pipe\example`.
    ///
    /// No pipe instance exists until the accept chain starts. The first one
    /// claims the name, and from then on one listening instance is kept until
    /// `Close` (see `common::pipe`), so the name is never free for another
    /// server to take. A client may therefore connect while no handler is
    /// registered; it is delivered once one is.
    ///
    /// Uses the `INamedPipeListenerSettings` and `SecurityDescriptor` set so
    /// far, and fails with `E_INVALIDARG` if they cannot go together --
    /// remote clients without a descriptor.
    pub fn bind(&self, name: &HSTRING) -> Result<IAsyncAction> {
        let mut options = self.core.pipe_options()?;
        options.security = self.core.security();
        options.validate()?;
        let server = PipeServer::new(&name.to_string(), options);
        self.core
            .install_source(Arc::new(AcceptSource::Pipe(Arc::new(server))), name.clone())
    }

    /// Typed Rust equivalent of the vtable's `SecurityDescriptor` setter.
    pub fn set_security_descriptor(&self, sddl: &HSTRING) -> Result<()> {
        self.core.set_security_descriptor(sddl)
    }
}

impl UnixConnectionListener {
    /// Binds to a filesystem path.
    ///
    /// The socket is bound lazily on the first accept, so a stale socket file
    /// surfaces as an error from whichever of this call or the first
    /// `ConnectionReceived` completes the gating condition.
    pub fn bind(&self, path: &HSTRING) -> Result<IAsyncAction> {
        self.core.install_source(
            Arc::new(AcceptSource::Unix(UnixSource {
                path: path.to_string(),
                security: self.core.security(),
                listener: Mutex::new(None),
            })),
            path.clone(),
        )
    }

    /// Typed Rust equivalent of the vtable's `SecurityDescriptor` setter.
    pub fn set_security_descriptor(&self, sddl: &HSTRING) -> Result<()> {
        self.core.set_security_descriptor(sddl)
    }
}

impl HyperVConnectionListener {
    /// Binds to one partition's `service_id` — the AF_HYPERV counterpart of
    /// `StreamSocketListener::BindEndpointAsync`, whose
    /// `(HostName, serviceName)` pair this mirrors: `vm_id` says *which
    /// partition*, `service_id` says *which endpoint inside it*.
    ///
    /// GUIDs rather than a `HostName` because WinRT has no way to spell this
    /// endpoint: `HostNameType` is `DomainName | Ipv4 | Ipv6 | Bluetooth`.
    ///
    /// Use `HV_GUID_WILDCARD` for `vm_id` to accept from any partition — or
    /// just call [`bind_service_name`](Self::bind_service_name).
        pub fn bind_endpoint(
        &self,
        vm_id: &windows_core::GUID,
        service_id: &windows_core::GUID,
    ) -> Result<IAsyncAction> {
        let addr = HvSocketAddr::new(*vm_id, *service_id);
        let label = HSTRING::from(addr.to_string());
        self.core.install_source(
            Arc::new(AcceptSource::HyperV(HyperVSource {
                addr,
                listener: Mutex::new(None),
            })),
            label,
        )
    }

    /// Binds `service_id` on *any* partition — the AF_HYPERV counterpart of
    /// `StreamSocketListener::BindServiceNameAsync`.
    ///
    /// The analogy is exact: where WinRT's version binds a service on every
    /// local adapter, this binds a service GUID with `HV_GUID_WILDCARD` as
    /// the VM id, accepting from any partition.
        pub fn bind_service_name(&self, service_id: &windows_core::GUID) -> Result<IAsyncAction> {
        self.bind_endpoint(&HV_GUID_WILDCARD, service_id)
    }
}

/// Registers one `ConnectionReceived` handler and, if it is the first
/// consumer, starts the accept chain.
///
/// Free rather than a `ListenerCore` method because it works from the shared
/// `ListenerState` alone, which is what the teardown path has in hand.
///
/// If starting the chain fails synchronously — a malformed endpoint bound
/// before any handler existed, or a stale AF_UNIX socket file — that error
/// surfaces here and the registration is rolled back, leaving the listener
/// exactly as it was.
fn add_handler(
    state: &Arc<ListenerState>,
    handler: &ConnectionHandler,
) -> Result<i64> {
    if state.closed.load(Ordering::Acquire) {
        return Err(Error::from_hresult(RO_E_CLOSED));
    }
    let token = state.event.add(handler)?;
    let first = {
        let mut h = state.handlers.lock().unwrap();
        h.insert(token);
        h.len() == 1
    };
    if first
        && let Err(e) = ensure_running(state.clone())
    {
        state.handlers.lock().unwrap().remove(&token);
        state.event.remove(token);
        return Err(e);
    }
    Ok(token)
}

/// Removes a handler. Removing the last one pauses the chain and cancels
/// the in-flight accept.
fn remove_handler(state: &Arc<ListenerState>, token: i64) -> Result<()> {
    if state.closed.load(Ordering::Acquire) {
        return Err(Error::from_hresult(RO_E_CLOSED));
    }
    state.event.remove(token);
    let last = {
        let mut h = state.handlers.lock().unwrap();
        h.remove(&token);
        h.is_empty()
    };
    if last
        && let Some(op) = state.current_op.lock().unwrap().take()
    {
        op.cancel();
    }
    Ok(())
}

fn want_running(state: &ListenerState) -> bool {
    if state.closed.load(Ordering::Acquire) {
        return false;
    }
    if state.source.lock().unwrap().is_none() {
        return false;
    }
    !state.handlers.lock().unwrap().is_empty()
}

/// Releases the `running` flag and reports whether the chain should restart —
/// i.e. a start was explicitly requested (`restart_pending`) *and* the listener
/// is still wanted. Restart happens *only* on an explicit request, never on a
/// bare stop/failure, so a persistent hard accept error can never hot-spin the
/// pool. The `SeqCst` store/swap pairs with the `SeqCst` set/load in [`drive`]'s
/// lost-CAS branch to form a Dekker-style handshake: it is impossible for a
/// requester and a releaser to miss each other (that would require a cycle in
/// the single total order `SeqCst` imposes), so a start request is never lost.
fn release_running(state: &Arc<ListenerState>) -> bool {
    state.running.store(false, Ordering::SeqCst);
    state.restart_pending.swap(false, Ordering::SeqCst) && want_running(state)
}

impl AcceptSink {
    /// One accept's outcome, from the transport's completion: fires the
    /// event and re-arms, or releases the chain (see [`drive`]).
    pub(crate) fn deliver(self, res: Result<Accepted>) {
        let state_cb = self.state;
        if state_cb.closed.load(Ordering::Acquire) {
            // Closed: just release (`want_running` is false, so no restart).
            let _ = release_running(&state_cb);
            return;
        }
        match res {
            Ok(accepted) => {
                // The peer's endpoint when the transport could name one
                // (AF_HYPERV: which partition connected), else this
                // listener's own bound endpoint — which is all the other
                // transports can say.
                let endpoint = accepted.remote.unwrap_or_else(|| {
                    state_cb
                        .endpoint
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or_default()
                });
                fire_event(&state_cb, accepted.stream, endpoint, accepted.peer);
                // Re-arm on a *fresh* pool stack, continuing to own `running`
                // (`drive(.., true)`). Handing the re-arm to the pool bounds
                // chain depth to O(1) even when completions arrive synchronously
                // (the named-pipe `ERROR_PIPE_CONNECTED` "already connected"
                // backlog case); `drive` itself parks the chain if it is no
                // longer wanted.
                let next = state_cb.clone();
                // `comio::pool::submit`, not `windows_threading::submit`: the
                // queued re-arm is code in this module that will run, so it is
                // counted until it has, and the module is pinned while it does.
                super::pool::submit(move || {
                    let _ = drive(next, true);
                });
            }
            // A named pipe was parked at its instance limit and an instance
            // has closed: arm again, still owning `running`, exactly as after
            // a connection -- `drive` parks the chain instead if it is no
            // longer wanted.
            Err(e) if e.code() == super::pipe::SLOT_FREED => {
                let next = state_cb.clone();
                super::pool::submit(move || {
                    let _ = drive(next, true);
                });
            }
            Err(_) => {
                // Hard accept failure, or a cancellation from Close / the last
                // handler being removed. Release the flag; restart only if a
                // start was explicitly requested meanwhile (the add-races-stop
                // handoff) — never on a bare failure, so a persistent error
                // can't hot-spin the pool.
                if release_running(&state_cb) {
                    let _ = drive(state_cb, false);
                }
            }
        }
        }
}

/// The single, non-recursive accept-chain driver.
///
/// `acquired` is `false` for a cold start (`BindAsync` / `ConnectionReceived`):
/// the loop first claims the `running` flag via a CAS. It is `true` for a re-arm
/// continuation already owning the flag (scheduled by `on_result` onto the pool).
/// The `running` CAS is the single serialization point — only the thread that
/// flips `false -> true` issues accepts, so concurrent starters never run two
/// overlapping chains.
///
/// Everything is a loop, never recursion: when the chain must stop but a restart
/// was requested in the meantime (the add-races-stop handoff), the loop simply
/// re-iterates instead of re-entering itself. `try_arm` likewise never calls
/// back into `drive`, and `on_result`'s re-arm is handed to a *fresh* pool stack
/// via `comio::pool::submit` — so chain depth is O(1) regardless of how
/// large a synchronous connect backlog is (a burst of already-connected clients
/// becomes a long *sequence* of pool callbacks, never a deep stack; a Rust stack
/// overflow would be an immediate process `abort`).
///
/// Returns the first accept's synchronous error on a cold start so `BindAsync`
/// can surface it (a malformed pipe name, an AF_UNIX bind/listen error, …); a
/// re-arm continuation has nowhere to surface it and ignores it.
// `acquired = true` below is dead *today* only because every path out of the
// `try_arm` match either returns or resets it first. It stays because the
// variable's meaning is "we own `running`", and a future `continue` added
// after the acquisition would otherwise re-enter the CAS while already
// holding it — a silent double-acquire. Keeping the invariant honest is worth
// more than the warning.
#[allow(unused_assignments)]
fn drive(state: Arc<ListenerState>, mut acquired: bool) -> Result<()> {
    loop {
        if !acquired {
            if !want_running(&state) {
                return Ok(());
            }
            if state
                .running
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                // A chain already owns `running`. Record that a (re)start is
                // wanted so that chain restarts when it stops, then close the
                // lost-request window: if it released `running` between our CAS
                // and now, retry rather than trust it to have seen our request.
                state.restart_pending.store(true, Ordering::SeqCst);
                if !state.running.load(Ordering::SeqCst) {
                    continue;
                }
                return Ok(());
            }
            acquired = true;
        }

        // We own `running`.
        match try_arm(&state) {
            // Armed; `on_result` drives the next step.
            Ok(true) => return Ok(()),
            // No longer wanted (closed / unbound / no handler). Release; if a
            // start was requested meanwhile, reclaim and retry, else stop.
            Ok(false) => {
                acquired = false;
                if release_running(&state) {
                    continue;
                }
                return Ok(());
            }
            // Couldn't issue the accept. Release and surface the error — unless
            // a start was explicitly requested meanwhile, in which case retry
            // once (bounded by the flag, so no hot-spin on a persistent error).
            Err(e) => {
                if release_running(&state) {
                    acquired = false;
                    continue;
                }
                return Err(e);
            }
        }
    }
}

/// Starts the accept chain if it is wanted and not already running; propagates
/// the first accept's synchronous error. Thin cold-start entry to [`drive`].
fn ensure_running(state: Arc<ListenerState>) -> Result<()> {
    drive(state, false)
}

/// Issues exactly one accept and installs its completion (`on_result`), which
/// fires `ConnectionReceived` and schedules the next step. Does *not* touch the
/// `running`/`restart_pending` lifecycle — the caller ([`drive`]) owns that — so
/// it can never recurse into the chain machinery. Returns:
/// * `Ok(true)`  — an accept was issued; its `OpHandle` is stored in `current_op`.
/// * `Ok(false)` — the listener is no longer wanted; nothing was issued.
/// * `Err(e)`    — a synchronous failure to issue the accept (a malformed pipe
///   name, or the AF_UNIX `bind`/`listen` on the first arm).
fn try_arm(state: &Arc<ListenerState>) -> Result<bool> {
    // The previous accept has finished by the time the chain gets here --
    // this runs after its `on_result`, or on a cold start -- but its handle
    // is still stored, and a finished op still holds its registration. Left
    // there, it keeps the instance it connected open after the consumer has
    // released the stream, until the next arm replaces it. Usually that is
    // at once. A named pipe parked at its instance limit arms nothing, and
    // then waits for that very instance to close -- forever, as the limit
    // test found. (A chain that stops because the last handler went in the
    // moment between a delivery and the re-arm would hold it too.)
    state.current_op.lock().unwrap().take();
    if !want_running(state) {
        return Ok(false);
    }

    // Clone the `Arc<AcceptSource>` out and drop the `source` lock before
    // calling `.arm()` on it below — keeps the lock scope minimal and avoids
    // holding it across an FFI-adjacent call. (`tpio::submit` always delivers
    // the completion on a later pool stack, never inline here.)
    let source = state.source.lock().unwrap().clone();
    let Some(source) = source else {
        // `want_running` saw a bound source a moment ago but it's gone now;
        // nothing to arm. (Source is never cleared today — defensive.)
        return Ok(false);
    };


    // Parked at a named pipe's instance limit: nothing is in flight, and
    // `on_result` re-arms once an instance closes.
    let Some(op) = source.arm(AcceptSink { state: state.clone() })? else {
        return Ok(true);
    };

    *state.current_op.lock().unwrap() = Some(op);

    // Double-check: a Close — or the last `RemoveConnectionReceived` — racing
    // this arm (observed as wanted above, then made unwanted before the op was
    // stored) must still cancel the op just stored. The cancelled op's
    // `on_result` (Err) then runs the release/handoff.
    if !want_running(state)
        && let Some(op) = state.current_op.lock().unwrap().take() {
            op.cancel();
        }

    Ok(true)
}

/// Wraps `stream`/`endpoint` into `ConnectionReceivedArgs` and fires
/// `ConnectionReceived` on this thread-pool callback thread with a null
/// sender — the connection is carried by `args`, not the (nonexistent)
/// sender object. Guards against a panicking consumer handler unwinding
/// across this FFI-adjacent boundary.
fn fire_event(
    state: &Arc<ListenerState>,
    stream: ITransportStream,
    endpoint: HSTRING,
    peer: PeerIdentity,
) {
    let args = new_args(stream, endpoint, peer);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        state.event.call(|h| h.Invoke(None, &args));
    }));
}

#[cfg(test)]
#[path = "listener_tests.rs"]
mod tests;

/// Who may connect (`IListenerSecurity`), who did (`RemoteProcessId` /
/// `RemoteUserSid`), and the pipe-name policy, through the ABI a projection
/// uses.
#[cfg(test)]
#[path = "listener_security_tests.rs"]
mod security_tests;

#[cfg(test)]
#[path = "listener_chain_tests.rs"]
mod chain_tests;

/// `INamedPipeListenerSettings`, `IMessageStream` and the connector's
/// timeout, through the ABI a projection uses.
#[cfg(test)]
#[path = "listener_pipe_settings_tests.rs"]
mod pipe_settings_tests;

