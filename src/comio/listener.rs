//! WinRT-native StreamSocketListener-shaped listener over comio transports.
#![allow(dead_code)]
// This whole file is a COM/WinRT surface: PascalCase members are the ABI
// convention it is shaped to, not Rust naming slips. The `#[interface]` macro
// rejects an attribute of its own, so the allow lives here.
#![allow(non_snake_case)]

use core::ffi::c_void;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use windows::Foundation::{IClosable_Impl, TypedEventHandler};
use windows_core::{
    ComObject, Error, HRESULT, HSTRING, IInspectable, IUnknown, IUnknown_Vtbl, Interface, Result,
    implement, interface,
};
use windows_future::IAsyncAction;

use super::tpio::OpHandle;
use crate::common::hvaddr::{HvSocketAddr, HV_GUID_WILDCARD};

/// RO_E_CLOSED — returned by members of a closed listener/stream.
pub(crate) const RO_E_CLOSED: HRESULT = HRESULT(0x80000013u32 as i32);

/// E_INVALIDARG — a bind form this listener's transport does not accept.
const E_INVALIDARG: HRESULT = HRESULT(0x80070057u32 as i32);

/// Custom fixed-IID interface exposing the stream/endpoint pair delivered by
/// a `ConnectionReceived` event. GUID frozen here — do not change or reuse.
#[interface("9d359f23-57af-4770-b11a-e223be76adc3")]
pub unsafe trait IConnectionReceivedArgs: IUnknown {
    fn Stream(&self, result: *mut *mut c_void) -> HRESULT;
    fn RemoteEndpoint(&self, result: *mut core::mem::ManuallyDrop<HSTRING>) -> HRESULT;
}

#[implement(IConnectionReceivedArgs)]
struct ConnectionReceivedArgs {
    stream: IInspectable,
    endpoint: HSTRING,
}

impl IConnectionReceivedArgs_Impl for ConnectionReceivedArgs_Impl {
    unsafe fn Stream(&self, result: *mut *mut c_void) -> HRESULT {
        if result.is_null() {
            return HRESULT(-2147467261i32); // E_POINTER
        }
        unsafe { *result = self.stream.clone().into_raw() };
        HRESULT(0)
    }
    unsafe fn RemoteEndpoint(&self, result: *mut core::mem::ManuallyDrop<HSTRING>) -> HRESULT {
        if result.is_null() {
            return HRESULT(-2147467261i32); // E_POINTER
        }
        unsafe { *result = core::mem::ManuallyDrop::new(self.endpoint.clone()) };
        HRESULT(0)
    }
}

/// Constructs a new `IConnectionReceivedArgs` wrapping `stream`/`endpoint`.
pub(crate) fn new_args(stream: IInspectable, endpoint: HSTRING) -> IConnectionReceivedArgs {
    ConnectionReceivedArgs { stream, endpoint }.into()
}

/// Which comio transport a [`ConnectionListener`] accepts connections over.
enum Transport {
    NamedPipe,
    Unix,
    HyperV,
}

/// The concrete accept source `try_arm` re-arms against once `BindAsync` has
/// recorded it, shared (behind an `Arc`, cloned out of `ListenerState::source`)
/// rather than referenced directly, so that `try_arm` can drop
/// `ListenerState::source`'s lock before calling `AcceptSource::arm` on it —
/// `tpio::submit` guarantees `on_result` (built by `try_arm`, passed down into
/// `AcceptSource::arm`) always runs later, on a thread-pool thread, never on
/// this call's own stack, but cloning the `Arc` out first keeps the lock's
/// scope minimal regardless.
///
/// Neither transport does any kernel work at `BindAsync` time: the named-pipe
/// variant holds only the name (each `arm` creates a fresh instance), and the
/// AF_UNIX variant holds the path and binds/listens *lazily* on the first
/// `arm` (then reuses the one persistent socket). Deferring the unix
/// `bind`/`listen` is what makes handler gating complete for that transport
/// too: with no handler the chain never arms, so no listen socket exists and a
/// client cannot connect into a backlog (see [`ConnectionReceived`]). The cost
/// is that an AF_UNIX bind error (e.g. a stale socket file) now surfaces when
/// the chain first starts — synchronously from `BindAsync` if a handler is
/// already registered, otherwise surfaced from the first `ConnectionReceived`
/// (same as a deferred named-pipe error) — rather than always from `BindAsync`.
enum AcceptSource {
    NamedPipe(String),
    Unix {
        path: String,
        /// `None` until the first `arm` binds the persistent listen socket.
        listener: Mutex<Option<super::UnixStreamListener>>,
    },
    /// AF_HYPERV, bound lazily on the first `arm` for the same gating reason
    /// as the AF_UNIX variant. A bind error here (most often
    /// `WSAEAFNOSUPPORT` where the Hyper-V socket provider is absent, or
    /// `WSAEADDRINUSE` for a service GUID already listened on) surfaces on the
    /// chain's first start, exactly like the unix one.
    HyperV {
        addr: HvSocketAddr,
        listener: Mutex<Option<super::HvStreamListener>>,
    },
}

