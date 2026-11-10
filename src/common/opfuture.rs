//! A `Future` over one operation of the overlapped core: resolves to the
//! result and hands the payload back, and cancels the operation if dropped
//! while it is in flight. Shared by `tp` and `http`, whichever pool or port
//! the handle is registered with.

use crate::common::overlapped::{self as ov, Completion, IoReg, Issue, OpHandle};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use windows::Win32::Foundation::{ERROR_MORE_DATA, WIN32_ERROR};
use windows::Win32::System::IO::OVERLAPPED;

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
/// on `reg`'s handle and classify its outcome truthfully.
pub(crate) unsafe fn submit<P>(
    reg: &Arc<IoReg>,
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
    let handle = unsafe { ov::submit(reg, payload, on_complete, issue) };
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

