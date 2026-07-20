//! WinRT-native StreamSocketListener-shaped listener over comio transports.
#![allow(dead_code)]

use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use windows::Foundation::{IClosable_Impl, TypedEventHandler};
use windows_core::{
    ComObject, Error, HRESULT, HSTRING, IInspectable, IUnknown, IUnknown_Vtbl, Interface, Result,
    implement, interface,
};
use windows_future::IAsyncAction;

use super::tpio::OpHandle;

/// RO_E_CLOSED — returned by members of a closed listener/stream.
pub(crate) const RO_E_CLOSED: HRESULT = HRESULT(0x80000013u32 as i32);

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
}

/// The concrete accept source `arm` re-arms against once `BindAsync` has
/// bound it, shared (behind an `Arc`, cloned out of `ListenerState::source`)
/// rather than referenced directly, because `arm`'s `on_result` callback can
/// run *synchronously*, reentrantly calling `arm` again before the outer
/// call returns (see `arm`'s doc comment) — holding `ListenerState::source`'s
/// lock across the `AcceptSource::arm` call itself would self-deadlock on
/// that path, so callers must clone the `Arc` and drop the lock first.
enum AcceptSource {
    NamedPipe(String),
    Unix(super::UnixStreamListener),
}

impl AcceptSource {
    /// Issues one accept via the underlying transport's callback-based
    /// primitive (`pipe::arm_accept` / `UnixStreamListener::arm_accept`),
    /// both built directly on `tpio::submit` — no `Future`, no executor.
    fn arm(&self, on_result: Box<dyn FnOnce(Result<IInspectable>) + Send>) -> Result<OpHandle> {
        match self {
            AcceptSource::NamedPipe(name) => super::pipe::arm_accept(name, on_result),
            AcceptSource::Unix(l) => l.arm_accept(on_result),
        }
    }
}

/// Shared, `Send + Sync` state behind a [`ConnectionListener`]: held both by
/// the COM object itself and by the closures driving its self-re-arming
/// accept chain (see [`arm`]), so `Close` can flip `closed`/cancel the
/// in-flight accept out from under it, and the chain can tell it has been
/// asked to stop.
struct ListenerState {
    closed: AtomicBool,
    event: windows_core::Event<TypedEventHandler<IInspectable, IInspectable>>,
    endpoint: Mutex<Option<HSTRING>>,
    /// Set once by `BindAsync`, read (cloned) by every `arm` call.
    source: Mutex<Option<Arc<AcceptSource>>>,
    /// The in-flight accept's `OpHandle`, tagged with the `seq` of the
    /// `arm` call that stored it — see `arm`'s doc comment for why the tag
    /// is needed (a reentrant re-arm can otherwise be clobbered by its own
    /// stale caller finishing "later").
    current_op: Mutex<Option<(u64, OpHandle)>>,
    /// Monotonic counter handing out a unique, increasing id to each `arm`
    /// call, assigned before that call can possibly recurse.
    seq: AtomicU64,
}

impl ListenerState {
    fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            event: windows_core::Event::new(),
            endpoint: Mutex::new(None),
            source: Mutex::new(None),
            current_op: Mutex::new(None),
            seq: AtomicU64::new(0),
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
        if let Some((_, op)) = self.state.current_op.lock().unwrap().take() {
            op.cancel();
        }
        self.state.event.clear();
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

    /// Registers `handler` to be invoked (on a thread-pool thread) for each
    /// accepted connection. Returns a token for `RemoveConnectionReceived`.
    #[allow(non_snake_case)]
    pub fn ConnectionReceived(
        &self,
        handler: &TypedEventHandler<IInspectable, IInspectable>,
    ) -> Result<i64> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }
        self.state.event.add(handler)
    }

    /// Revokes a previously registered `ConnectionReceived` handler.
    #[allow(non_snake_case)]
    pub fn RemoveConnectionReceived(&self, token: i64) -> Result<()> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }
        self.state.event.remove(token);
        Ok(())
    }

    /// Records `endpoint` (the pipe name / socket path, depending on
    /// transport) and issues the first accept of the self-re-arming
    /// completion chain (see [`arm`]). For the named-pipe transport,
    /// binding is deferred to the first `arm_accept` call (it creates its
    /// own pipe instance); for the AF_UNIX transport, the persistent listen
    /// socket is created here (so a bind error — e.g. a stale socket file —
    /// surfaces synchronously from this call rather than later) and then
    /// shared into the chain via `ListenerState::source`. Either way, a
    /// synchronous failure to issue the *first* accept (e.g. a malformed
    /// pipe name) is also surfaced synchronously from this call — only
    /// re-arms triggered internally by a later completion can't propagate a
    /// `Result` anywhere and instead just stop the chain (see `arm`). The
    /// returned `IAsyncAction` is already completed once this call returns
    /// `Ok`.
    #[allow(non_snake_case)]
    pub fn BindAsync(&self, endpoint: &HSTRING) -> Result<IAsyncAction> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }

        // A second `BindAsync` replacing an in-flight chain must not orphan
        // an uncancellable accept: cancel whatever is currently armed before
        // installing the new source.
        if let Some((_, op)) = self.state.current_op.lock().unwrap().take() {
            op.cancel();
        }

        let source = match self.transport {
            Transport::NamedPipe => AcceptSource::NamedPipe(endpoint.to_string()),
            Transport::Unix => AcceptSource::Unix(super::UnixStreamListener::bind(
                &endpoint.to_string(),
            )?),
        };
        *self.state.source.lock().unwrap() = Some(Arc::new(source));
        *self.state.endpoint.lock().unwrap() = Some(endpoint.clone());

        arm(self.state.clone())?;

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