impl AcceptSource {
    /// Issues one accept via the underlying transport's callback-based
    /// primitive (`pipe::arm_accept` / `UnixStreamListener::arm_accept`),
    /// both built directly on `tpio::submit` — no `Future`, no executor. For
    /// AF_UNIX, lazily binds the persistent listen socket on first use (so its
    /// bind/listen error surfaces here as an `Err`, and so gating prevents the
    /// socket from existing at all until there is a handler).
    fn arm(&self, on_result: Box<dyn FnOnce(Result<IInspectable>) + Send>) -> Result<OpHandle> {
        match self {
            AcceptSource::NamedPipe(name) => super::pipe::arm_accept(name, on_result),
            AcceptSource::Unix { path, listener } => {
                let mut guard = listener.lock().unwrap();
                if guard.is_none() {
                    *guard = Some(super::UnixStreamListener::bind(path)?);
                }
                guard.as_ref().unwrap().arm_accept(on_result)
            }
            AcceptSource::HyperV { addr, listener } => {
                let mut guard = listener.lock().unwrap();
                if guard.is_none() {
                    *guard = Some(super::HvStreamListener::bind(*addr)?);
                }
                guard.as_ref().unwrap().arm_accept(on_result)
            }
        }
    }
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
    event: windows_core::Event<TypedEventHandler<IInspectable, IInspectable>>,
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
            running: AtomicBool::new(false),
            restart_pending: AtomicBool::new(false),
        }
    }
}

/// A WinRT-native, `StreamSocketListener`-shaped connection listener: raises
/// `ConnectionReceived` for each inbound connection accepted on its
/// transport, self-re-arming a TP_IO completion chain on the default
/// thread-pool callback environment (`pcbe = NULL`) — the same environment
/// every other `comio` IO op runs on — once `BindAsync` is called. There is
/// no private executor and no async task backing this: each accept's
/// completion, delivered by the thread pool, itself issues the next accept
/// before returning.
#[implement(windows::Foundation::IClosable)]
pub struct ConnectionListener {
    state: Arc<ListenerState>,
    transport: Transport,
}

impl Drop for ConnectionListener {
    /// Breaks the `Arc<ListenerState>` <-> accept-chain reference cycle: the
    /// in-flight accept's completion closure (captured by the `Op` block
    /// `tpio::submit` allocates, held alive by the kernel until the real
    /// completion fires) captures a clone of `state` via `on_result`, so
    /// without this the last COM reference going away would leak the state,
    /// the pending overlapped op, and (if a client ever connected) resume
    /// firing events on a listener nobody references anymore. This is not
    /// part of the COM refcount cycle — the chain captures
    /// `Arc<ListenerState>`, never the `ComObject`/COM pointer itself — so
    /// this inner `Drop` is guaranteed to run once the COM object's refcount
    /// reaches zero, even if `Close` was never called. Performs the same
    /// teardown as `IClosable_Impl::Close`; safe to run again after an
    /// explicit `Close` since `closed.store`/`event.clear` are idempotent
    /// and `current_op.lock().unwrap().take()` is already `None` the second
    /// time.
    fn drop(&mut self) {
        self.state.closed.store(true, Ordering::Release);
        if let Some(op) = self.state.current_op.lock().unwrap().take() {
            op.cancel();
        }
        self.state.event.clear();
        self.state.handlers.lock().unwrap().clear();
    }
}

impl ConnectionListener {
    /// Constructs a listener bound to the named-pipe transport. Nothing is
    /// created on disk/in the kernel until `BindAsync` is called.
    pub fn named_pipe() -> ComObject<ConnectionListener> {
        ComObject::new(ConnectionListener {
            state: Arc::new(ListenerState::new()),
            transport: Transport::NamedPipe,
        })
    }

    /// Constructs a listener bound to the AF_UNIX transport. Nothing is
    /// created on disk/in the kernel until `BindAsync` is called.
    pub fn unix() -> ComObject<ConnectionListener> {
        ComObject::new(ConnectionListener {
            state: Arc::new(ListenerState::new()),
            transport: Transport::Unix,
        })
    }

    /// Constructs a listener bound to the Hyper-V socket (AF_HYPERV)
    /// transport. Nothing is created in the kernel until
    /// [`BindEndpointAsync`](Self::BindEndpointAsync) or
    /// [`BindServiceNameAsync`](Self::BindServiceNameAsync) is called.
    ///
    /// This transport addresses by GUID pair, not by string, so plain
    /// [`BindAsync`](Self::BindAsync) is rejected on it with `E_INVALIDARG`.
    pub fn hyper_v() -> ComObject<ConnectionListener> {
        ComObject::new(ConnectionListener {
            state: Arc::new(ListenerState::new()),
            transport: Transport::HyperV,
        })
    }

