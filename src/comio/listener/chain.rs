//! The accept chain: one non-recursive loop that arms accepts while the
//! listener is wanted running, and the handler bookkeeping that decides that.

use super::*;

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
pub(super) fn add_handler(
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
pub(super) fn remove_handler(state: &Arc<ListenerState>, token: i64) -> Result<()> {
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

pub(super) fn want_running(state: &ListenerState) -> bool {
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
pub(super) fn release_running(state: &Arc<ListenerState>) -> bool {
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
                crate::comio::pool::submit(move || {
                    let _ = drive(next, true);
                });
            }
            // A named pipe was parked at its instance limit and an instance
            // has closed: arm again, still owning `running`, exactly as after
            // a connection -- `drive` parks the chain instead if it is no
            // longer wanted.
            Err(e) if e.code() == crate::comio::pipe::SLOT_FREED => {
                let next = state_cb.clone();
                crate::comio::pool::submit(move || {
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
pub(super) fn drive(state: Arc<ListenerState>, mut acquired: bool) -> Result<()> {
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
pub(super) fn ensure_running(state: Arc<ListenerState>) -> Result<()> {
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
pub(super) fn try_arm(state: &Arc<ListenerState>) -> Result<bool> {
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
pub(super) fn fire_event(
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
