//! Expose an arbitrary Rust `Future` to WinRT consumers as an awaitable
//! `IAsyncOperation<T>` / `IAsyncAction`.
//!
//! This is the "promise" half only — a COM object plus a [`Completer`] handle,
//! like [`super`]'s [`oneshot`](crate::async_util::oneshot) but where the
//! receiving side is a real WinRT async object (so C#/XAML/WinRT can `await` it).
//! The producing side stays runtime-agnostic: `tp` or `dq` just run a normal
//! future and call `completer.complete(result)` when done — neither runtime
//! needs any COM code of its own.
//!
//! ```ignore
//! let (op, completer) = future_op::operation::<u32>();
//! rt.spawn(async move { completer.complete(Ok(compute().await)); }); // tp
//! op   // hand to WinRT: `await`-able IAsyncOperation<u32>
//! ```
//! or, with a `!Send` future on the UI thread:
//! ```ignore
//! let (op, completer) = future_op::operation::<u32>();
//! dq::spawn_local(async move { completer.complete(Ok(compute_local().await)); });
//! op
//! ```
//!
//! Cancellation is cooperative: `IAsyncInfo::Cancel` finalizes the op as
//! `Canceled` (so a waiter unblocks immediately) and flips a flag the producer
//! can read via [`Completer::is_canceled`] to stop early. If the `Completer` is
//! dropped without completing (e.g. the producing task was aborted), the op is
//! finalized with `E_ABORT` rather than hanging forever.

use std::sync::Mutex;
use windows_core::{
    implement, ComObject, Error, IUnknownImpl, Ref, Result, RuntimeType, HRESULT,
};
use windows_future::{
    AsyncActionCompletedHandler, AsyncOperationCompletedHandler, AsyncStatus, IAsyncAction,
    IAsyncAction_Impl, IAsyncInfo_Impl, IAsyncOperation, IAsyncOperation_Impl,
};

/// E_ILLEGAL_METHOD_CALL — `GetResults` before completion.
const E_ILLEGAL_METHOD_CALL: HRESULT = HRESULT(0x8000000Eu32 as i32);
/// E_ILLEGAL_DELEGATE_ASSIGNMENT — `Completed` set more than once.
const E_ILLEGAL_DELEGATE_ASSIGNMENT: HRESULT = HRESULT(0x80000018u32 as i32);
/// E_ABORT — a cancelled op's result, and the drop-without-complete fallback.
const E_ABORT: HRESULT = HRESULT(0x80004004u32 as i32);

// ---------------------------------------------------------------------------
// IAsyncOperation<T>
// ---------------------------------------------------------------------------

struct OpState<T: RuntimeType + 'static> {
    result: Option<Result<T>>,
    completed: Option<AsyncOperationCompletedHandler<T>>,
    completed_assigned: bool,
    canceled: bool,
}

impl<T: RuntimeType + 'static> OpState<T> {
    fn status(&self) -> AsyncStatus {
        match &self.result {
            Some(Ok(_)) => AsyncStatus::Completed,
            _ if self.canceled => AsyncStatus::Canceled,
            Some(Err(_)) => AsyncStatus::Error,
            None => AsyncStatus::Started,
        }
    }
}

#[implement(IAsyncOperation<T>, windows_future::IAsyncInfo)]
struct FutureOp<T>(Mutex<OpState<T>>)
where
    T: RuntimeType + 'static;

// SAFETY: the only non-`Send` data (the pending `T`/handler WinRT interfaces)
// lives behind the `Mutex`; the object is agile (`#[implement]` default), so its
// interfaces may be used from any thread — including the producer's. Mirrors
// `super::asyncop::IoAsyncOp`.
unsafe impl<T: RuntimeType + 'static> Send for FutureOp<T> {}
unsafe impl<T: RuntimeType + 'static> Sync for FutureOp<T> {}

impl<T: RuntimeType + 'static> FutureOp_Impl<T> {
    /// Stores the outcome (first write wins) and fires the `Completed` handler.
    fn complete(&self, res: Result<T>) {
        let (status, completed) = {
            let mut g = self.0.lock().unwrap();
            if g.result.is_some() {
                return; // already finalized (normal completion, cancel, or drop)
            }
            g.result = Some(res);
            (g.status(), g.completed.take())
        };
        if let Some(h) = completed {
            let sender: IAsyncOperation<T> = self.to_interface();
            let _ = h.Invoke(&sender, status);
        }
    }
}