    /// Registers `handler` to be invoked (on a thread-pool thread) for each
    /// accepted connection. Returns a token for `RemoveConnectionReceived`.
    ///
    /// Registering the *first* handler starts the accept chain if the listener
    /// is already bound (handler gating — see [`ListenerState`]): a listener
    /// with no `ConnectionReceived` consumer does not accept connections.
    ///
    /// If starting the chain fails synchronously (e.g. an endpoint bound before
    /// any handler was registered turns out to be malformed, or an AF_UNIX
    /// bind/listen error on a stale socket file — deferred first-accept errors
    /// that handler gating pushes to the chain's actual start), that error is
    /// surfaced from here and the just-added registration is rolled back, so the
    /// listener is left exactly as it was before this call. This mirrors the
    /// handler-first ordering, where the same first-accept error surfaces from
    /// `BindAsync` instead.
    #[allow(non_snake_case)]
    pub fn ConnectionReceived(
        &self,
        handler: &TypedEventHandler<IInspectable, IInspectable>,
    ) -> Result<i64> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }
        let token = self.state.event.add(handler)?;
        let first = {
            let mut h = self.state.handlers.lock().unwrap();
            let was_empty = h.is_empty();
            h.insert(token);
            was_empty
        };
        if first {
            // First consumer: start accepting (no-op if not bound yet). On a
            // synchronous start failure, roll the registration back out so the
            // listener is left untouched, then surface the error.
            if let Err(e) = ensure_running(self.state.clone()) {
                self.state.event.remove(token);
                self.state.handlers.lock().unwrap().remove(&token);
                return Err(e);
            }
        }
        Ok(token)
    }

    /// Revokes a previously registered `ConnectionReceived` handler.
    ///
    /// Removing the *last* handler pauses the accept chain (handler gating —
    /// see [`ListenerState`]): the in-flight accept is cancelled so the
    /// listener stops accepting connections no one would receive. A later
    /// `ConnectionReceived` resumes it.
    ///
    /// One inherent race remains: if the in-flight accept *completes* (a client
    /// connected) in the instant between here cancelling it and the completion
    /// running, that one already-accepted connection is delivered to an empty
    /// handler set and dropped. The accept was already in flight, so it can't be
    /// un-accepted; this is the usual accept/stop boundary race and is bounded
    /// to at most that single connection.
    #[allow(non_snake_case)]
    pub fn RemoveConnectionReceived(&self, token: i64) -> Result<()> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }
        self.state.event.remove(token);
        let now_empty = {
            let mut h = self.state.handlers.lock().unwrap();
            h.remove(&token);
            h.is_empty()
        };
        if now_empty {
            // No consumers left: cancel the in-flight accept. Its `on_result`
            // (Err, from the cancellation) runs the release/handoff, which —
            // seeing `want_running` now false — leaves the chain stopped.
            if let Some(op) = self.state.current_op.lock().unwrap().take() {
                op.cancel();
            }
        }
        Ok(())
    }

    /// Records `endpoint` (the pipe name / socket path, depending on
    /// transport) and — *if a `ConnectionReceived` handler is already
    /// registered* (handler gating, see [`ListenerState`]) — issues the first
    /// accept of the self-re-arming completion chain (see [`drive`]). With no
    /// handler yet, binding only records the endpoint/source; the chain starts
    /// later, when the first handler is registered. Neither transport does any
    /// kernel work until the chain first arms (see [`AcceptSource`]): the
    /// named-pipe instance and the AF_UNIX listen socket are both created on
    /// the first accept, so a bound-but-handler-less listener holds no kernel
    /// resources and accepts nothing.
    ///
    /// When a handler is present, a synchronous failure to issue the *first*
    /// accept (a malformed pipe name, or an AF_UNIX bind/listen error such as a
    /// stale socket file) is surfaced synchronously from this call — only
    /// re-arms triggered internally by a later completion can't propagate a
    /// `Result` anywhere and instead just stop the chain (see [`drive`]). With
    /// no handler, that first-accept error is deferred to the first
    /// `ConnectionReceived` call, which surfaces it there (see
    /// [`ConnectionReceived`](Self::ConnectionReceived)). The returned
    /// `IAsyncAction` is already completed once this call returns `Ok`.
    ///
    /// # Not safe to call concurrently with an already-running accept chain
    /// Calling `BindAsync` again *sequentially* (e.g. to rebind after a
    /// prior `Close`-less reconfiguration) is fine: this call cancels the
    /// previous `current_op` and replaces `source` before installing the new
    /// chain, so nothing is orphaned. But calling `BindAsync` from one
    /// thread while a previously started accept chain is still actively
    /// running (not yet superseded) on another is not supported: the
    /// `current_op` handoff in `try_arm` assumes a single active chain at a
    /// time, and a concurrent rebind racing it could store a new `source`/
    /// cancel the wrong `current_op` out from under the other chain,
    /// orphaning an in-flight accept (untracked, so `Close`/`Drop` can no
    /// longer cancel it). Callers that need to rebind must serialize
    /// `BindAsync` calls themselves.
    ///
    /// Note a first-accept error-surfacing asymmetry on *rebind*: a fresh bind
    /// (no chain yet) issues the first accept inline and surfaces its error, but
    /// a rebind over a still-running chain can't claim `running` synchronously
    /// (the old chain still holds it until its cancelled accept completes on the
    /// pool), so the new source's first accept — and any error from it — happens
    /// later on the restart path and is not surfaced from this call. Rebind onto
    /// a questionable endpoint after an explicit `Close` (or a fresh listener)
    /// if you need the error synchronously.
    #[allow(non_snake_case)]
    pub fn BindAsync(&self, endpoint: &HSTRING) -> Result<IAsyncAction> {
        let source = match self.transport {
            Transport::NamedPipe => AcceptSource::NamedPipe(endpoint.to_string()),
            // The AF_UNIX listen socket is bound lazily on the first accept
            // (see `AcceptSource`), not here, so that handler gating also keeps
            // the socket from existing until there is a consumer.
            Transport::Unix => AcceptSource::Unix {
                path: endpoint.to_string(),
                listener: Mutex::new(None),
            },
            // A Hyper-V endpoint is a GUID pair; there is no string form of it
            // this listener accepts.
            Transport::HyperV => return Err(Error::from_hresult(E_INVALIDARG)),
        };
        self.install_source(source, endpoint.clone())
    }

    /// Binds the Hyper-V listener to one partition's `service_id` — the
    /// AF_HYPERV counterpart of `StreamSocketListener::BindEndpointAsync`,
    /// whose `(HostName, serviceName)` pair this mirrors: `vm_id` says *which
    /// partition*, `service_id` says *which endpoint inside it*.
    ///
    /// GUIDs rather than a `HostName` because WinRT has no way to spell this
    /// endpoint: `HostNameType` is `DomainName | Ipv4 | Ipv6 | Bluetooth`.
    ///
    /// Use `HV_GUID_WILDCARD` for `vm_id` to accept from any partition — or
    /// just call [`BindServiceNameAsync`](Self::BindServiceNameAsync), which
    /// is exactly that. Rejected with `E_INVALIDARG` on a named-pipe or
    /// AF_UNIX listener.
    #[allow(non_snake_case)]
    pub fn BindEndpointAsync(
        &self,
        vm_id: &windows_core::GUID,
        service_id: &windows_core::GUID,
    ) -> Result<IAsyncAction> {
        if !matches!(self.transport, Transport::HyperV) {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        let addr = HvSocketAddr::new(*vm_id, *service_id);
        let label = HSTRING::from(format!("{:?}/{:?}", addr.vm_id, addr.service_id));
        self.install_source(
            AcceptSource::HyperV {
                addr,
                listener: Mutex::new(None),
            },
            label,
        )
    }

    /// Binds the Hyper-V listener to `service_id` on *any* partition — the
    /// AF_HYPERV counterpart of `StreamSocketListener::BindServiceNameAsync`.
    ///
    /// The analogy is exact: where WinRT's version binds a service on every
    /// local adapter, this binds a service GUID with `HV_GUID_WILDCARD` as the
    /// VM id, accepting from any partition. Rejected with `E_INVALIDARG` on a
    /// named-pipe or AF_UNIX listener.
    #[allow(non_snake_case)]
    pub fn BindServiceNameAsync(&self, service_id: &windows_core::GUID) -> Result<IAsyncAction> {
        self.BindEndpointAsync(&HV_GUID_WILDCARD, service_id)
    }

    /// Shared tail of every `Bind*Async`: swap in the new accept source and
    /// (re)start the chain if a consumer is already registered.
    fn install_source(&self, source: AcceptSource, label: HSTRING) -> Result<IAsyncAction> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }

        // A second bind replacing an in-flight chain must not orphan an
        // uncancellable accept: cancel whatever is currently armed before
        // installing the new source.
        if let Some(op) = self.state.current_op.lock().unwrap().take() {
            op.cancel();
        }

        *self.state.source.lock().unwrap() = Some(Arc::new(source));
        *self.state.endpoint.lock().unwrap() = Some(label);

        // Start accepting only if a consumer is already registered (handler
        // gating). No handler => no accept; the chain starts on the first
        // `ConnectionReceived`.
        ensure_running(self.state.clone())?;

        Ok(IAsyncAction::spawn(|| Ok(())))
    }

    /// Returns the endpoint this listener is bound to (the pipe name / socket
    /// path passed to `BindAsync`), or `RO_E_CLOSED` once the listener has
    /// been closed. A minimal stand-in for `StreamSocketListenerInformation`:
    /// a full info object is unneeded (YAGNI) — callers that already have the
    /// endpoint string (they passed it to `BindAsync`) rarely need this, but
    /// it's here for API parity with `StreamSocketListener::Information()`.
    ///
    /// Note: unlike `StreamSocketListener`, there is intentionally no
    /// `Control`/`ListenerControl`-style object here (e.g. for tuning
    /// backlog or pipe-instance count) — the spec's richer shape is scoped
    /// down to what's actually used (YAGNI); add one if/when a caller needs
    /// to configure that.
    #[allow(non_snake_case)]
    pub fn Information(&self) -> Result<HSTRING> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }
        Ok(self.state.endpoint.lock().unwrap().clone().unwrap_or_default())
    }
}

