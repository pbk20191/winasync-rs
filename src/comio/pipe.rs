//! Named-pipe transport: `NamedPipeListener`/`NamedPipeConnector` produce
//! `TransportStream`s (via `stream::from_reg_with_mode`) over Win32 named
//! pipes, the first end-to-end transport built on the `comio` overlapped-IO
//! engine.
//!
//! Instance creation follows `common::pipe`: whenever we hold no instance the
//! next one claims the name, one listening instance is kept between accepts,
//! remote clients are refused unless asked for, and `NamedPipeOptions` sets
//! the rest. The connector opens at identification level and matches its
//! read mode to the pipe's.

use crate::comio::unload::ModuleRef;
use super::bindings;
use super::bindings::Pbk20191::WinAsync::Comio::{
    INamedPipeConnector_Impl, ITransport_Impl, ITransportStream,
};
use super::stream;
use super::tpio::{self, IoHandle, IoReg, Issue, OpHandle};
use super::waitop::await_io;
pub use crate::common::pipe::NamedPipeOptions;
use crate::common::pipe::{
    self as server, ClientEnd, Instance, PipeServer, ReplenishOnExit, Slot, SlotWaiter,
};
use crate::common::security::PeerIdentity;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Win32::Foundation::{
    E_INVALIDARG, ERROR_IO_PENDING, ERROR_OPERATION_ABORTED, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED,
};
use windows::Win32::System::Pipes::ConnectNamedPipe;
use windows_core::{Error, Result, HRESULT, HSTRING};

/// What an accept chain's `on_result` is handed when it was parked at the
/// instance limit and an instance has since closed: not a failure, a cue to
/// arm again. `ERROR_PIPE_BUSY` because that is the condition it waited out;
/// `ConnectNamedPipe` itself never completes with it.
pub(crate) const SLOT_FREED: HRESULT = HRESULT(0x800700E7u32 as i32);

/// An accept parked at the instance limit, shared with the arm that parked
/// it: whichever runs first -- an instance closing, or the completion of an
/// instance that turned up after all -- takes the sink.
pub(crate) struct ParkedAccept(Arc<Mutex<Option<super::listener::AcceptSink>>>);

impl ParkedAccept {
    /// An instance closed (`freed`), or the server is gone: the chain is
    /// told to arm again, or to stop.
    pub(crate) fn wake(self, freed: bool) {
        if let Some(on_result) = self.0.lock().unwrap().take() {
            let code = if freed {
                SLOT_FREED
            } else {
                ERROR_OPERATION_ABORTED.to_hresult()
            };
            on_result.deliver(Err(Error::from_hresult(code)));
        }
    }
}

/// Registers a created instance, handing its count to the registration so it
/// outlives the handle.
fn register(instance: Instance) -> Result<Arc<IoReg>> {
    IoReg::register(instance)
}

/// Server (listener) side of a named pipe. Each `accept()` waits for a
/// client on a listening instance.
pub struct NamedPipeListener {
    server: PipeServer,
}

impl NamedPipeListener {
    /// A listener for `name` with `options` -- [`NamedPipeOptions::new()`]
    /// for the defaults, including the default security: this process's
    /// account, administrators and SYSTEM. Fails with `E_INVALIDARG` for
    /// options no pipe can have.
    pub fn create(name: &str, options: &NamedPipeOptions) -> Result<Self> {
        options.validate()?;
        Ok(Self {
            server: PipeServer::new(name, options.clone()),
        })
    }

