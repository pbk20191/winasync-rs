//! Async file/socket IO for `tp`: a thin `Future`-based layer over the shared
//! overlapped core ([`crate::common::overlapped`]). The `OpFuture` cancels on
//! drop and hands the payload back; ops run on `tp`'s private pool.
//!
//! Module-wide unsafe invariants (overlapped engine is in `common::overlapped`):
//! the op block refcount starts at 2 (future + kernel) and frees exactly once;
//! `StartThreadpoolIo` is balanced 1:1; no panic crosses the completion FFI
//! boundary; cancellation is `CancelIoEx` + a ref release.

pub mod async_rw;
pub mod device;
pub mod file;
pub mod hvsock;
pub mod net;
pub mod pipe;
pub mod serial;
pub mod unix;

use crate::common::buffer::{IoBuf, IoBufMut};
use crate::common::overlapped::{self as ov, Completion, IoReg, OpHandle};
use crate::tp::RuntimeInner;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use windows::Win32::Foundation::{ERROR_MORE_DATA, HANDLE, WIN32_ERROR};
use windows::Win32::System::IO::OVERLAPPED;

pub(crate) use crate::common::overlapped::Issue;
// Leak counter reached by tests as `super::super::LIVE_OPS`.
#[allow(unused_imports)]
#[cfg(any(test, feature = "counters"))]
pub(crate) use crate::common::overlapped::LIVE_OPS;

/// A handle registered with the runtime's pool, plus the typed runtime so
/// accept/connect can register freshly-accepted sockets on the same pool via
/// `res.rt`. Derefs to the shared [`IoReg`] (for `handle`/`socket()`/`is_socket()`).
pub(crate) struct IoRes {
    inner: Arc<IoReg>,
    pub(crate) rt: Arc<RuntimeInner>,
}

impl Deref for IoRes {
    type Target = IoReg;
    fn deref(&self) -> &IoReg {
        &self.inner
    }
}

impl IoRes {
    /// Takes ownership of `handle` on success; on error the caller still owns it.
    pub(crate) fn register(
        rt: &Arc<RuntimeInner>,
        handle: HANDLE,
        is_socket: bool,
    ) -> windows_core::Result<Arc<Self>> {
        Self::register_with(rt, handle, is_socket, &mut None)
    }

    /// [`register`](Self::register), also holding `extra` until after the
    /// handle is closed -- a named-pipe instance's count, which must not drop
    /// while the instance still exists. Taken only on success: on failure the
    /// caller still owns both, and must close the handle before dropping it.
    pub(crate) fn register_with(
        rt: &Arc<RuntimeInner>,
        handle: HANDLE,
        is_socket: bool,
        extra: &mut Option<Box<dyn std::any::Any + Send + Sync>>,
    ) -> windows_core::Result<Arc<Self>> {
        // Keepalive = the pool-owning Arc, so the pool outlives in-flight ops and
        // each TP_IO's `CloseThreadpoolIo` (in `IoReg::Drop`) runs before the
        // pool's cleanup-group teardown.
        let keep = Arc::new(Keep {
            _rt: rt.clone(),
            _extra: std::sync::Mutex::new(None),
        });
        let inner = IoReg::register(handle, is_socket, Some(rt.pool.default_env()), Some(keep.clone()))?;
        *keep._extra.lock().unwrap() = extra.take();
        Ok(Arc::new(Self {
            inner,
            rt: rt.clone(),
        }))
    }
}

/// Buffers of an in-flight `WSAIoctl`, kept in the op block until it
/// completes.
struct SocketIoctl<I, O> {
    input: I,
    output: O,
    /// Where `WSAIoctl` writes the output length when it finishes at once.
    returned: u32,
}

/// Issues `WSAIoctl(code)` overlapped on a registered socket; resolves to the
/// bytes written into `output`, with both buffers handed back.
///
/// Measured on 26320 (codes Winsock answers in user mode included --
/// `SIO_GET_EXTENSION_FUNCTION_POINTER`, `FIONBIO` -- and ones the provider
/// rejects): every call either returned `WSA_IO_PENDING` or succeeded at
/// once, and either way a completion packet followed. But a call that
/// succeeds at once reports its output length only through
/// `lpcbBytesReturned`; the packet says 0. So the length is read from there
/// when it is set, and from the packet otherwise.
pub(crate) fn socket_ioctl<I: IoBuf, O: IoBufMut>(
    res: &Arc<IoRes>,
    code: u32,
    input: I,
    output: O,
) -> impl Future<Output = (windows_core::Result<usize>, I, O)> + Send + use<I, O> {
    use windows::Win32::Networking::WinSock::WSAIoctl;
    let sock = res.socket();
    let payload = SocketIoctl {
        input,
        output,
        returned: 0,
    };
    let fut = unsafe {
        submit(res, payload, move |ovl, p| {
            let in_len = IoBuf::len(&p.input);
            let out_len = IoBuf::bytes_total(&p.output);
            let r = WSAIoctl(
                sock,
                code,
                (in_len != 0).then(|| p.input.stable_ptr().cast()),
                in_len as u32,
                (out_len != 0).then(|| p.output.stable_mut_ptr().cast()),
                out_len as u32,
                &mut p.returned,
                Some(ovl),
                None,
            );
            crate::common::winsock::classify_wsa_overlapped(r == 0)
        })
    };
    async move {
        let (result, p) = fut.await;
        let result = result.map(|bytes| if p.returned != 0 { p.returned as usize } else { bytes });
        let n = result.as_ref().ok().copied();
        let output = crate::common::buffer::mark_read(n, p.output);
        (result, p.input, output)
    }
}

