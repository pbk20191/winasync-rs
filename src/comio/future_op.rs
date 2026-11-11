//! A Rust future driven on the process thread pool, seen from COM as an
//! `IAsyncOperation<T>` or an `IAsyncAction`.
//!
//! comio's other operations are armed callbacks (`tpio::submit`); the http
//! surface is written as futures over the shared core, so these run one.
//! There is no executor: each wake submits one poll to the pool, and the
//! future lives in the operation object until it is done or cancelled.
//! `Cancel` drops the future, which cancels whatever it had in flight.

use crate::comio::unload::ModuleRef;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use windows_core::{ComObject, ComObjectInner, Error, IUnknownImpl, Ref, Result, RuntimeType, HRESULT};
use winasync_implement::implement;
use windows_future::{
    AsyncActionCompletedHandler, AsyncOperationCompletedHandler, AsyncStatus, IAsyncAction,
    IAsyncAction_Impl, IAsyncInfo_Impl, IAsyncOperation, IAsyncOperation_Impl,
};

const E_ILLEGAL_METHOD_CALL: HRESULT = HRESULT(0x8000000Eu32 as i32);
const E_ILLEGAL_DELEGATE_ASSIGNMENT: HRESULT = HRESULT(0x80000018u32 as i32);

/// The outcome and who to tell; `H` is the completed-handler type.
struct State<T, H> {
    result: Option<Result<T>>,
    canceled: bool,
    completed: Option<H>,
    completed_assigned: bool,
}

impl<T, H> State<T, H> {
    fn new() -> Self {
        State {
            result: None,
            canceled: false,
            completed: None,
            completed_assigned: false,
        }
    }

    fn status(&self) -> AsyncStatus {
        match &self.result {
            None => AsyncStatus::Started,
            Some(Ok(_)) => AsyncStatus::Completed,
            Some(Err(_)) if self.canceled => AsyncStatus::Canceled,
            Some(Err(_)) => AsyncStatus::Error,
        }
    }

    fn error_code(&self) -> HRESULT {
        match &self.result {
            Some(Err(e)) => e.code(),
            _ => HRESULT(0),
        }
    }

    /// Records the outcome, once; the handler to fire, if any.
    fn settle(&mut self, outcome: Result<T>, canceled: bool) -> Option<(H, AsyncStatus)> {
        if self.result.is_some() {
            return None;
        }
        self.result = Some(outcome);
        self.canceled = canceled;
        self.completed.take().map(|h| (h, self.status()))
    }

    /// `SetCompleted`: the handler to fire now if the outcome is already in.
    fn assign(&mut self, handler: Option<H>) -> Result<Option<(H, AsyncStatus)>> {
        if self.completed_assigned {
            return Err(Error::from_hresult(E_ILLEGAL_DELEGATE_ASSIGNMENT));
        }
        self.completed_assigned = true;
        if self.result.is_some() {
            Ok(handler.map(|h| (h, self.status())))
        } else {
            self.completed = handler;
            Ok(None)
        }
    }

    fn results(&self) -> Result<T>
    where
        T: Clone,
    {
        match &self.result {
            Some(r) => r.clone(),
            None => Err(Error::from_hresult(E_ILLEGAL_METHOD_CALL)),
        }
    }
}

/// The future, until it finishes or is cancelled.
type Slot<F> = Mutex<Option<Pin<Box<F>>>>;

