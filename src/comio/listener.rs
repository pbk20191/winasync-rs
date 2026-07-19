//! WinRT-native StreamSocketListener-shaped listener over comio transports.
#![allow(dead_code)]

use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use windows::Foundation::{IClosable_Impl, TypedEventHandler};
use windows_core::{
    ComObject, Error, HRESULT, HSTRING, IInspectable, IUnknown, IUnknown_Vtbl, Interface, Result,
    implement, interface,
};
use windows_future::IAsyncAction;

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

/// Shared, `Send + Sync` state behind a [`ConnectionListener`]: held both by
/// the COM object itself and by its self-driving accept loop (spawned on
/// [`super::shared_runtime`]), so `Close` can flip `closed`/abort the loop
/// out from under a still-running accept, and the loop can tell it has been
/// asked to stop.
struct ListenerState {
    closed: AtomicBool,
    event: windows_core::Event<TypedEventHandler<IInspectable, IInspectable>>,
    endpoint: Mutex<Option<HSTRING>>,
    task: Mutex<Option<crate::tp::JoinHandle<()>>>,
}

impl ListenerState {
    fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            event: windows_core::Event::new(),
            endpoint: Mutex::new(None),
            task: Mutex::new(None),
        }
    }
}

/// A WinRT-native, `StreamSocketListener`-shaped connection listener: raises
/// `ConnectionReceived` for each inbound connection accepted on its
/// transport, self-driving the accept loop on the crate-shared
/// [`super::shared_runtime`] thread pool once `BindAsync` is called.
#[implement(windows::Foundation::IClosable)]
pub struct ConnectionListener {
    state: Arc<ListenerState>,
    transport: Transport,
}