impl<T: RuntimeType + 'static> IAsyncInfo_Impl for FutureOp_Impl<T> {
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
        // Finalize as cancelled so a waiter unblocks; the producer sees the flag.
        let (fire, completed) = {
            let mut g = self.0.lock().unwrap();
            g.canceled = true;
            if g.result.is_none() {
                g.result = Some(Err(Error::from_hresult(E_ABORT)));
                (true, g.completed.take())
            } else {
                (false, None)
            }
        };
        if fire {
            if let Some(h) = completed {
                let sender: IAsyncOperation<T> = self.to_interface();
                let _ = h.Invoke(&sender, AsyncStatus::Canceled);
            }
        }
        Ok(())
    }
    fn Close(&self) -> Result<()> {
        Ok(())
    }
}

impl<T: RuntimeType + 'static> IAsyncOperation_Impl<T> for FutureOp_Impl<T> {
    fn SetCompleted(&self, handler: Ref<AsyncOperationCompletedHandler<T>>) -> Result<()> {
        let mut g = self.0.lock().unwrap();
        if g.completed_assigned {
            return Err(Error::from_hresult(E_ILLEGAL_DELEGATE_ASSIGNMENT));
        }
        g.completed_assigned = true;
        let status = g.status();
        let handler = handler.ok()?;
        if status == AsyncStatus::Started {
            g.completed = Some(handler.clone());
        } else {
            drop(g);
            let sender: IAsyncOperation<T> = self.to_interface();
            let _ = handler.Invoke(&sender, status);
        }
        Ok(())
    }
    fn Completed(&self) -> Result<AsyncOperationCompletedHandler<T>> {
        Err(Error::empty())
    }
    fn GetResults(&self) -> Result<T> {
        match &self.0.lock().unwrap().result {
            Some(result) => result.clone(),
            None => Err(Error::from_hresult(E_ILLEGAL_METHOD_CALL)),
        }
    }
}

/// Fulfils the `IAsyncOperation<T>` returned alongside it. `Send`/`Sync` (the
/// object is agile), so it can be moved into a `tp`/`dq`/thread task.
pub struct Completer<T: RuntimeType + 'static> {
    op: ComObject<FutureOp<T>>,
}

impl<T: RuntimeType + 'static> Completer<T> {
    /// Completes the operation with `result`, firing its `Completed` handler.
    pub fn complete(self, result: Result<T>) {
        self.op.complete(result);
        // `self` drops here; the drop guard sees the result is set and no-ops.
    }

    /// Whether the consumer requested cancellation (via `IAsyncInfo::Cancel`).
    pub fn is_canceled(&self) -> bool {
        self.op.0.lock().unwrap().canceled
    }
}

impl<T: RuntimeType + 'static> Drop for Completer<T> {
    fn drop(&mut self) {
        // If the producer never completed (task dropped/aborted), finalize as
        // E_ABORT so the op resolves rather than hanging. No-op if already set.
        self.op.complete(Err(Error::from_hresult(E_ABORT)));
    }
}

/// Creates an `IAsyncOperation<T>` and the [`Completer`] that fulfils it.
pub fn operation<T: RuntimeType + 'static>() -> (IAsyncOperation<T>, Completer<T>) {
    let op = ComObject::new(FutureOp::<T>(Mutex::new(OpState {
        result: None,
        completed: None,
        completed_assigned: false,
        canceled: false,
    })));
    let iface: IAsyncOperation<T> = op.to_interface();
    (iface, Completer { op })
}

// ---------------------------------------------------------------------------
// IAsyncAction (the T = () twin)
// ---------------------------------------------------------------------------

struct ActionState {
    result: Option<Result<()>>,
    completed: Option<AsyncActionCompletedHandler>,
    completed_assigned: bool,
    canceled: bool,
}

impl ActionState {
    fn status(&self) -> AsyncStatus {
        match &self.result {
            Some(Ok(())) => AsyncStatus::Completed,
            _ if self.canceled => AsyncStatus::Canceled,
            Some(Err(_)) => AsyncStatus::Error,
            None => AsyncStatus::Started,
        }
    }
}

#[implement(IAsyncAction, windows_future::IAsyncInfo)]
struct FutureAction(Mutex<ActionState>);

unsafe impl Send for FutureAction {}
unsafe impl Sync for FutureAction {}