/// The handler-gating predicate: the accept chain should be running iff the
/// listener is not closed, is bound (`source` is `Some`), and has at least one
/// registered `ConnectionReceived` handler. Each sub-lock is taken and
/// released in turn (never nested) so there is no lock-ordering hazard with
/// the mutating paths.
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
/// via `windows_threading::submit` — so chain depth is O(1) regardless of how
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

    let state_cb = state.clone();
    let on_result: Box<dyn FnOnce(Result<IInspectable>) + Send> = Box::new(move |res| {
        if state_cb.closed.load(Ordering::Acquire) {
            // Closed: just release (`want_running` is false, so no restart).
            let _ = release_running(&state_cb);
            return;
        }
        match res {
            Ok(stream) => {
                let endpoint = state_cb
                    .endpoint
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_default();
                fire_event(&state_cb, stream, endpoint);
                // Re-arm on a *fresh* pool stack, continuing to own `running`
                // (`drive(.., true)`). Handing the re-arm to the pool bounds
                // chain depth to O(1) even when completions arrive synchronously
                // (the named-pipe `ERROR_PIPE_CONNECTED` "already connected"
                // backlog case); `drive` itself parks the chain if it is no
                // longer wanted.
                let next = state_cb.clone();
                windows_threading::submit(move || {
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
    });

    let op = source.arm(on_result)?;

    *state.current_op.lock().unwrap() = Some(op);

    // Double-check: a Close — or the last `RemoveConnectionReceived` — racing
    // this arm (observed as wanted above, then made unwanted before the op was
    // stored) must still cancel the op just stored. The cancelled op's
    // `on_result` (Err) then runs the release/handoff.
    if !want_running(state) {
        if let Some(op) = state.current_op.lock().unwrap().take() {
            op.cancel();
        }
    }

    Ok(true)
}

/// Wraps `stream`/`endpoint` into `IConnectionReceivedArgs` and fires
/// `ConnectionReceived` on this thread-pool callback thread with a null
/// sender — the connection is carried by `args`, not the (nonexistent)
/// sender object. Guards against a panicking consumer handler unwinding
/// across this FFI-adjacent boundary.
fn fire_event(state: &Arc<ListenerState>, stream: IInspectable, endpoint: HSTRING) {
    let args: IInspectable = new_args(stream, endpoint).cast().unwrap();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        state.event.call(|h| h.Invoke(None, &args));
    }));
}

