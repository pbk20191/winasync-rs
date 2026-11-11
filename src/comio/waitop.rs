//! Bridges `tpio::submit`'s callback-based completion model to
//! `std::future::Future`, for one-shot overlapped operations whose result is
//! just a byte count (often irrelevant) or an error — e.g. the server side
//! of `ConnectNamedPipe`, and (later) AF_UNIX `AcceptEx`/`ConnectEx`. Kept
//! transport-agnostic: nothing here mentions pipes or sockets.
//!
//! The op block carries the outcome and the waker (see
//! `OpHandle::poll_completion`); the returned `OpHandle` is stored so the
//! future's `Drop` can cancel the operation if it is abandoned before
//! completion.

use super::tpio::{self, IoReg, Issue, OpHandle};
use crate::common::overlapped::Completion;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use windows::Win32::Foundation::ERROR_PIPE_CONNECTED;
use windows::Win32::System::IO::OVERLAPPED;
use windows_core::{Error, Result, HRESULT};

/// Future returned by [`await_io`]. Not exposed by name — callers see only
/// `impl Future<Output = Result<usize>>`. Holds the caller-side ref that
/// keeps the `Op` block (and thus the `Arc<IoReg>` it holds) alive until
/// the completion callback's own ref drop tears it down; the block carries
/// the outcome and the waker, so nothing else is allocated.
struct AwaitIo {
    op: OpHandle,
}

impl Future for AwaitIo {
    type Output = Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let c = std::task::ready!(self.op.poll_completion(cx));
        // A synchronous failure is a win32 code with no bytes, as
        // `tpio::submit`'s callbacks have always seen it -- and
        // `ERROR_PIPE_CONNECTED` arrives that way.
        let (ioresult, bytes) = match c {
            Completion::Done(ioresult, bytes) => (ioresult, bytes),
            Completion::SyncFail(hr) => (sync_fail_win32(hr), 0),
        };
        Poll::Ready(if ioresult == 0 || ioresult == ERROR_PIPE_CONNECTED.0 {
            Ok(bytes)
        } else {
            Err(Error::from_hresult(HRESULT::from_win32(ioresult)))
        })
    }
}

impl Drop for AwaitIo {
    fn drop(&mut self) {
        // If the future is abandoned (e.g. dropped by a `select!` or a
        // timeout) before the op completed, ask the kernel to cancel it so
        // it doesn't run forever unobserved. `OpHandle::cancel` itself
        // checks whether a completion already fired and is a no-op if so.
        self.op.cancel();
    }
}

/// The `(win32, 0)` a synchronous failure has always been delivered as by
/// `tpio::submit`, as an HRESULT again: nonzero is what is load-bearing.
fn sync_fail_win32(hr: HRESULT) -> u32 {
    let hr_val = hr.0 as u32;
    if (hr_val >> 16) & 0x7ff == 7 { hr_val & 0xffff } else { 31 }
}

/// Issues one overlapped operation via `tpio::submit` and returns a Future
/// resolving to the byte count on success or the classified error.
///
/// `issue` runs synchronously inside `submit` on the calling thread (exactly
/// as `tpio::submit`'s own `issue` parameter does), so — unlike the
/// completion callback registered with `submit` — it need not be
/// `Send`/`'static`.
///
/// Completion mapping: `ioresult == 0` is success (`Ok(bytes)`).
/// `ERROR_PIPE_CONNECTED` is also mapped to success: it is the documented
/// case where `ConnectNamedPipe` reports (via the synchronous-failure path)
/// that a client already connected between pipe creation and the connect
/// call — semantically a successful connect, not an error, even though no
/// completion packet is ever queued for it. Every other nonzero code is
/// `Err(Error::from_hresult(HRESULT::from_win32(ioresult)))`.
#[cfg_attr(test, track_caller)]
pub(crate) fn await_io(
    reg: &Arc<IoReg>,
    issue: impl FnOnce(*mut OVERLAPPED) -> Issue,
) -> impl Future<Output = Result<usize>> {
    let op = unsafe { tpio::submit(reg, |_, _| {}, issue) };
    AwaitIo { op }
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
#[cfg_attr(test, track_caller)]
pub(crate) fn await_io_owning<P: Send + 'static>(
    reg: &Arc<IoReg>,
    payload: P,
    issue: impl FnOnce(*mut OVERLAPPED) -> Issue + Send + 'static,
) -> impl Future<Output = Result<usize>> {
    // Dropped at the real completion (whether the op ran to natural
    // completion or was aborted by `OpHandle::cancel` and completed with
    // `ERROR_OPERATION_ABORTED`), never earlier — see the contract above.
    let op = unsafe { tpio::submit(reg, move |_, _| drop(payload), issue) };
    AwaitIo { op }
}