impl Drop for ConnectionListener {
    /// Breaks the `Arc<ListenerState>` <-> accept-loop reference cycle: the
    /// accept loop future (driven by `state.task`'s `JoinHandle`) captures a
    /// clone of `state`, so without this the last COM reference going away
    /// would leak the state, the pending overlapped op, and run the accept
    /// loop forever. This is not part of the COM refcount cycle — the loop
    /// captures `Arc<ListenerState>`, never the `ComObject`/COM pointer
    /// itself — so this inner `Drop` is guaranteed to run once the COM
    /// object's refcount reaches zero, even if `Close` was never called.
    /// Performs the same teardown as `IClosable_Impl::Close`; safe to run
    /// again after an explicit `Close` since both `closed.store` and
    /// `event.clear` are idempotent and `task.lock().unwrap().take()` is
    /// already `None` the second time.
    fn drop(&mut self) {
        self.state.closed.store(true, Ordering::Release);
        if let Some(h) = self.state.task.lock().unwrap().take() {
            h.abort();
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
    /// transport) and starts the self-driving accept loop on
    /// `shared_runtime()`. For the named-pipe transport, binding is
    /// synchronous and the loop itself creates pipe instances and waits for
    /// clients. For the AF_UNIX transport, the persistent listen socket is
    /// created here (so a bind error — e.g. a stale socket file — surfaces
    /// synchronously from this call rather than from the completed action)
    /// and then moved into the loop, which repeatedly `accept()`s on that
    /// same socket. Either way the returned `IAsyncAction` is already
    /// completed once this call returns `Ok`.
    #[allow(non_snake_case)]
    pub fn BindAsync(&self, endpoint: &HSTRING) -> Result<IAsyncAction> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }

        match self.transport {
            Transport::NamedPipe => {
                *self.state.endpoint.lock().unwrap() = Some(endpoint.clone());
                let name = endpoint.to_string();
                let state = self.state.clone();
                let handle = super::shared_runtime().spawn(run_named_pipe_accept_loop(state, name));
                // Abort any existing accept loop before overwriting its
                // handle, so a second `BindAsync` can't orphan an
                // uncancellable loop (its `JoinHandle` would otherwise be
                // dropped here, detaching rather than stopping it).
                if let Some(old) = self.state.task.lock().unwrap().replace(handle) {
                    old.abort();
                }
            }
            Transport::Unix => {
                let path = endpoint.to_string();
                let l = super::UnixStreamListener::bind(&path)?;
                *self.state.endpoint.lock().unwrap() = Some(endpoint.clone());
                let state = self.state.clone();
                let handle = super::shared_runtime().spawn(run_unix_accept_loop(state, path, l));
                if let Some(old) = self.state.task.lock().unwrap().replace(handle) {
                    old.abort();
                }
            }
        }

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

/// `NamedPipeListener::accept()`'s returned future is not `Send`: its own
/// generator holds a raw Win32 `HANDLE` (from `CreateNamedPipeW`) across an
/// internal `.await`, and windows-rs's `HANDLE` isn't `Send`/`Sync` by
/// default. Moving a named-pipe handle between threads is safe in practice —
/// `comio`'s own `IoReg`/`OpHandle` (in `tpio.rs`) make the identical
/// unsafe-Send call around the same kind of handle — so this wrapper asserts
/// it locally for the one nested future that needs to cross our own accept
/// loop's `Send` boundary (required by `tp::Runtime::spawn`), without having
/// to modify `pipe.rs` itself.
struct AssertSend<F>(F);
// # Safety
//
// This blanket `unsafe impl` is sound only because every future this wrapper
// is actually applied to — `NamedPipeListener::accept()` and
// `UnixStreamListener::accept()`'s returned futures, per the two call sites
// in this file — upholds the following invariant: across every `.await`
// point, the future's state holds nothing thread-affine. Concretely, what it
// may hold across an await is limited to:
//   - raw kernel handles/identifiers (Win32 `HANDLE`, `SOCKET`) and comio's
//     own overlapped-op bookkeeping around them (`tpio::IoReg`/`OpHandle`),
//     which are safe to move/poll from any thread — the kernel object itself
//     has no thread affinity, only windows-rs's `HANDLE` wrapper type is
//     conservatively `!Send` by default;
//   - other wrappers that are already `Send` in their own right;
//   - agile COM objects (e.g. the accepted stream's `IInspectable`)
//     constructed *after* the last await, never carried across one.
// It must NOT be used to wrap a future that parks an STA-affine COM pointer,
// a thread-local, or any other genuinely thread-bound value across an await.
// Callers introducing a new use of `AssertSend` must re-verify this
// invariant for the specific future being wrapped — it is not enforced by
// the type system.
unsafe impl<F> Send for AssertSend<F> {}
impl<F: Future> Future for AssertSend<F> {
    type Output = F::Output;
    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        unsafe { self.map_unchecked_mut(|s| &mut s.0) }.poll(cx)
    }
}

/// The named-pipe accept loop: creates a fresh pipe instance per iteration
/// (a listener instance is single-use once connected), waits for a client,
/// and raises `ConnectionReceived` with the accepted stream. Runs until
/// `Close` flips `state.closed` (observed either before starting the next
/// accept or right after one completes) or `abort()`s the task outright.
async fn run_named_pipe_accept_loop(state: Arc<ListenerState>, name: String) {
    loop {
        if state.closed.load(Ordering::Acquire) {
            break;
        }
        let listener = super::pipe::NamedPipeListener::create(&name);
        // The accepted `Result` (and its match) live inside this nested
        // block so its storage is lexically dead before the backoff
        // `.await` below: otherwise this generator would capture the
        // non-`Send` `Result<IInspectable, Error>` across that await too,
        // since the compiler's live-locals analysis is scope-based, not
        // move-based, for a plain `let` binding used earlier in the same
        // block.
        let failed = {
            match AssertSend(listener.accept()).await {
                Ok(stream) => {
                    if state.closed.load(Ordering::Acquire) {
                        break;
                    }
                    let args: IInspectable = new_args(stream, HSTRING::from(&name)).cast().unwrap();
                    // Fire on this pool thread with a null sender — the
                    // connection is carried by `args`, not the (nonexistent)
                    // sender object. Guard against a panicking consumer
                    // handler unwinding across this FFI-adjacent boundary.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        state.event.call(|h| h.Invoke(None, &args));
                    }));
                    false
                }
                Err(_) => true,
            }
        };
        if failed {
            // Transient accept failure (or a cancellation from Close racing
            // the in-flight accept); keep looping unless we've been asked to
            // stop. A short async backoff prevents a persistent failure
            // (e.g. a malformed pipe name) from hot-spinning this pool
            // thread at 100% CPU forever.
            if state.closed.load(Ordering::Acquire) {
                break;
            }
            crate::tp::sleep(crate::comio::shared_runtime(), std::time::Duration::from_millis(100)).await;
        }
    }
}

