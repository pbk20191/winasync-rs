//! What a listener is made of: the shared state its chain and its COM
//! object both hold, and the core the classes wrap.

use super::*;

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
pub(super) struct ListenerState {
    pub(super) closed: AtomicBool,
    pub(super) event: windows_core::Event<ConnectionHandler>,
    pub(super) endpoint: Mutex<Option<HSTRING>>,
    /// Set once by `BindAsync`, read (cloned) by every `try_arm` call.
    pub(super) source: Mutex<Option<Arc<AcceptSource>>>,
    /// The in-flight accept's `OpHandle`. `tpio::submit` guarantees every
    /// completion — synchronous or asynchronous — is delivered on a
    /// thread-pool thread (see `tpio::submit`'s doc comment), so `try_arm`
    /// never recurses/re-enters on the caller's own stack and a plain
    /// `Option<OpHandle>` (no sequence tag) is sufficient here.
    pub(super) current_op: Mutex<Option<OpHandle>>,
    /// Live `ConnectionReceived` tokens — `windows_core::Event` exposes no
    /// count, so handler presence (the gating predicate) is tracked here.
    pub(super) handlers: Mutex<HashSet<i64>>,
    /// Who may connect, set through `IListenerSecurity` before the bind that
    /// uses it. `None` is the transport's default.
    pub(super) security: Mutex<Option<SecurityDescriptor>>,
    /// How a named pipe's instances are made, set through
    /// `INamedPipeListenerSettings` before the bind. Its `security` is unused:
    /// that comes from the field above when the bind combines the two.
    pub(super) pipe_options: Mutex<NamedPipeOptions>,
    /// Whether an accept chain is logically active. See the type-level
    /// "Handler gating" note.
    pub(super) running: AtomicBool,
    /// A start was requested while `running` was still held by a stopping
    /// chain; the chain restarts once on release. See the "Handler gating" note.
    pub(super) restart_pending: AtomicBool,
}

impl ListenerState {
    pub(super) fn new() -> Self {
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
    pub(super) state: Arc<ListenerState>,
}

impl ListenerCore {
    pub(super) fn new() -> Self {
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
    pub(super) fn connection_received(
        &self,
        handler: &ConnectionHandler,
    ) -> Result<i64> {
        add_handler(&self.state, handler)
    }

    /// Removes a handler. Removing the last one pauses the chain and cancels
    /// the in-flight accept.
    pub(super) fn remove_connection_received(&self, token: i64) -> Result<()> {
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
    pub(super) fn information(&self) -> Result<HSTRING> {
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

    pub(super) fn close(&self) -> Result<()> {
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
    pub(super) fn security_descriptor(&self) -> Result<HSTRING> {
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
    pub(super) fn set_security_descriptor(&self, sddl: &HSTRING) -> Result<()> {
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
    pub(super) fn security(&self) -> Option<SecurityDescriptor> {
        self.state.security.lock().unwrap().clone()
    }

    /// The named-pipe settings, as `INamedPipeListenerSettings` reads them.
    pub(super) fn pipe_options(&self) -> Result<NamedPipeOptions> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }
        Ok(self.state.pipe_options.lock().unwrap().clone())
    }

    /// Changes one named-pipe setting. Only before the first bind, for the
    /// reason [`set_security_descriptor`](Self::set_security_descriptor)
    /// gives.
    pub(super) fn update_pipe_options(
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
    pub(super) fn install_source(&self, source: Arc<AcceptSource>, label: HSTRING) -> Result<IAsyncAction> {
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
        Ok(crate::comio::settled::action_done())
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