/// One poll of the future in an operation object; `true` when it finished.
trait Step {
    fn step(&self, cx: &mut Context<'_>);
    fn cancel(&self);
}

/// What a waker holds: the operation object, which holds the future.
struct Driver<O: ComObjectInner>(ComObject<O>);

impl<O> Wake for Driver<O>
where
    O: ComObjectInner + Send + Sync + 'static,
    O::Outer: Step,
{
    fn wake(self: Arc<Self>) {
        let me = self.clone();
        super::pool::submit(move || {
            let waker = Waker::from(me.clone());
            me.0.step(&mut Context::from_waker(&waker));
        });
    }
}

/// Polls `slot`'s future; `Some` when it finished (and was dropped).
fn poll_slot<F: Future>(slot: &Slot<F>, cx: &mut Context<'_>) -> Option<F::Output> {
    let mut g = slot.lock().unwrap();
    let fut = g.as_mut()?;
    match fut.as_mut().poll(cx) {
        Poll::Pending => None,
        Poll::Ready(r) => {
            *g = None;
            Some(r)
        }
    }
}

// ---------------------------------------------------------------- operation

#[implement(IAsyncOperation<T>, windows_future::IAsyncInfo)]
pub(crate) struct FutureOp<T, F>
where
    T: RuntimeType + 'static,
    F: Future<Output = Result<T>> + Send + 'static,
{
    state: Mutex<State<T, AsyncOperationCompletedHandler<T>>>,
    fut: Slot<F>,
    _module: ModuleRef,
}

// SAFETY: the handler, the only non-`Send` datum, lives behind the `Mutex`;
// the future is `Send`. Mirrors `ConnectOp`.
unsafe impl<T: RuntimeType + 'static, F: Future<Output = Result<T>> + Send + 'static> Send for FutureOp<T, F> {}
unsafe impl<T: RuntimeType + 'static, F: Future<Output = Result<T>> + Send + 'static> Sync for FutureOp<T, F> {}

impl<T: RuntimeType + 'static, F: Future<Output = Result<T>> + Send + 'static> FutureOp_Impl<T, F> {
    fn deliver(&self, outcome: Result<T>, canceled: bool) {
        let fire = self.state.lock().unwrap().settle(outcome, canceled);
        if let Some((handler, status)) = fire {
            let sender: IAsyncOperation<T> = self.to_interface();
            let _ = handler.Invoke(&sender, status);
        }
    }
}

impl<T: RuntimeType + 'static, F: Future<Output = Result<T>> + Send + 'static> Step for FutureOp_Impl<T, F> {
    fn step(&self, cx: &mut Context<'_>) {
        if let Some(outcome) = poll_slot(&self.fut, cx) {
            self.deliver(outcome, false);
        }
    }
    fn cancel(&self) {
        // Dropping the future cancels what it has in flight; a poll under
        // way finishes first, since it holds the slot.
        if self.fut.lock().unwrap().take().is_some() {
            self.deliver(Err(Error::from_hresult(super::asyncop::E_CANCELLED)), true);
        }
    }
}

impl<T: RuntimeType + 'static, F: Future<Output = Result<T>> + Send + 'static> IAsyncInfo_Impl for FutureOp_Impl<T, F> {
    fn Id(&self) -> Result<u32> {
        Ok(1)
    }
    fn Status(&self) -> Result<AsyncStatus> {
        Ok(self.state.lock().unwrap().status())
    }
    fn ErrorCode(&self) -> Result<HRESULT> {
        Ok(self.state.lock().unwrap().error_code())
    }
    fn Cancel(&self) -> Result<()> {
        Step::cancel(self);
        Ok(())
    }
    fn Close(&self) -> Result<()> {
        Ok(())
    }
}

impl<T: RuntimeType + 'static, F: Future<Output = Result<T>> + Send + 'static> IAsyncOperation_Impl<T> for FutureOp_Impl<T, F> {
    fn SetCompleted(&self, handler: Ref<'_, AsyncOperationCompletedHandler<T>>) -> Result<()> {
        let fire = self.state.lock().unwrap().assign(handler.cloned())?;
        if let Some((handler, status)) = fire {
            let sender: IAsyncOperation<T> = self.to_interface();
            let _ = handler.Invoke(&sender, status);
        }
        Ok(())
    }
    fn Completed(&self) -> Result<AsyncOperationCompletedHandler<T>> {
        self.state.lock().unwrap().completed.clone().ok_or_else(Error::empty)
    }
    fn GetResults(&self) -> Result<T> {
        self.state.lock().unwrap().results()
    }
}

/// Runs `fut` on the pool; its result is the operation's.
pub(crate) fn spawn_op<T, F>(fut: F) -> IAsyncOperation<T>
where
    T: RuntimeType + 'static,
    F: Future<Output = Result<T>> + Send + 'static,
{
    let object = ComObject::new(FutureOp {
        state: Mutex::new(State::new()),
        fut: Mutex::new(Some(Box::pin(fut))),
        _module: ModuleRef::new(),
    });
    let iface = object.to_interface();
    Arc::new(Driver(object)).wake();
    iface
}

// ---------------------------------------------------------------- action

#[implement(IAsyncAction, windows_future::IAsyncInfo)]
pub(crate) struct FutureAction<F>
where
    F: Future<Output = Result<()>> + Send + 'static,
{
    state: Mutex<State<(), AsyncActionCompletedHandler>>,
    fut: Slot<F>,
    _module: ModuleRef,
}

// SAFETY: as for `FutureOp`.
unsafe impl<F: Future<Output = Result<()>> + Send + 'static> Send for FutureAction<F> {}
unsafe impl<F: Future<Output = Result<()>> + Send + 'static> Sync for FutureAction<F> {}

impl<F: Future<Output = Result<()>> + Send + 'static> FutureAction_Impl<F> {
    fn deliver(&self, outcome: Result<()>, canceled: bool) {
        let fire = self.state.lock().unwrap().settle(outcome, canceled);
        if let Some((handler, status)) = fire {
            let sender: IAsyncAction = self.to_interface();
            let _ = handler.Invoke(&sender, status);
        }
    }
}