/// The AF_UNIX accept loop: owns the single persistent listen socket `l`
/// (created once by `BindAsync`) and repeatedly `accept()`s on it, raising
/// `ConnectionReceived` with each accepted stream. Runs until `Close` flips
/// `state.closed` (observed either before starting the next accept or right
/// after one completes) or `abort()`s the task outright. `l.accept()`'s
/// returned future is `!Send` for the same reason the named-pipe accept
/// future is (raw kernel `SOCKET`/overlapped state held across an internal
/// `.await`), so it's wrapped in the same `AssertSend` — sound for the same
/// reason: only kernel socket/overlapped state and agile COM objects are
/// held past the await, no thread-affine state.
async fn run_unix_accept_loop(state: Arc<ListenerState>, path: String, l: super::UnixStreamListener) {
    loop {
        if state.closed.load(Ordering::Acquire) {
            break;
        }
        // See the comment in `run_named_pipe_accept_loop` on why the match on
        // the awaited `Result` is scoped to its own nested block.
        let failed = {
            match AssertSend(l.accept()).await {
                Ok(stream) => {
                    if state.closed.load(Ordering::Acquire) {
                        break;
                    }
                    let args: IInspectable = new_args(stream, HSTRING::from(&path)).cast().unwrap();
                    // Fire on this pool thread with a null sender — the
                    // connection is carried by `args`, not the (nonexistent)
                    // sender object. Guard against a panicking consumer
                    // handler unwinding across this FFI-adjacent boundary.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        state.event.call(|h| h.Invoke(None, &args));
                    }));
                    false
                }
                Err(_) => true,
            }
        };
        if failed {
            // Transient accept failure (or a cancellation from Close racing
            // the in-flight accept); keep looping unless we've been asked to
            // stop. Same backoff as the named-pipe loop, for symmetry — a
            // persistent failure here would be similarly unexpected, but
            // hot-spinning is still undesirable if one ever occurs.
            if state.closed.load(Ordering::Acquire) {
                break;
            }
            crate::tp::sleep(crate::comio::shared_runtime(), std::time::Duration::from_millis(100)).await;
        }
    }
}

impl IClosable_Impl for ConnectionListener_Impl {
    /// Idempotent: flips `closed` (guarding every other member), aborts the
    /// accept loop (dropping its in-flight `accept()` and thus cancelling
    /// the pending overlapped `ConnectNamedPipe`), and clears all registered
    /// `ConnectionReceived` handlers.
    fn Close(&self) -> Result<()> {
        self.state.closed.store(true, Ordering::Release);
        if let Some(h) = self.state.task.lock().unwrap().take() {
            h.abort();
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
        // No client connects; the loop is parked in an in-flight ConnectNamedPipe.
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
        // No client connects; the loop is parked in an in-flight ConnectNamedPipe.
        std::thread::sleep(std::time::Duration::from_millis(50));
        // Drop the sole COM reference WITHOUT calling Close(). This proves
        // `impl Drop for ConnectionListener` (not `IClosable::Close`) is what
        // breaks the `Arc<ListenerState>` <-> accept-loop reference cycle:
        // without it, this drop would leak the state, the in-flight
        // overlapped op, and leave the accept loop spinning forever.
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
}
