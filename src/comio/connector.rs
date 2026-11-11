//! COM connectors: the client half of [`super::listener`].
//!
//! The listeners are reachable from COM; without these the transports were
//! only half exposed — an external caller could accept a connection but never
//! make one. Same shape as the listeners: one concrete type per transport,
//! each carrying the connect its addressing supports, because a pipe name, a
//! filesystem path and a pair of GUIDs cannot share one signature.
//!
//! The interfaces themselves are no longer declared here. They live in
//! `metadata/winasync.rdl` and are generated into [`super::bindings`],
//! because a hand-written `#[interface]` cannot derive from `IInspectable`
//! and so cannot be a WinRT interface at all. What stays in this module is
//! the part metadata cannot express: the operation object behind every
//! `ConnectAsync`.
//!
//! # No blocked threads
//!
//! Each `ConnectAsync` returns an `IAsyncOperation<ITransportStream>` whose
//! result is delivered **from the overlapped completion**, not by running the
//! connect to completion on a thread. AF_UNIX and AF_HYPERV arm a `ConnectEx`
//! through `tpio::submit` and fill the operation from its callback — the same
//! shape `super::asyncop::IoAsyncOp` uses for reads and writes — so nothing
//! waits while a connect is outstanding, and `IAsyncInfo::Cancel` cancels the
//! real IO.
//!
//! The named pipe is the exception, and only partly: opening the client end
//! is a plain synchronous `CreateFileW`, so the usual case is done inline and
//! returns an already-finished operation, with no thread involved at all.
//! Only `ERROR_PIPE_BUSY` has something to wait for, and that wait is handed
//! to the thread pool rather than to the caller.

// PascalCase members are the COM ABI convention the generated traits are
// shaped to, not Rust naming slips.
#![allow(non_snake_case)]

use crate::comio::unload::ModuleRef;
use std::sync::Mutex;
use windows_core::{ComObject, HRESULT, IUnknownImpl, Result, implement};
use windows_future::{
    AsyncOperationCompletedHandler, AsyncStatus, IAsyncInfo_Impl, IAsyncOperation,
    IAsyncOperation_Impl,
};

use super::bindings::Pbk20191::WinAsync::Comio::ITransportStream;
use super::tpio::OpHandle;

/// An `IAsyncOperation<ITransportStream>` whose result arrives from an overlapped
/// completion rather than from a closure run to completion on a thread.
///
/// The same state machine as `super::asyncop::IoAsyncOp`, which does this for
/// reads and writes; this one is fixed to `IAsyncOperation<ITransportStream>`,
/// the shape a connect returns. `IAsyncInfo::Cancel` cancels the in-flight
/// `ConnectEx`.
struct ConnectState {
    result: Option<Result<ITransportStream>>,
    /// See `asyncop::settle`.
    cancel_requested: bool,
    canceled: bool,
    completed: Option<AsyncOperationCompletedHandler<ITransportStream>>,
    completed_assigned: bool,
    op: Option<OpHandle>,
}

#[implement(IAsyncOperation<ITransportStream>, windows_future::IAsyncInfo)]
struct ConnectOp(Mutex<ConnectState>, ModuleRef);

// SAFETY: the only non-`Send` data (the pending handler) lives behind the
// `Mutex` and every access is serialized by it; the object is agile, so its
// interfaces may be used from the completion callback's thread. Mirrors
// `asyncop::IoAsyncOp`.
unsafe impl Send for ConnectOp {}
unsafe impl Sync for ConnectOp {}

impl ConnectOp_Impl {
    /// Stores the outcome and fires `Completed` if one is already assigned.
    /// Runs on the thread-pool completion thread.
    fn complete(&self, res: Result<ITransportStream>) {
        let (status, completed) = {
            let mut g = self.0.lock().unwrap();
            debug_assert!(g.result.is_none());
            let (res, canceled) = super::asyncop::settle(res, g.cancel_requested);
            g.result = Some(res);
            g.canceled = canceled;
            (g.status(), g.completed.take())
        };
        // Outside the lock: the handler may re-enter to read Status/GetResults.
        if let Some(completed) = completed {
            let sender: IAsyncOperation<ITransportStream> = self.to_interface();
            let _ = completed.Invoke(&sender, status);
        }
    }
}

impl ConnectState {
    fn status(&self) -> AsyncStatus {
        match &self.result {
            None => AsyncStatus::Started,
            Some(Ok(_)) => AsyncStatus::Completed,
            Some(Err(_)) if self.canceled => AsyncStatus::Canceled,
            Some(Err(_)) => AsyncStatus::Error,
        }
    }
}

impl IAsyncInfo_Impl for ConnectOp_Impl {
    fn Id(&self) -> Result<u32> {
        Ok(1)
    }
    fn Status(&self) -> Result<AsyncStatus> {
        Ok(self.0.lock().unwrap().status())
    }
    fn ErrorCode(&self) -> Result<HRESULT> {
        Ok(match &self.0.lock().unwrap().result {
            Some(Err(e)) => e.code(),
            _ => HRESULT(0),
        })
    }
    fn Cancel(&self) -> Result<()> {
        let mut g = self.0.lock().unwrap();
        if g.result.is_none() {
            g.cancel_requested = true;
            if let Some(op) = g.op.as_ref() {
                op.cancel();
            }
        }
        Ok(())
    }
    fn Close(&self) -> Result<()> {
        Ok(())
    }
}