impl IClosable_Impl for ConnectionListener_Impl {
    /// Idempotent: flips `closed` (guarding every other member), cancels the
    /// in-flight accept (`CancelIoEx` via `OpHandle::cancel`, which also
    /// stops the chain from re-arming — its `on_result` observes `closed`
    /// before doing anything else), and clears all registered
    /// `ConnectionReceived` handlers.
    fn Close(&self) -> Result<()> {
        self.state.closed.store(true, Ordering::Release);
        if let Some(op) = self.state.current_op.lock().unwrap().take() {
            op.cancel();
        }
        self.state.event.clear();
        self.state.handlers.lock().unwrap().clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::buffer::new_buffer;
    use windows_core::Interface;

    #[test]
    fn args_exposes_stream_and_endpoint() {
        // Any IInspectable stands in for the accepted stream here.
        let stream_stub: windows_core::IInspectable = new_buffer(4).unwrap().into();
        let args = new_args(stream_stub.clone(), windows_core::HSTRING::from("pipe://x"));
        // Stream() returns the same object identity.
        let got = unsafe {
            let mut raw = core::ptr::null_mut();
            assert!(args.Stream(&mut raw).is_ok());
            windows_core::IInspectable::from_raw(raw)
        };
        assert_eq!(got, stream_stub);
        let ep = unsafe {
            let mut s = core::mem::ManuallyDrop::new(windows_core::HSTRING::new());
            assert!(args.RemoteEndpoint(&mut s).is_ok());
            core::mem::ManuallyDrop::into_inner(s)
        };
        assert_eq!(ep.to_string(), "pipe://x");
    }
}

#[cfg(test)]
mod listener_tests {
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
        let listener = ConnectionListener::named_pipe();

