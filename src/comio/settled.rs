//! Async results that are finished before they are handed out.
//!
//! Several vtable members have nothing to wait for. `BindAsync` installs a
//! source synchronously, `FlushAsync` has no buffer to flush, and a connect
//! that failed to arm has already failed. WinRT still requires them to return
//! an `IAsyncAction` or `IAsyncOperation<T>`, so these two types are those
//! interfaces over a result that is already in hand.
//!
//! # Why not `windows_future`
//!
//! `IAsyncAction::spawn(|| Ok(()))` says the same thing in one line, and that
//! is what these replaced. The problem is not what the object does — it is
//! *whose* object it is. `windows-future` is a Rust dependency compiled into
//! this crate, so the vtable it hands a caller points into **this module**,
//! and nothing tells us when the caller releases it. One of those objects
//! outstanding is enough to make unloading this module a jump into freed
//! code, and [`crate::comio::unload`] would never know.
//!
//! Owning the type is what buys the `ModuleRef`. That is the whole reason
//! these exist; the async behaviour is deliberately the least interesting
//! thing about them.
//!
//! # Behaviour
//!
//! `Status` is `Completed` from birth, so `SetCompleted` invokes its handler
//! inline rather than storing one nothing would ever call. `Cancel` is a
//! no-op that succeeds — there is nothing left to cancel, and WinRT lets a
//! finished operation ignore it. `Close` likewise.

use crate::comio::unload::ModuleRef;
use std::sync::Mutex;
use windows_core::{HRESULT, IUnknownImpl, Result, RuntimeType, implement};
use windows_future::{
    AsyncActionCompletedHandler, AsyncOperationCompletedHandler, AsyncStatus, IAsyncAction,
    IAsyncAction_Impl, IAsyncInfo, IAsyncInfo_Impl, IAsyncOperation, IAsyncOperation_Impl,
};

/// E_ILLEGAL_METHOD_CALL — `Completed` read before it was set.
const E_ILLEGAL_METHOD_CALL: HRESULT = HRESULT(0x8000000Eu32 as i32);
/// E_ILLEGAL_DELEGATE_ASSIGNMENT — `Completed` set twice.
const E_ILLEGAL_DELEGATE_ASSIGNMENT: HRESULT = HRESULT(0x80000018u32 as i32);

/// An `IAsyncAction` that has already succeeded.
#[implement(IAsyncAction, IAsyncInfo)]
pub(crate) struct SettledAction {
    completed: Mutex<Option<AsyncActionCompletedHandler>>,
    _module: ModuleRef,
}

// SAFETY: the only non-`Send` datum is the handler, which lives behind the
// `Mutex` and is only touched while it is held. Mirrors `ConnectOp`.
unsafe impl Send for SettledAction {}
unsafe impl Sync for SettledAction {}

/// The action every synchronous bind returns.
pub(crate) fn action_done() -> IAsyncAction {
    SettledAction {
        completed: Mutex::new(None),
        _module: ModuleRef::new(),
    }
    .into()
}

impl IAsyncInfo_Impl for SettledAction_Impl {
    fn Id(&self) -> Result<u32> {
        Ok(1)
    }
    fn Status(&self) -> Result<AsyncStatus> {
        Ok(AsyncStatus::Completed)
    }
    fn ErrorCode(&self) -> Result<HRESULT> {
        Ok(HRESULT(0))
    }
    fn Cancel(&self) -> Result<()> {
        Ok(())
    }
    fn Close(&self) -> Result<()> {
        Ok(())
    }
}

impl IAsyncAction_Impl for SettledAction_Impl {
    fn SetCompleted(
        &self,
        handler: windows_core::Ref<'_, AsyncActionCompletedHandler>,
    ) -> Result<()> {
        let h = {
            let mut g = self.completed.lock().unwrap();
            if g.is_some() {
                return Err(windows_core::Error::from_hresult(
                    E_ILLEGAL_DELEGATE_ASSIGNMENT,
                ));
            }
            let h = handler.ok()?.clone();
            *g = Some(h.clone());
            h
        };
        // Outside the lock: the handler may re-enter to read `Status`.
        let sender: IAsyncAction = self.to_interface();
        let _ = h.Invoke(&sender, AsyncStatus::Completed);
        Ok(())
    }

    fn Completed(&self) -> Result<AsyncActionCompletedHandler> {
        self.completed
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| windows_core::Error::from_hresult(E_ILLEGAL_METHOD_CALL))
    }

    fn GetResults(&self) -> Result<()> {
        Ok(())
    }
}

/// An `IAsyncOperation<T>` that has already produced `value`.
#[implement(IAsyncOperation<T>, IAsyncInfo)]
pub(crate) struct SettledOp<T>
where
    T: RuntimeType + 'static,
{
    value: T,
    completed: Mutex<Option<AsyncOperationCompletedHandler<T>>>,
    _module: ModuleRef,
}

// SAFETY: as for `SettledAction`. `T` here is only ever a WinRT value type
// or an agile interface produced by this crate.
unsafe impl<T: RuntimeType + 'static> Send for SettledOp<T> {}
unsafe impl<T: RuntimeType + 'static> Sync for SettledOp<T> {}

/// The operation a member returns when its result needs no waiting.
pub(crate) fn op_done<T: RuntimeType + 'static>(value: T) -> IAsyncOperation<T> {
    SettledOp {
        value,
        completed: Mutex::new(None),
        _module: ModuleRef::new(),
    }
    .into()
}

impl<T: RuntimeType + 'static> IAsyncInfo_Impl for SettledOp_Impl<T> {
    fn Id(&self) -> Result<u32> {
        Ok(1)
    }
    fn Status(&self) -> Result<AsyncStatus> {
        Ok(AsyncStatus::Completed)
    }
    fn ErrorCode(&self) -> Result<HRESULT> {
        Ok(HRESULT(0))
    }
    fn Cancel(&self) -> Result<()> {
        Ok(())
    }
    fn Close(&self) -> Result<()> {
        Ok(())
    }
}

impl<T: RuntimeType + 'static> IAsyncOperation_Impl<T> for SettledOp_Impl<T> {
    fn SetCompleted(
        &self,
        handler: windows_core::Ref<'_, AsyncOperationCompletedHandler<T>>,
    ) -> Result<()> {
        let h = {
            let mut g = self.completed.lock().unwrap();
            if g.is_some() {
                return Err(windows_core::Error::from_hresult(
                    E_ILLEGAL_DELEGATE_ASSIGNMENT,
                ));
            }
            let h = handler.ok()?.clone();
            *g = Some(h.clone());
            h
        };
        let sender: IAsyncOperation<T> = self.to_interface();
        let _ = h.Invoke(&sender, AsyncStatus::Completed);
        Ok(())
    }

    fn Completed(&self) -> Result<AsyncOperationCompletedHandler<T>> {
        self.completed
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| windows_core::Error::from_hresult(E_ILLEGAL_METHOD_CALL))
    }

    fn GetResults(&self) -> Result<T> {
        Ok(self.value.clone())
    }
}