    /// Waits for a client on a listening instance, returning the connected
    /// instance as a duplex WinRT stream. At the instance limit this first
    /// waits for one of this listener's instances to close.
    pub async fn accept(&self) -> Result<ITransportStream> {
        let instance = loop {
            let (freed, slot) = crate::common::oneshot::channel::<bool>();
            match self.server.instance_or_wait(SlotWaiter::Signal(freed))? {
                Slot::Ready(instance) => break instance,
                Slot::Wait => {
                    if slot.await != Ok(true) {
                        return Err(Error::from_hresult(ERROR_OPERATION_ABORTED.to_hresult()));
                    }
                }
                // Nothing to await but time; this future has no timer of its
                // own, so a pool timer completes a oneshot instead.
                Slot::Retry => {
                    let (tick, later) = crate::common::oneshot::channel::<()>();
                    super::pool::submit_after(server::RETRY_AFTER, move || {
                        let _ = tick.send(());
                    });
                    let _ = later.await;
                }
            }
        };
        let handle = instance.handle;
        let reg = register(instance)?;

        let _replenish = ReplenishOnExit(&self.server);
        let result = await_io(&reg, move |ovl| unsafe {
            // On an overlapped handle, `ConnectNamedPipe` returning `Ok(())`
            // or `ERROR_IO_PENDING` both mean a completion packet is (or
            // will be) queued to the thread pool's IOCP — same as
            // `ReadFile`/`WriteFile` elsewhere in `comio`.
            //
            // `ERROR_PIPE_CONNECTED` is the one documented exception: it
            // means a client connected between `CreateNamedPipeW` and this
            // call, so the "connect" is already done — but *no* completion
            // packet is ever queued for it, unlike every other overlapped
            // call on this handle. We still classify it as `Issue::Failed`
            // so `tpio::submit` balances the `StartThreadpoolIo` it already
            // armed (via `CancelThreadpoolIo`) and invokes the completion
            // callback synchronously right here, instead of leaving the
            // thread pool waiting for a callback that will never arrive.
            // `await_io`'s completion mapping then treats
            // `ERROR_PIPE_CONNECTED` as success rather than as an error, so
            // callers of `accept` see a single, ordinary "connected" outcome
            // regardless of which path produced it.
            match ConnectNamedPipe(handle, Some(ovl)) {
                Ok(()) => Issue::SyncSuccess,
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                Err(e) => Issue::Failed(e.code()),
            }
        })
        .await;
        result?;
        Ok(stream::from_reg_with_mode(
            reg,
            stream::Peer::PipeServer,
            self.server.message_mode(),
        ))
    }
}

/// Callback-based counterpart to [`NamedPipeListener::accept`], used by
/// `listener.rs`'s self-re-arming, executor-free accept chain via
/// [`tpio::submit`] directly instead of the `Future`-based `accept()`/
/// `await_io`. Takes a listening instance, registers it, and issues the
/// overlapped `ConnectNamedPipe` — the same flags/classification as
/// `accept()` above, including the `ERROR_PIPE_CONNECTED` special case.
///
/// `on_result` is invoked exactly once: normally on a thread-pool worker
/// thread when the kernel completion arrives, but — per `tpio::submit`'s
/// documented `Issue::Failed` behavior — it may instead run synchronously,
/// on the calling thread, *before this function returns*, whenever
/// `ConnectNamedPipe` reports `ERROR_PIPE_CONNECTED` (a client connected
/// between `CreateNamedPipeW` and this call) or any other synchronous
/// failure. Callers that re-arm from within `on_result` must account for
/// that reentrancy.
///
/// At the instance limit there is nothing to arm yet. Then this returns
/// `Ok(None)` and `on_result` runs later with [`SLOT_FREED`]: when an
/// instance of ours closes, or after `RETRY_AFTER` if none is open (see
/// `common::pipe::Slot::Retry`) -- or with `ERROR_OPERATION_ABORTED` if the
/// server is dropped first. Nothing is in flight meanwhile, so there is
/// nothing to cancel.
///
/// A synchronous failure to create/register the pipe instance itself is
/// returned as `Err` directly, without ever invoking `on_result` — mirroring
/// `accept()`'s own synchronous-failure behavior — so a bad pipe name, or one
/// another server already holds, surfaces to the immediate caller rather
/// than through the callback.
///
/// On a connection the completion reads who connected, then creates the
/// next listening instance before handing the connected one on.
pub(crate) fn arm_accept(
    server: &Arc<PipeServer>,
    on_result: super::listener::AcceptSink,
) -> Result<Option<OpHandle>> {
    // Shared by the slot waiter and the completion; whichever runs takes it.
    let on_result = Arc::new(Mutex::new(Some(on_result)));
    let waiter = SlotWaiter::Accept(ParkedAccept(on_result.clone()));
    let instance = match server.instance_or_wait(waiter)? {
        Slot::Ready(instance) => instance,
        Slot::Wait => return Ok(None),
        Slot::Retry => {
            super::pool::submit_after(server::RETRY_AFTER, move || {
                if let Some(on_result) = on_result.lock().unwrap().take() {
                    on_result.deliver(Err(Error::from_hresult(SLOT_FREED)));
                }
            });
            return Ok(None);
        }
    };
    let handle = instance.handle;
    let reg = register(instance)?;
    let server = server.clone();
    let message_mode = server.message_mode();
    let reg_for_complete = reg.clone();
    let op = unsafe {
        tpio::submit(
            &reg,
            move |ioresult: u32, _bytes: usize| {
                // Whatever happened, this instance is about to leave the
                // listening pool -- handed on as a stream, or closed after a
                // failure or a cancellation -- so the next one is made first,
                // while this one still holds the name.
                server.replenish();
                let res = if ioresult == 0 || ioresult == ERROR_PIPE_CONNECTED.0 {
                    let peer = PeerIdentity::of_pipe_client(reg_for_complete.handle);
                    Ok(super::listener::Accepted::local(
                        stream::from_reg_with_mode(
                            reg_for_complete,
                            stream::Peer::PipeServer,
                            message_mode,
                        ),
                        peer,
                    ))
                } else {
                    Err(Error::from_hresult(HRESULT::from_win32(ioresult)))
                };
                if let Some(on_result) = on_result.lock().unwrap().take() {
                    on_result.deliver(res);
                }
            },
            move |ovl| match ConnectNamedPipe(handle, Some(ovl)) {
                Ok(()) => Issue::SyncSuccess,
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                Err(e) => Issue::Failed(e.code()),
            },
        )
    };
    Ok(Some(op))
}