impl FutureAction_Impl {
    fn complete(&self, res: Result<()>) {
        let (status, completed) = {
            let mut g = self.0.lock().unwrap();
            if g.result.is_some() {
                return;
            }
            g.result = Some(res);
            (g.status(), g.completed.take())
        };
        if let Some(h) = completed {
            let sender: IAsyncAction = self.to_interface();
            let _ = h.Invoke(&sender, status);
        }
    }
}

impl IAsyncInfo_Impl for FutureAction_Impl {
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
        let (fire, completed) = {
            let mut g = self.0.lock().unwrap();
            g.canceled = true;
            if g.result.is_none() {
                g.result = Some(Err(Error::from_hresult(E_ABORT)));
                (true, g.completed.take())
            } else {
                (false, None)
            }
        };
        if fire {
            if let Some(h) = completed {
                let sender: IAsyncAction = self.to_interface();
                let _ = h.Invoke(&sender, AsyncStatus::Canceled);
            }
        }
        Ok(())
    }
    fn Close(&self) -> Result<()> {
        Ok(())
    }
}

impl IAsyncAction_Impl for FutureAction_Impl {
    fn SetCompleted(&self, handler: Ref<AsyncActionCompletedHandler>) -> Result<()> {
        let mut g = self.0.lock().unwrap();
        if g.completed_assigned {
            return Err(Error::from_hresult(E_ILLEGAL_DELEGATE_ASSIGNMENT));
        }
        g.completed_assigned = true;
        let status = g.status();
        let handler = handler.ok()?;
        if status == AsyncStatus::Started {
            g.completed = Some(handler.clone());
        } else {
            drop(g);
            let sender: IAsyncAction = self.to_interface();
            let _ = handler.Invoke(&sender, status);
        }
        Ok(())
    }
    fn Completed(&self) -> Result<AsyncActionCompletedHandler> {
        Err(Error::empty())
    }
    fn GetResults(&self) -> Result<()> {
        match &self.0.lock().unwrap().result {
            Some(result) => result.clone(),
            None => Err(Error::from_hresult(E_ILLEGAL_METHOD_CALL)),
        }
    }
}

/// Fulfils the `IAsyncAction` returned alongside it (the `T = ()` twin of
/// [`Completer`]).
pub struct ActionCompleter {
    op: ComObject<FutureAction>,
}

impl ActionCompleter {
    /// Completes the action, firing its `Completed` handler.
    pub fn complete(self, result: Result<()>) {
        self.op.complete(result);
    }
    /// Whether the consumer requested cancellation.
    pub fn is_canceled(&self) -> bool {
        self.op.0.lock().unwrap().canceled
    }
}

impl Drop for ActionCompleter {
    fn drop(&mut self) {
        self.op.complete(Err(Error::from_hresult(E_ABORT)));
    }
}

/// Creates an `IAsyncAction` and the [`ActionCompleter`] that fulfils it.
pub fn action() -> (IAsyncAction, ActionCompleter) {
    let op = ComObject::new(FutureAction(Mutex::new(ActionState {
        result: None,
        completed: None,
        completed_assigned: false,
        canceled: false,
    })));
    let iface: IAsyncAction = op.to_interface();
    (iface, ActionCompleter { op })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tp::{block_on, Runtime};
    use std::future::IntoFuture;
    use windows_core::Interface;
    use windows_future::IAsyncInfo;

    #[test]
    fn operation_completes_from_pool() {
        let rt = Runtime::new();
        let (op, completer) = operation::<u32>();
        rt.spawn(async move {
            // Any tp future; here just produce a value.
            completer.complete(Ok(7));
        });
        assert_eq!(block_on(op.into_future()).unwrap(), 7);
    }

    #[test]
    fn operation_cancel_finalizes() {
        let (op, _completer) = operation::<u32>();
        let info: IAsyncInfo = op.cast().unwrap();
        info.Cancel().unwrap();
        assert_eq!(info.Status().unwrap(), AsyncStatus::Canceled);
        assert!(block_on(op.into_future()).is_err());
    }

    #[test]
    fn operation_abandoned_on_drop() {
        let (op, completer) = operation::<u32>();
        drop(completer); // producer never completed
        let info: IAsyncInfo = op.cast().unwrap();
        assert_eq!(info.Status().unwrap(), AsyncStatus::Error);
        assert!(block_on(op.into_future()).is_err());
    }

    #[test]
    fn action_completes_from_pool() {
        let rt = Runtime::new();
        let (act, completer) = action();
        rt.spawn(async move {
            completer.complete(Ok(()));
        });
        block_on(act.into_future()).unwrap();
    }
}