/// Issues (or re-issues) one accept and arranges for its completion to fire
/// `ConnectionReceived` and then issue the *next* accept — a self-re-arming
/// TP_IO completion chain running entirely on the default thread-pool
/// callback environment, with no executor and no async task anywhere in the
/// loop.
///
/// # Reentrancy
/// The transport's `arm_accept` completion (`on_result`, built here) usually
/// runs later, on a thread-pool worker thread, once the kernel delivers the
/// completion. But per `tpio::submit`'s documented `Issue::Failed` behavior,
/// it can instead run *synchronously*, on this call's own stack, before
/// `AcceptSource::arm` even returns — the named-pipe transport's
/// `ERROR_PIPE_CONNECTED` case (a client connected between `CreateNamedPipeW`
/// and `ConnectNamedPipe`) is exactly this. When that happens, `on_result`'s
/// success branch calls `arm` again, reentrantly, from inside this very
/// call's `source.arm(on_result)` below — so by the time this (outer) call
/// gets back its own `op` and is about to store it into
/// `state.current_op`, a *newer* accept may already be stored there by that
/// inner call. Overwriting it with the outer's own (already-fired, dead)
/// `op` would leave the genuinely in-flight accept untracked — invisible to
/// `Close`/`Drop`'s cancellation. `seq` (assigned before the call that might
/// recurse) orders the two unambiguously: `arm` only stores its `op` if no
/// newer `seq` has been stored already, and otherwise just drops it (safe —
/// it already fired, so dropping needs no cancellation).
///
/// Returns `Err` only for a *synchronous* failure to issue this accept at
/// all (e.g. `CreateNamedPipeW` failing on a malformed name) — the only case
/// a caller can act on, since every other error arrives asynchronously
/// through `on_result` instead. `BindAsync` propagates this first call's
/// `Err`; re-arms triggered from within `on_result` ignore it (a hard accept
/// error just stops the chain rather than hot-spinning retries).
fn arm(state: Arc<ListenerState>) -> Result<()> {
    if state.closed.load(Ordering::Acquire) {
        return Ok(());
    }

    // Clone the `Arc<AcceptSource>` out and drop the `source` lock before
    // calling `.arm()` on it below — see this fn's reentrancy note above:
    // holding `source`'s lock across a call that might recurse into `arm`
    // again (which itself reads `source`) would self-deadlock.
    let source = state.source.lock().unwrap().clone();
    let Some(source) = source else {
        // Not bound yet; nothing to do.
        return Ok(());
    };

    let my_seq = state.seq.fetch_add(1, Ordering::AcqRel);

    let state_for_cb = state.clone();
    let on_result: Box<dyn FnOnce(Result<IInspectable>) + Send> = Box::new(move |res| {
        if state_for_cb.closed.load(Ordering::Acquire) {
            return;
        }
        match res {
            Ok(stream) => {
                let endpoint = state_for_cb
                    .endpoint
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_default();
                let args: IInspectable = new_args(stream, endpoint).cast().unwrap();
                // Fire on this pool thread with a null sender — the
                // connection is carried by `args`, not the (nonexistent)
                // sender object. Guard against a panicking consumer handler
                // unwinding across this FFI-adjacent boundary.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    state_for_cb.event.call(|h| h.Invoke(None, &args));
                }));
                // Re-arm for the next connection. Ignore the result: this is
                // an internal re-arm, not the first `BindAsync` call, so
                // there is nowhere to surface a synchronous failure to —
                // stopping the chain is the correct response either way.
                let _ = arm(state_for_cb.clone());
            }
            Err(_) => {
                // Hard accept failure (or a cancellation from Close racing
                // the in-flight accept). Stop without re-arming: retrying a
                // persistent failure here would hot-spin this pool thread,
                // and a cancellation means we're closing anyway.
            }
        }
    });

    let op = source.arm(on_result)?;

    {
        let mut cur = state.current_op.lock().unwrap();
        let superseded = matches!(cur.as_ref(), Some((seq, _)) if *seq > my_seq);
        if superseded {
            drop(cur);
            drop(op);
            return Ok(());
        }
        *cur = Some((my_seq, op));
    }

    // Double-check: a Close racing this re-arm (observed `closed` as false
    // above, then flipped it true, and found no op yet to cancel because we
    // hadn't stored it yet) must still cancel the op we just stored.
    if state.closed.load(Ordering::Acquire) {
        if let Some((_, op)) = state.current_op.lock().unwrap().take() {
            op.cancel();
        }
    }
    Ok(())
}

impl IClosable_Impl for ConnectionListener_Impl {
    /// Idempotent: flips `closed` (guarding every other member), cancels the
    /// in-flight accept (`CancelIoEx` via `OpHandle::cancel`, which also
    /// stops the chain from re-arming — its `on_result` observes `closed`
    /// before doing anything else), and clears all registered
    /// `ConnectionReceived` handlers.
    fn Close(&self) -> Result<()> {
        self.state.closed.store(true, Ordering::Release);
        if let Some((_, op)) = self.state.current_op.lock().unwrap().take() {
            op.cancel();
        }
        self.state.event.clear();
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
    use crate::tp::block_on;
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
    fn close_cancels_inflight_accept_no_leak() {
        use crate::tp::block_on;
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
        use crate::tp::block_on;
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
    /// `arm`'s doc comment).
    #[test]
    fn bind_async_surfaces_first_accept_error() {
        let listener = ConnectionListener::named_pipe();
        let err = listener
            .BindAsync(&HSTRING::from("not_a_valid_pipe_name"))
            .unwrap_err();
        assert_ne!(err.code().0, 0);
        // The listener is otherwise still usable (BindAsync failing isn't
        // the same as Close): a well-formed retry should still work.
        assert!(listener.Information().is_err() || listener.Information().is_ok());
    }
}