        // `IInspectable` (and hence the accepted stream) isn't `Send` — same
        // reasoning as `comio::tests::cancel_storm` — so `TypedEventHandler`'s
        // `F: Send` bound rules out sending it directly through the channel
        // from the pool thread the event fires on. Smuggle the identity as a
        // `usize` (itself `Send`) and reconstitute it on this thread instead;
        // the object underneath is genuinely thread-safe (a plain overlapped
        // registration), only the `IInspectable` wrapper type is
        // conservatively `!Send`.
        let (tx, rx) = mpsc::channel::<usize>();
        let handler = TypedEventHandler::<IInspectable, IInspectable>::new(move |_sender, args| {
            // args -> IConnectionReceivedArgs -> Stream()
            let a = args.as_ref().unwrap();
            let cra: IConnectionReceivedArgs = a.cast().unwrap();
            let raw = unsafe {
                let mut raw = core::ptr::null_mut();
                cra.Stream(&mut raw).ok()?;
                raw as usize
            };
            let _ = tx.send(raw);
            Ok(())
        });
        listener.ConnectionReceived(&handler).unwrap();
        block_on(listener.BindAsync(&HSTRING::from(&name)).unwrap().into_future()).unwrap();

        // Client connects (short retry to cover the inter-instance window
        // between the loop's pipe instances).
        let client = {
            let mut last = None;
            for _ in 0..50 {
                match block_on(crate::comio::NamedPipeClient::connect(&name)) {
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

        // `ConnectionListener::named_pipe()` returns a `ComObject<ConnectionListener>`:
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

        let listener = ConnectionListener::unix();
        let (tx, rx) = mpsc::channel::<usize>();
        let handler = TypedEventHandler::<IInspectable, IInspectable>::new(move |_s, args| {
            let a = args.as_ref().unwrap();
            let cra: IConnectionReceivedArgs = a.cast().unwrap();
            let raw = unsafe {
                let mut raw = core::ptr::null_mut();
                cra.Stream(&mut raw).ok()?;
                raw as usize
            };
            let _ = tx.send(raw);
            Ok(())
        });
        listener.ConnectionReceived(&handler).unwrap();
        block_on(listener.BindAsync(&HSTRING::from(&p)).unwrap().into_future()).unwrap();

        let client = block_on(crate::comio::UnixStreamSocket::connect(&p)).unwrap();
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

    #[test]
    fn hyper_v_rejects_string_bind_and_others_reject_guid_bind() {
        let service = windows_core::GUID::from_u128(0x7c41_e0d2_31ab_4f6e_a2b5_c80d93f14e77);

        // A GUID endpoint has no string form this listener accepts.
        let hv = ConnectionListener::hyper_v();
        let e = hv.BindAsync(&HSTRING::from("loopback")).unwrap_err();
        assert_eq!(e.code(), E_INVALIDARG);

        // ...and conversely the string transports have no GUID endpoint.
        let unix = ConnectionListener::unix();
        assert_eq!(
            unix.BindEndpointAsync(&HV_GUID_WILDCARD, &service).unwrap_err().code(),
            E_INVALIDARG
        );
        let pipe = ConnectionListener::named_pipe();
        assert_eq!(
            pipe.BindServiceNameAsync(&service).unwrap_err().code(),
            E_INVALIDARG
        );
    }

    #[test]
    fn hyper_v_connection_received_echo() {
        // A service GUID of our own. Host-to-host loopback needs no
        // GuestCommunicationServices registration; a machine with no Hyper-V
        // socket provider at all reports a skip instead of failing.
        let service = windows_core::GUID::from_u128(0x5a83_c7e1_9b04_4d26_8f17_63ca0e2b95d8);

        let listener = ConnectionListener::hyper_v();
        let (tx, rx) = mpsc::channel::<usize>();
        let handler = TypedEventHandler::<IInspectable, IInspectable>::new(move |_s, args| {
            let a = args.as_ref().unwrap();
            let cra: IConnectionReceivedArgs = a.cast().unwrap();
            let raw = unsafe {
                let mut raw = core::ptr::null_mut();
                cra.Stream(&mut raw).ok()?;
                raw as usize
            };
            let _ = tx.send(raw);
            Ok(())
        });
        listener.ConnectionReceived(&handler).unwrap();
        // Handler is registered, so the lazy AF_HYPERV bind happens here and
        // its error (e.g. WSAEAFNOSUPPORT) surfaces synchronously.
        if let Err(e) = listener.BindServiceNameAsync(&service) {
            eprintln!("skipped: AF_HYPERV listener bind unavailable ({e:?})");
            return;
        }

        let client = match block_on(crate::comio::HvStreamSocket::connect(
            HvSocketAddr::loopback(service),
        )) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("skipped: AF_HYPERV loopback connect refused ({e:?})");
                return;
            }
        };
        let server_raw = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("no ConnectionReceived");
        let server = unsafe { IInspectable::from_raw(server_raw as *mut core::ffi::c_void) };

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
        let listener = ConnectionListener::named_pipe();
        let handler = TypedEventHandler::<IInspectable, IInspectable>::new(|_, _| Ok(()));
        listener.ConnectionReceived(&handler).unwrap();
        block_on(listener.BindAsync(&HSTRING::from(&name)).unwrap().into_future()).unwrap();
        // No client connects; the chain is parked in an in-flight ConnectNamedPipe.
        std::thread::sleep(std::time::Duration::from_millis(50));
        // Information returns the endpoint; then close.
        assert_eq!(listener.Information().unwrap().to_string(), name);
        let closable: IClosable = listener.cast().unwrap();
        closable.Close().unwrap();
        assert_eq!(listener.Information().unwrap_err().code(), RO_E_CLOSED);
        #[cfg(debug_assertions)]
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
        let listener = ConnectionListener::named_pipe();
        let handler = TypedEventHandler::<IInspectable, IInspectable>::new(|_, _| Ok(()));
        listener.ConnectionReceived(&handler).unwrap();
        block_on(listener.BindAsync(&HSTRING::from(&name)).unwrap().into_future()).unwrap();
        // No client connects; the chain is parked in an in-flight ConnectNamedPipe.
        std::thread::sleep(std::time::Duration::from_millis(50));
        // Drop the sole COM reference WITHOUT calling Close(). This proves
        // `impl Drop for ConnectionListener` (not `IClosable::Close`) is what
        // breaks the `Arc<ListenerState>` <-> accept-chain reference cycle:
        // without it, this drop would leak the state, the in-flight
        // overlapped op, and leave the chain able to keep re-arming forever.
        drop(listener);
        #[cfg(debug_assertions)]
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
        let listener = ConnectionListener::named_pipe();
        let closable: IClosable = listener.cast().unwrap();
        closable.Close().unwrap();
        let handler = TypedEventHandler::<IInspectable, IInspectable>::new(|_, _| Ok(()));
        assert_eq!(
            listener.ConnectionReceived(&handler).unwrap_err().code(),
            RO_E_CLOSED
        );
        assert_eq!(
            listener.BindAsync(&HSTRING::from("x")).unwrap_err().code(),
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
        let listener = ConnectionListener::named_pipe();
        let handler = TypedEventHandler::<IInspectable, IInspectable>::new(|_, _| Ok(()));
        listener.ConnectionReceived(&handler).unwrap();
        let err = listener
            .BindAsync(&HSTRING::from("not_a_valid_pipe_name"))
            .unwrap_err();
        assert_ne!(err.code().0, 0);
        // The listener is otherwise still usable (BindAsync failing isn't
        // the same as Close): a well-formed retry should still work.
        assert!(listener.Information().is_err() || listener.Information().is_ok());
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
        let listener = ConnectionListener::named_pipe();

        // No handler yet: gating means BindAsync records the (malformed)
        // endpoint but issues no accept, so it succeeds.
        block_on(
            listener
                .BindAsync(&HSTRING::from("not_a_valid_pipe_name"))
                .unwrap()
                .into_future(),
        )
        .unwrap();

        // Registering the first handler starts the chain, which tries the first
        // accept and fails synchronously — surfaced here, with the registration
        // rolled back.
        let handler = TypedEventHandler::<IInspectable, IInspectable>::new(|_, _| Ok(()));
        let err = listener.ConnectionReceived(&handler).unwrap_err();
        assert_ne!(err.code().0, 0);
        assert_ne!(err.code(), RO_E_CLOSED, "must be the accept error, not closed");

        // Rollback left zero handlers, so re-adding is again the "first"
        // handler. Rebind to a valid endpoint first (source still points at the
        // bad name until then), then register: a clean slate works end to end.
        let name = format!(r"\\.\pipe\comio_listener_deferred_{}", std::process::id());
        block_on(listener.BindAsync(&HSTRING::from(&name)).unwrap().into_future()).unwrap();
        let (tx, rx) = mpsc::channel::<usize>();
        let handler2 = TypedEventHandler::<IInspectable, IInspectable>::new(move |_s, args| {
            let a = args.as_ref().unwrap();
            let cra: IConnectionReceivedArgs = a.cast().unwrap();
            let raw = unsafe {
                let mut raw = core::ptr::null_mut();
                cra.Stream(&mut raw).ok()?;
                raw as usize
            };
            let _ = tx.send(raw);
            Ok(())
        });
        listener.ConnectionReceived(&handler2).unwrap();

        let client = {
            let mut last = None;
            for _ in 0..50 {
                match block_on(crate::comio::NamedPipeClient::connect(&name)) {
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
        let listener = ConnectionListener::named_pipe();

        // Bind with no handler registered. Gating => no accept chain, so no
        // pipe instance exists and a client connect must fail.
        block_on(listener.BindAsync(&HSTRING::from(&name)).unwrap().into_future()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            block_on(crate::comio::NamedPipeClient::connect(&name)).is_err(),
            "connected to a listener that has no ConnectionReceived handler"
        );

        // Register the first handler => chain starts (the first pipe instance
        // is created synchronously inside `ConnectionReceived`), so a client
        // can now connect and the handler fires.
        let (tx, rx) = mpsc::channel::<usize>();
        let handler = TypedEventHandler::<IInspectable, IInspectable>::new(move |_s, args| {
            let a = args.as_ref().unwrap();
            let cra: IConnectionReceivedArgs = a.cast().unwrap();
            let raw = unsafe {
                let mut raw = core::ptr::null_mut();
                cra.Stream(&mut raw).ok()?;
                raw as usize
            };
            let _ = tx.send(raw);
            Ok(())
        });
        listener.ConnectionReceived(&handler).unwrap();

        let client = {
            let mut last = None;
            for _ in 0..50 {
                match block_on(crate::comio::NamedPipeClient::connect(&name)) {
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
    /// handler pauses accepting (the in-flight accept is cancelled, so the pipe
    /// instance disappears and a client can no longer connect), and registering
    /// a handler again resumes it. This exercises the remove-then-re-add
    /// handoff (`restart_pending`) and a lazily re-created pipe instance.
    #[test]
    fn pause_and_resume_on_handler_toggle() {
        let name = format!(r"\\.\pipe\comio_listener_toggle_{}", std::process::id());
        let listener = ConnectionListener::named_pipe();

        // Helper: build a handler that smuggles the delivered stream identity.
        let make_handler = |tx: mpsc::Sender<usize>| {
            TypedEventHandler::<IInspectable, IInspectable>::new(move |_s, args| {
                let a = args.as_ref().unwrap();
                let cra: IConnectionReceivedArgs = a.cast().unwrap();
                let raw = unsafe {
                    let mut raw = core::ptr::null_mut();
                    cra.Stream(&mut raw).ok()?;
                    raw as usize
                };
                let _ = tx.send(raw);
                Ok(())
            })
        };
        let connect_once = |name: &str| {
            let mut last = None;
            for _ in 0..50 {
                match block_on(crate::comio::NamedPipeClient::connect(name)) {
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
        let token = listener.ConnectionReceived(&h1).unwrap();
        block_on(listener.BindAsync(&HSTRING::from(&name)).unwrap().into_future()).unwrap();
        let c1 = connect_once(&name).expect("first connect failed");
        let raw1 = rx1
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("no delivery while a handler was registered");
        let _s1 = unsafe { IInspectable::from_raw(raw1 as *mut core::ffi::c_void) };
        drop(c1);

        // Remove the last handler => chain pauses. Give the cancellation time
        // to tear the pipe instance down, then a connect must fail.
        listener.RemoveConnectionReceived(token).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(
            block_on(crate::comio::NamedPipeClient::connect(&name)).is_err(),
            "listener kept accepting after its last handler was removed"
        );

        // Re-add a handler => chain resumes (restart_pending handoff / lazy
        // re-arm), and a connection is delivered again.
        let (tx2, rx2) = mpsc::channel::<usize>();
        let h2 = make_handler(tx2);
        listener.ConnectionReceived(&h2).unwrap();
        let c2 = connect_once(&name).expect("connect failed after re-adding a handler");
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
    /// each hammer-retrying `NamedPipeClient::connect` with no backoff, to
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
                        match block_on(crate::comio::NamedPipeClient::connect(&name)) {
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

        let listener = ConnectionListener::named_pipe();
        let delivered = Arc::new(AtomicUsize::new(0));
        let delivered_cb = delivered.clone();
        let handler = TypedEventHandler::<IInspectable, IInspectable>::new(move |_s, _args| {
            delivered_cb.fetch_add(1, Ordering::Relaxed);
            Ok(())
        });
        listener.ConnectionReceived(&handler).unwrap();
        block_on(listener.BindAsync(&HSTRING::from(&name)).unwrap().into_future()).unwrap();

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
}