impl IAsyncOperation_Impl<ITransportStream> for ConnectOp_Impl {
    fn SetCompleted(
        &self,
        handler: windows_core::Ref<'_, AsyncOperationCompletedHandler<ITransportStream>>,
    ) -> Result<()> {
        let (fire, status) = {
            let mut g = self.0.lock().unwrap();
            if g.completed_assigned {
                return Err(windows_core::Error::from_hresult(E_ILLEGAL_DELEGATE_ASSIGNMENT));
            }
            g.completed_assigned = true;
            let h = handler.ok()?.clone();
            // Already finished: fire now rather than storing a handler that
            // nothing will ever call.
            if g.result.is_some() {
                (Some(h), g.status())
            } else {
                g.completed = Some(h);
                (None, g.status())
            }
        };
        if let Some(h) = fire {
            let sender: IAsyncOperation<ITransportStream> = self.to_interface();
            let _ = h.Invoke(&sender, status);
        }
        Ok(())
    }

    fn Completed(&self) -> Result<AsyncOperationCompletedHandler<ITransportStream>> {
        self.0
            .lock()
            .unwrap()
            .completed
            .clone()
            .ok_or_else(|| windows_core::Error::from_hresult(E_ILLEGAL_METHOD_CALL))
    }

    fn GetResults(&self) -> Result<ITransportStream> {
        match &self.0.lock().unwrap().result {
            Some(r) => r.clone(),
            None => Err(windows_core::Error::from_hresult(E_ILLEGAL_METHOD_CALL)),
        }
    }
}

/// E_ILLEGAL_METHOD_CALL — results read before completion.
const E_ILLEGAL_METHOD_CALL: HRESULT = HRESULT(0x8000000Eu32 as i32);
/// E_ILLEGAL_DELEGATE_ASSIGNMENT — `Completed` set twice.
const E_ILLEGAL_DELEGATE_ASSIGNMENT: HRESULT = HRESULT(0x80000018u32 as i32);

/// A fresh, unfinished operation. Counted from here on: `ConnectOp` carries a
/// `ModuleRef`, which is the point of building every connect result through
/// this type rather than through `windows_future`.
fn new_op() -> ComObject<ConnectOp> {
    ComObject::new(ConnectOp(
        Mutex::new(ConnectState {
            result: None,
            cancel_requested: false,
            canceled: false,
            completed: None,
            completed_assigned: false,
            op: None,
        }),
        ModuleRef::new(),
    ))
}

/// Where an armed connect delivers its one result: the operation handed to
/// the caller.
pub(crate) struct ConnectSink(ComObject<ConnectOp>);

impl ConnectSink {
    pub(crate) fn deliver(self, res: Result<ITransportStream>) {
        self.0.complete(res);
    }
}

/// Builds an operation and arms `start`, which must deliver exactly one
/// result through the sink it is given.
pub(crate) fn connect_op(
    start: impl FnOnce(ConnectSink) -> Result<OpHandle>,
) -> IAsyncOperation<ITransportStream> {
    let op = new_op();
    match start(ConnectSink(op.clone())) {
        Ok(handle) => {
            op.0.lock().unwrap().op = Some(handle);
            op.to_interface()
        }
        // Never armed, so nothing will call back: finish it here. Through
        // `op` rather than `IAsyncOperation::ready`, so the object handed
        // back is one this module counts -- see `comio::unload`.
        Err(e) => {
            op.complete(Err(e));
            op.to_interface()
        }
    }
}

/// An operation that already has its result.
///
/// What `windows_future::IAsyncOperation::ready` would give, except that the
/// object is ours and therefore counted. A named-pipe connect is a plain
/// synchronous `CreateFileW`, so this is the shape of nearly every pipe
/// connect there is -- which is why handing out the uncounted kind here was
/// the largest hole in `DllCanUnloadNow`'s answer.
pub(crate) fn finished(res: Result<ITransportStream>) -> IAsyncOperation<ITransportStream> {
    let op = new_op();
    op.complete(res);
    op.to_interface()
}

/// An operation whose result `f` produces on the thread pool.
///
/// What `windows_future::IAsyncOperation::spawn` would give, with both of its
/// gaps closed: the object is counted, and the work item goes through
/// `comio::pool::submit`, so it is counted while queued and the module is
/// pinned while it runs. `Cancel` does nothing, as with `spawn`: there is no
/// overlapped operation to cancel, only a closure already on its way.
pub(crate) fn spawned(
    f: impl FnOnce() -> Result<ITransportStream> + Send + 'static,
) -> IAsyncOperation<ITransportStream> {
    let op = new_op();
    let sink = op.clone();
    super::pool::submit(move || sink.complete(f()));
    op.to_interface()
}

#[cfg(test)]
#[path = "connector_tests.rs"]
mod tests;