/// What a registration keeps alive until after its handle is closed.
struct Keep {
    _rt: Arc<RuntimeInner>,
    // Filled after the registration succeeds, so a failed one leaves `extra`
    // with the caller. A `Mutex` only to make that late write `Sync`.
    _extra: std::sync::Mutex<Option<Box<dyn std::any::Any + Send + Sync>>>,
}

/// Result/waker handoff between the completion callback and the polling future.
struct ResultSlot {
    inner: Mutex<(Option<Completion>, Option<Waker>)>,
}

/// Future of one overlapped operation; resolves to the transfer result and hands
/// the payload (with its buffers) back. Cancels the op on drop if still in flight.
pub(crate) struct OpFuture<P> {
    handle: OpHandle<P>,
    slot: Arc<ResultSlot>,
    done: bool,
}

/// Starts an overlapped op over the shared core, wrapping it in a `Future`.
///
/// # Safety
/// Same contract as [`ov::submit`]: `issue` must issue exactly one overlapped op
/// on `res`'s handle and classify its outcome truthfully.
pub(crate) unsafe fn submit<P>(
    res: &Arc<IoRes>,
    payload: P,
    issue: impl FnOnce(*mut OVERLAPPED, &mut P) -> Issue,
) -> OpFuture<P> {
    let slot = Arc::new(ResultSlot {
        inner: Mutex::new((None, None)),
    });
    let slot2 = slot.clone();
    let on_complete: ov::CompleteFn = Box::new(move |c: Completion| {
        let waker = {
            let mut g = slot2.inner.lock().unwrap();
            g.0 = Some(c);
            g.1.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    });
    let handle = unsafe { ov::submit(&res.inner, payload, on_complete, issue) };
    OpFuture {
        handle,
        slot,
        done: false,
    }
}

impl<P> OpFuture<P> {
    /// The completion as the kernel reported it, with the payload.
    fn poll_completion(&mut self, cx: &mut Context<'_>) -> Poll<(Completion, P)> {
        assert!(!self.done, "OpFuture polled after completion");
        let completion = {
            let mut g = self.slot.inner.lock().unwrap();
            match g.0.take() {
                Some(c) => c,
                None => {
                    g.1 = Some(cx.waker().clone());
                    return Poll::Pending;
                }
            }
        };
        self.done = true;
        // Completed (or sync-failed): the kernel is done with the block, so the
        // payload is exclusively ours again.
        let payload = unsafe { self.handle.take_payload() }.unwrap();
        Poll::Ready((completion, payload))
    }

    /// Reads a message-mode pipe: like awaiting the op, except that
    /// `ERROR_MORE_DATA` is a partial message, not a failure. Resolves to the
    /// bytes read and whether the message has more.
    pub(crate) fn message(self) -> MessageOp<P> {
        MessageOp(self)
    }
}

/// See [`OpFuture::message`].
pub(crate) struct MessageOp<P>(OpFuture<P>);

impl<P> Future for MessageOp<P> {
    type Output = (windows_core::Result<(usize, bool)>, P);

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let (completion, payload) = std::task::ready!(self.0.poll_completion(cx));
        let result = match completion {
            Completion::Done(0, bytes) => Ok((bytes, false)),
            Completion::Done(ioresult, bytes) if ioresult == ERROR_MORE_DATA.0 => Ok((bytes, true)),
            Completion::Done(ioresult, _) => Err(windows_core::Error::from_hresult(
                WIN32_ERROR(ioresult).to_hresult(),
            )),
            Completion::SyncFail(hr) => Err(windows_core::Error::from_hresult(hr)),
        };
        Poll::Ready((result, payload))
    }
}

impl<P> Future for OpFuture<P> {
    type Output = (windows_core::Result<usize>, P);

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let (completion, payload) = std::task::ready!(self.poll_completion(cx));
        let result = match completion {
            Completion::Done(0, bytes) => Ok(bytes),
            Completion::Done(ioresult, _) => Err(windows_core::Error::from_hresult(
                WIN32_ERROR(ioresult).to_hresult(),
            )),
            Completion::SyncFail(hr) => Err(windows_core::Error::from_hresult(hr)),
        };
        Poll::Ready((result, payload))
    }
}

impl<P> Drop for OpFuture<P> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        // Not yet consumed: if no completion has landed, request cancellation.
        // The completion still arrives (ERROR_OPERATION_ABORTED, or a raced
        // success) and releases the kernel ref; the handle's own `Drop` releases
        // our caller ref, freeing the block + payload together.
        let pending = self.slot.inner.lock().unwrap().0.is_none();
        if pending {
            self.handle.cancel();
        }
    }
}