/// Client side of a named pipe.
#[windows_core::implement(
    bindings::Pbk20191::WinAsync::Comio::NamedPipeConnector,
    bindings::Pbk20191::WinAsync::Comio::INamedPipeConnector
)]
pub struct NamedPipeConnector {
    _module: ModuleRef,
}

/// The one spelling of this transport's name, as the vtable's `Transport`
/// reports it.
pub(crate) const TRANSPORT: &str = "namedpipe";

/// Wraps an opened client end as a stream.
fn client_stream(client: ClientEnd) -> Result<ITransportStream> {
    let reg = IoReg::register(unsafe { IoHandle::from_handle(client.handle) })?;
    Ok(stream::from_reg_with_mode(reg, stream::Peer::PipeClient, client.message_mode))
}

impl NamedPipeConnector {
    /// Opens the client end of an existing named pipe, waiting up to five
    /// seconds in all if every instance is busy.
    pub async fn connect(name: &str) -> Result<ITransportStream> {
        Self::connect_with_timeout(name, server::DEFAULT_CONNECT_TIMEOUT).await
    }

    /// [`connect`](Self::connect), waiting up to `timeout` in all while every
    /// instance is busy; zero does not wait. Running out of time is
    /// `ERROR_PIPE_BUSY`. The wait blocks the polling thread:
    /// `WaitNamedPipeW` has no overlapped form.
    pub async fn connect_with_timeout(name: &str, timeout: Duration) -> Result<ITransportStream> {
        client_stream(server::open_client_waiting(&HSTRING::from(name), timeout)?)
    }

    /// Creates the connector. It holds no state; connecting is what it does.
    #[allow(clippy::new_without_default)]
    pub fn new() -> windows_core::ComObject<NamedPipeConnector> {
        windows_core::ComObject::new(NamedPipeConnector {
            _module: ModuleRef::new(),
        })
    }

    /// Typed Rust mirror of the vtable's `ConnectAsync`.
    pub fn connect_async(
        &self,
        name: &windows_core::HSTRING,
    ) -> windows_future::IAsyncOperation<ITransportStream> {
        self.connect_with_timeout_async(name, server::DEFAULT_CONNECT_TIMEOUT)
    }

    /// Typed Rust mirror of the vtable's `ConnectWithTimeoutAsync`.
    ///
    /// Opening the client end is a plain synchronous `CreateFileW`, so the
    /// usual case needs no thread at all: it is done here and handed back as
    /// an already-finished operation. Only `ERROR_PIPE_BUSY` has anything to
    /// wait for, and that wait goes to the thread pool rather than to the
    /// caller.
    pub fn connect_with_timeout_async(
        &self,
        name: &windows_core::HSTRING,
        timeout: Duration,
    ) -> windows_future::IAsyncOperation<ITransportStream> {
        // Every arm builds a `ConnectOp`, never a `windows_future` object:
        // this is the vtable's `ConnectAsync`, so whatever comes back crosses
        // the ABI, and only our own type is counted by `DllCanUnloadNow`.
        match server::open_client(name) {
            Ok(client) => super::connector::finished(client_stream(client)),
            Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() && !timeout.is_zero() => {
                let name = name.clone();
                super::connector::spawned(move || {
                    client_stream(server::open_client_waiting(&name, timeout)?)
                })
            }
            Err(e) => super::connector::finished(Err(e)),
        }
    }
}

impl ITransport_Impl for NamedPipeConnector_Impl {
    fn Transport(&self) -> Result<HSTRING> {
        Ok(HSTRING::from(TRANSPORT))
    }
}

impl INamedPipeConnector_Impl for NamedPipeConnector_Impl {
    fn ConnectAsync(
        &self,
        name: &HSTRING,
    ) -> Result<windows_future::IAsyncOperation<ITransportStream>> {
        Ok(self.connect_async(name))
    }

