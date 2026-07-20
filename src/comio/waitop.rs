//! Bridges [`tpio::submit`]'s callback-based completion model to
//! `std::future::Future`, for one-shot overlapped operations whose result is
//! just a byte count (often irrelevant) or an error — e.g. the server side
//! of `ConnectNamedPipe`, and (later) AF_UNIX `AcceptEx`/`ConnectEx`. Kept
//! transport-agnostic: nothing here mentions pipes or sockets.
//!
//! Shape mirrors [`super::asyncop`]'s `IoAsyncOp`: a `Mutex`-guarded slot
//! holds the pending result and the last-registered `Waker`, filled by the
//! `tpio::submit` completion callback, which also wakes the task. The
//! returned [`OpHandle`] is stored so the future's `Drop` can cancel the
//! operation if it is abandoned before completion.

use super::tpio::{self, IoReg, Issue, OpHandle};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use windows::Win32::System::IO::OVERLAPPED;
use windows_core::{Error, Result, HRESULT};

/// `.0`: the result, once the completion callback has run. `.1`: the waker
/// to notify when that happens, if `poll` has been called and found nothing
/// there yet.
type Slot = Mutex<(Option<Result<usize>>, Option<Waker>)>;

/// Future returned by [`await_io`]. Not exposed by name — callers see only
/// `impl Future<Output = Result<usize>>`.
struct AwaitIo {
    slot: Arc<Slot>,
    // `Some` for the object's whole lifetime; `Option` only so `Drop` can
    // move it out if it ever needed to (it doesn't currently, but mirrors
    // `OpState::op`'s shape). Holds the caller-side ref that keeps the `Op`
    // block (and thus the `Arc<IoReg>` it holds) alive until the completion
    // callback's own ref drop tears it down — see `tpio::submit`'s doc
    // comment.
    op: Option<OpHandle>,
}

impl Future for AwaitIo {
    type Output = Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // No `!Unpin` fields (`Arc`/`Option<OpHandle>` are both `Unpin`), so
        // a plain field access through the `Pin` is sound without unsafe
        // projection helpers.
        let mut guard = self.slot.lock().unwrap();
        if let Some(res) = guard.0.take() {
            return Poll::Ready(res);
        }
        guard.1 = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for AwaitIo {
    fn drop(&mut self) {
        // If the future is abandoned (e.g. dropped by a `select!` or a
        // timeout) before the op completed, ask the kernel to cancel it so
        // it doesn't run forever unobserved. `OpHandle::cancel` itself
        // checks whether a completion already fired and is a no-op if so.
        if let Some(op) = &self.op {
            op.cancel();
        }
    }
}

/// Issues one overlapped operation via [`tpio::submit`] and returns a Future
/// resolving to the byte count on success or the classified error.
///
/// `issue` runs synchronously inside `submit` on the calling thread (exactly
/// as `tpio::submit`'s own `issue` parameter does), so — unlike the
/// completion callback registered with `submit` — it need not be
/// `Send`/`'static`.
///
/// Completion mapping: `ioresult == 0` is success (`Ok(bytes)`); every other
/// nonzero code is `Err(Error::from_hresult(HRESULT::from_win32(ioresult)))`.
/// (The `ConnectNamedPipe` "already connected" case surfaces as `ioresult == 0`
/// too — `tpio` classifies it as `Issue::Completed(0, 0)`, not a failure.)
pub(crate) fn await_io(
    reg: &Arc<IoReg>,
    issue: impl FnOnce(*mut OVERLAPPED) -> Issue,
) -> impl Future<Output = Result<usize>> {
    let slot: Arc<Slot> = Arc::new(Mutex::new((None, None)));
    let cb_slot = slot.clone();
    let handle = unsafe {
        tpio::submit(
            reg,
            Box::new(move |ioresult: u32, bytes: usize| {
                let res = if ioresult == 0 {
                    Ok(bytes)
                } else {
                    Err(Error::from_hresult(HRESULT::from_win32(ioresult)))
                };
                let waker = {
                    let mut guard = cb_slot.lock().unwrap();
                    guard.0 = Some(res);
                    guard.1.take()
                };
                if let Some(w) = waker {
                    w.wake();
                }
            }),
            issue,
        )
    };
    AwaitIo {
        slot,
        op: Some(handle),
    }
}

/// As [`await_io`], but additionally keeps `payload` alive until the
/// operation's real completion is delivered by the kernel — not merely
/// until the returned future is dropped.
///
/// Use this whenever the overlapped call touches memory owned by `payload`:
/// an output buffer `AcceptEx` writes accepted addresses into, an input
/// `SOCKADDR` `ConnectEx` reads from, etc. `await_io`'s `Drop` only issues
/// `CancelIoEx` and returns immediately — it does not wait for the
/// `ERROR_OPERATION_ABORTED` completion — so if such memory were an ordinary
/// local in the caller's async fn frame, dropping the future in flight (a
/// timeout, a `select!`, ...) would free that memory while the kernel might
/// still be about to write into (or read from) it: a write-after-free.
///
/// This is safe because of `tpio::submit`'s completion discipline (see its
/// doc comment): the `Op` block — and therefore the `on_complete` closure
/// moved into it here — is kept alive by a kernel-side refcount that is
/// released only when `io_complete` actually fires, including for an
/// aborted completion. Moving `payload` into `on_complete` therefore ties
/// its lifetime to the real completion rather than to the future's `Drop`.
///
/// # Contract
/// `issue` must not capture `payload` itself — by the time `issue` runs,
/// `payload` has already been moved into `on_complete`. Instead `issue` must
/// capture only raw pointers/values derived from `payload` beforehand that
/// stay valid regardless of where the `payload` value itself is
/// subsequently moved to — e.g. a pointer read out of a `Box` field: the
/// `Box`'s heap allocation doesn't move even though the `Box` value (and
/// thus `payload`) does. Raw pointers aren't `Send`, so callers typically
/// smuggle them through as a `usize` and cast back inside `issue`.
pub(crate) fn await_io_owning<P: Send + 'static>(
    reg: &Arc<IoReg>,
    payload: P,
    issue: impl FnOnce(*mut OVERLAPPED) -> Issue + Send + 'static,
) -> impl Future<Output = Result<usize>> {
    let slot: Arc<Slot> = Arc::new(Mutex::new((None, None)));
    let cb_slot = slot.clone();
    let handle = unsafe {
        tpio::submit(
            reg,
            Box::new(move |ioresult: u32, bytes: usize| {
                // Dropped here: at the real completion (whether the op ran
                // to natural completion or was aborted by `OpHandle::cancel`
                // and completed with `ERROR_OPERATION_ABORTED`), never
                // earlier — see the contract above.
                drop(payload);
                let res = if ioresult == 0 {
                    Ok(bytes)
                } else {
                    Err(Error::from_hresult(HRESULT::from_win32(ioresult)))
                };
                let waker = {
                    let mut guard = cb_slot.lock().unwrap();
                    guard.0 = Some(res);
                    guard.1.take()
                };
                if let Some(w) = waker {
                    w.wake();
                }
            }),
            issue,
        )
    };
    AwaitIo {
        slot,
        op: Some(handle),
    }
}