impl<F: Future<Output = Result<()>> + Send + 'static> Step for FutureAction_Impl<F> {
    fn step(&self, cx: &mut Context<'_>) {
        if let Some(outcome) = poll_slot(&self.fut, cx) {
            self.deliver(outcome, false);
        }
    }
    fn cancel(&self) {
        if self.fut.lock().unwrap().take().is_some() {
            self.deliver(Err(Error::from_hresult(super::asyncop::E_CANCELLED)), true);
        }
    }
}

impl<F: Future<Output = Result<()>> + Send + 'static> IAsyncInfo_Impl for FutureAction_Impl<F> {
    fn Id(&self) -> Result<u32> {
        Ok(1)
    }
    fn Status(&self) -> Result<AsyncStatus> {
        Ok(self.state.lock().unwrap().status())
    }
    fn ErrorCode(&self) -> Result<HRESULT> {
        Ok(self.state.lock().unwrap().error_code())
    }
    fn Cancel(&self) -> Result<()> {
        Step::cancel(self);
        Ok(())
    }
    fn Close(&self) -> Result<()> {
        Ok(())
    }
}

impl<F: Future<Output = Result<()>> + Send + 'static> IAsyncAction_Impl for FutureAction_Impl<F> {
    fn SetCompleted(&self, handler: Ref<'_, AsyncActionCompletedHandler>) -> Result<()> {
        let fire = self.state.lock().unwrap().assign(handler.cloned())?;
        if let Some((handler, status)) = fire {
            let sender: IAsyncAction = self.to_interface();
            let _ = handler.Invoke(&sender, status);
        }
        Ok(())
    }
    fn Completed(&self) -> Result<AsyncActionCompletedHandler> {
        self.state.lock().unwrap().completed.clone().ok_or_else(Error::empty)
    }
    fn GetResults(&self) -> Result<()> {
        self.state.lock().unwrap().results()
    }
}

/// Runs `fut` on the pool; the action completes when it does.
pub(crate) fn spawn_action<F>(fut: F) -> IAsyncAction
where
    F: Future<Output = Result<()>> + Send + 'static,
{
    let object = ComObject::new(FutureAction {
        state: Mutex::new(State::new()),
        fut: Mutex::new(Some(Box::pin(fut))),
        _module: ModuleRef::new(),
    });
    let iface = object.to_interface();
    Arc::new(Driver(object)).wake();
    iface
}