    fn ConnectWithTimeoutAsync(
        &self,
        name: &HSTRING,
        timeout: &windows::Foundation::TimeSpan,
    ) -> Result<windows_future::IAsyncOperation<ITransportStream>> {
        // 100 ns ticks. A negative wait is not a wait.
        let ticks = u64::try_from(timeout.Duration)
            .map_err(|_| Error::new(E_INVALIDARG, "a negative timeout"))?;
        Ok(self.connect_with_timeout_async(name, Duration::from_nanos(ticks.saturating_mul(100))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::buffer::{buffer_ptr, new_buffer};
    use crate::common::block_on;
    use std::future::IntoFuture;
    use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows_core::Interface;

    #[test]
    fn named_pipe_echo() {
        let name = format!(r"\\.\pipe\comio_pipe_echo_{}", std::process::id());
        let sname = name.clone();
        let server = std::thread::spawn(move || {
            let listener = NamedPipeListener::create(&sname, &NamedPipeOptions::new()).unwrap();
            let stream = block_on(listener.accept()).unwrap();
            let input: IInputStream = stream.cast().unwrap();
            let output: IOutputStream = stream.cast().unwrap();
            let rbuf = new_buffer(16).unwrap();
            let got: IBuffer = block_on(
                input
                    .ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), 16, InputStreamOptions(0))
                    .unwrap()
                    .into_future(),
            )
            .unwrap();
            let n = got.Length().unwrap();
            block_on(output.WriteAsync(&got).unwrap().into_future()).unwrap();
            n
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let client = block_on(NamedPipeConnector::connect(&name)).unwrap();
        let c_out: IOutputStream = client.cast().unwrap();
        let c_in: IInputStream = client.cast().unwrap();
        let wbuf = new_buffer(5).unwrap();
        unsafe {
            buffer_ptr(&wbuf.clone().into())
                .unwrap()
                .copy_from_nonoverlapping(b"hello".as_ptr(), 5);
        }
        wbuf.SetLength(5).unwrap();
        block_on(c_out.WriteAsync(&wbuf.cast::<IBuffer>().unwrap()).unwrap().into_future()).unwrap();
        let rbuf = new_buffer(5).unwrap();
        let echoed: IBuffer = block_on(
            c_in.ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), 5, InputStreamOptions(0))
                .unwrap()
                .into_future(),
        )
        .unwrap();
        unsafe {
            assert_eq!(
                std::slice::from_raw_parts(buffer_ptr(&echoed).unwrap(), 5),
                b"hello"
            );
        }
        assert_eq!(server.join().unwrap(), 5);
    }

    /// Regression test for the trickiest edge case: if a client has already
    /// connected to a pipe instance before the server issues
    /// `ConnectNamedPipe` on it, `ConnectNamedPipe` reports
    /// `ERROR_PIPE_CONNECTED` synchronously instead of `ERROR_IO_PENDING` —
    /// and, per its documented behavior, queues no completion packet for
    /// it. `await_io` must still resolve successfully (rather than hanging
    /// forever waiting for a completion that will never arrive), while
    /// still balancing the `StartThreadpoolIo` that was armed before the
    /// call.
    #[test]
    fn accept_when_already_connected() {
        let name = format!(
            r"\\.\pipe\comio_pipe_preconnected_{}",
            std::process::id()
        );
        let listener = NamedPipeListener::create(&name, &NamedPipeOptions::new()).unwrap();
        // Force the server's pipe instance to exist first so the client's
        // `CreateFileW` can succeed, then connect the client *before*
        // `accept` is ever called — guaranteeing `ConnectNamedPipe` will
        // observe `ERROR_PIPE_CONNECTED` rather than pending.
        let handle = unsafe {
            CreateNamedPipeW(
                listener.server.name(),
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0),
                PIPE_TYPE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                4096,
                0,
                None,
            )
        };
        assert!(!handle.is_invalid());
        let client = block_on(NamedPipeConnector::connect(&name)).unwrap();
        // Now build a listener bound to the SAME already-connected instance
        // by registering the handle we created above directly, bypassing
        // `CreateNamedPipeW` inside `accept` (which would create a *new*,
        // unconnected instance). This isolates exactly the
        // `ConnectNamedPipe` => `ERROR_PIPE_CONNECTED` path that `accept`'s
        // `await_io` closure must special-case.
        let reg = IoReg::register(unsafe { crate::comio::tpio::IoHandle::from_handle(handle) }).unwrap();
        let server_stream = block_on(await_io(&reg, |ovl| unsafe {
            match ConnectNamedPipe(handle, Some(ovl)) {
                Ok(()) => Issue::SyncSuccess,
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                Err(e) => Issue::Failed(e.code()),
            }
        }))
        .unwrap();
        let _ = server_stream;

        let c_out: IOutputStream = client.cast().unwrap();
        block_on(
            c_out
                .WriteAsync(&new_buffer(0).unwrap().cast::<IBuffer>().unwrap())
                .unwrap()
                .into_future(),
        )
        .unwrap();
    }
}
