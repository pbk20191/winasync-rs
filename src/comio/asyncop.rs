//! Custom `IAsyncOperationWithProgress<T, u32>` whose result is produced by a
//! TP_IO overlapped completion (see [`super::tpio`]) rather than by a Rust
//! closure run on the thread pool (as in `windows-future`'s `spawn`).
//!
//! The design mirrors `windows-future`'s `async_spawn.rs`: a `Mutex`-guarded
//! state holding the pending `result`, the `Completed` handler, and a flag
//! tracking whether the handler has been assigned. The differences are:
//!
//! * the progress type `P` is fixed to `u32`;
//! * the result is filled by a `super::tpio::submit` completion callback instead of
//!   a synchronous closure; and
//! * the `OpHandle` returned by `submit` is stored in the state so
//!   `IAsyncInfo::Cancel` can cancel the in-flight IO.

use crate::comio::unload::ModuleRef;
use super::check;
use super::tpio::{submit, IoReg, Issue, OpHandle};
use std::sync::{Arc, Mutex};
use windows::Storage::Streams::IBuffer;
use windows::Win32::Foundation::{ERROR_HANDLE_EOF, ERROR_IO_PENDING, ERROR_MORE_DATA};
use windows::Win32::System::IO::OVERLAPPED;
use windows_core::{
    implement, AgileReference, ComObject, Error, IUnknownImpl, Ref, Result, RuntimeType, HRESULT,
};
use windows_future::{
    AsyncOperationProgressHandler, AsyncOperationWithProgressCompletedHandler, AsyncStatus,
    IAsyncInfo_Impl, IAsyncOperationWithProgress, IAsyncOperationWithProgress_Impl,
};

/// E_ILLEGAL_METHOD_CALL — `GetResults` called before completion.
const E_ILLEGAL_METHOD_CALL: HRESULT = HRESULT(0x8000000Eu32 as i32);
/// E_ILLEGAL_DELEGATE_ASSIGNMENT — `Completed` handler set more than once.
const E_ILLEGAL_DELEGATE_ASSIGNMENT: HRESULT = HRESULT(0x80000018u32 as i32);

/// `HRESULT_FROM_WIN32(ERROR_CANCELLED)`: what a cancelled operation's
/// `GetResults` fails with, as it does for C++/WinRT and the system's own
/// operations.
pub(crate) const E_CANCELLED: HRESULT = HRESULT(0x800704C7u32 as i32);

/// Turns the I/O's own outcome into the operation's, once `Cancel` has been
/// called.
///
/// `IAsyncInfo::Cancel` is a request, and the WinRT contract for what follows
/// is specific: an operation that ends *because* it was cancelled reports
/// `AsyncStatus::Canceled`, not `Error`. The I/O cannot say that by itself --
/// `CancelIoEx` makes it complete with `ERROR_OPERATION_ABORTED`, which is
/// indistinguishable from any other failure unless someone remembers asking.
/// This is where that is remembered. An operation that finished anyway --
/// the cancel lost the race to the completion -- keeps its real outcome, as
/// the contract allows.
///
/// Until 2026-11-09 these operations reported `Error` here. Nothing in-process
/// noticed; the out-of-process test, which checks the status a remote caller
/// sees, did.
pub(crate) fn settle<T>(res: Result<T>, cancel_requested: bool) -> (Result<T>, bool) {
    const ABORTED: HRESULT = HRESULT(0x800703E3u32 as i32); // ERROR_OPERATION_ABORTED
    match res {
        Err(e) if cancel_requested && e.code() == ABORTED => {
            (Err(Error::from_hresult(E_CANCELLED)), true)
        }
        res => (res, false),
    }
}

struct OpState<T: RuntimeType + 'static> {
    result: Option<Result<T>>,
    /// `Cancel` was called while the operation was still running.
    cancel_requested: bool,
    /// It ended because of that; see [`settle`].
    canceled: bool,
    completed: Option<AsyncOperationWithProgressCompletedHandler<T, u32>>,
    completed_assigned: bool,
    op: Option<OpHandle>,
    /// Which overlapped call `op` belongs to, for an operation made of more
    /// than one (`start_read_message`). A step stores its handle only if no
    /// later step has started, so a fast completion cannot have its
    /// successor's handle overwritten by a stale one.
    step: u64,
}

impl<T: RuntimeType + 'static> OpState<T> {
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
            Some(Err(error)) => error.code(),
            _ => HRESULT(0),
        }
    }

    fn get_results(&self) -> Result<T> {
        match &self.result {
            Some(result) => result.clone(),
            None => Err(Error::from_hresult(E_ILLEGAL_METHOD_CALL)),
        }
    }
}

#[implement(IAsyncOperationWithProgress<T, u32>, windows_future::IAsyncInfo)]
pub(crate) struct IoAsyncOp<T>(Mutex<OpState<T>>, ModuleRef)
where
    T: RuntimeType + 'static;

// SAFETY: the only non-`Send` data (the pending `IBuffer`/handler WinRT
// interfaces) lives behind the `Mutex`, and every access is serialized by it.
// The object is registered agile (the `#[implement]` default), so its
// interfaces may legally be used from any thread — including the thread-pool
// completion callback that fills the result. Mirrors `windows-future`'s
// `unsafe impl Send for SyncState`.
unsafe impl<T: RuntimeType + 'static> Send for IoAsyncOp<T> {}
unsafe impl<T: RuntimeType + 'static> Sync for IoAsyncOp<T> {}

impl<T: RuntimeType + 'static> IoAsyncOp_Impl<T> {
    /// Stores the operation's outcome and fires the `Completed` handler if one
    /// has already been assigned. Called from the TP_IO completion callback.
    fn complete(&self, res: Result<T>) {
        let (status, completed) = {
            let mut guard = self.0.lock().unwrap();
            debug_assert!(guard.result.is_none());
            let (res, canceled) = settle(res, guard.cancel_requested);
            guard.result = Some(res);
            guard.canceled = canceled;
            (guard.status(), guard.completed.take())
        };
        // Invoke outside the lock so the handler (which may re-enter to read
        // Status/GetResults) can take it.
        if let Some(completed) = completed {
            let sender: IAsyncOperationWithProgress<T, u32> = self.to_interface();
            let _ = completed.Invoke(&sender, status);
        }
    }
}

impl<T: RuntimeType + 'static> IAsyncInfo_Impl for IoAsyncOp_Impl<T> {
    fn Id(&self) -> Result<u32> {
        Ok(1)
    }
    fn Status(&self) -> Result<AsyncStatus> {
        Ok(self.0.lock().unwrap().status())
    }
    fn ErrorCode(&self) -> Result<HRESULT> {
        Ok(self.0.lock().unwrap().error_code())
    }
    fn Cancel(&self) -> Result<()> {
        let mut guard = self.0.lock().unwrap();
        if guard.result.is_none() {
            guard.cancel_requested = true;
            if let Some(op) = guard.op.as_ref() {
                op.cancel();
            }
        }
        Ok(())
    }
    fn Close(&self) -> Result<()> {
        Ok(())
    }
}

impl<T: RuntimeType + 'static> IAsyncOperationWithProgress_Impl<T, u32> for IoAsyncOp_Impl<T> {
    fn SetCompleted(
        &self,
        handler: Ref<AsyncOperationWithProgressCompletedHandler<T, u32>>,
    ) -> Result<()> {
        let mut guard = self.0.lock().unwrap();
        if guard.completed_assigned {
            return Err(Error::from_hresult(E_ILLEGAL_DELEGATE_ASSIGNMENT));
        }
        guard.completed_assigned = true;
        let status = guard.status();
        let handler = handler.ok()?;

        if status == AsyncStatus::Started {
            guard.completed = Some(handler.clone());
        } else {
            // The result is already present — invoke immediately, outside the
            // lock, using this object's interface as the sender.
            drop(guard);
            let sender: IAsyncOperationWithProgress<T, u32> = self.to_interface();
            let _ = handler.Invoke(&sender, status);
        }
        Ok(())
    }
    fn Completed(&self) -> Result<AsyncOperationWithProgressCompletedHandler<T, u32>> {
        Err(Error::empty())
    }
    fn GetResults(&self) -> Result<T> {
        self.0.lock().unwrap().get_results()
    }
    fn SetProgress(&self, _: Ref<AsyncOperationProgressHandler<T, u32>>) -> Result<()> {
        Ok(())
    }
    fn Progress(&self) -> Result<AsyncOperationProgressHandler<T, u32>> {
        Err(Error::empty())
    }
}

fn new_object<T: RuntimeType + 'static>() -> ComObject<IoAsyncOp<T>> {
    ComObject::new(IoAsyncOp(
        Mutex::new(OpState {
            result: None,
            cancel_requested: false,
            canceled: false,
            completed: None,
            completed_assigned: false,
            op: None,
            step: 0,
        }),
        ModuleRef::new(),
    ))
}

impl IoAsyncOp<IBuffer> {
    /// Starts an overlapped read whose result is the (length-adjusted) `out`
    /// buffer. `ioresult == 0` (or `ERROR_MORE_DATA`, a partial message) sets
    /// the buffer length to `bytes`;
    /// `ERROR_HANDLE_EOF` sets it to 0 (clean EOF); any other error fails the
    /// operation.
    pub(crate) fn start_read(
        reg: &Arc<IoReg>,
        out: IBuffer,
        // Retained (moved, unused) inside the completion-gated `on_complete`
        // closure below purely to extend its lifetime to the operation's
        // real completion — exactly like `out_agile`. This exists for
        // callers whose `issue` closure hands the kernel a pointer into some
        // *other* piece of memory that the provider may write back through
        // only at completion time (e.g. `WSARecv`'s `lpFlags` out-param):
        // without this, that memory would have to be a plain local that gets
        // freed the instant `issue` returns, well before the deferred write
        // — a write-after-free. Pass `()` (or any trivial value) when `issue`
        // needs no such keep-alive (e.g. the `ReadFile` path).
        keep_alive: impl Send + 'static,
        // `issue` is run synchronously inside `submit` on the calling thread,
        // so — unlike the completion callback — it need not be `Send`/`'static`.
        issue: impl FnOnce(*mut OVERLAPPED) -> Issue,
    ) -> IAsyncOperationWithProgress<IBuffer, u32> {
        let object = new_object::<IBuffer>();
        let iface: IAsyncOperationWithProgress<IBuffer, u32> = object.to_interface();
        // The completion callback runs on a thread-pool thread, so it may not
        // capture the non-agile `IBuffer` directly; hold an agile reference and
        // resolve it there.
        let out_agile = check(AgileReference::new(&out));
        let cb_object = object.clone();
        let handle = unsafe {
            submit(
                reg,
                move |ioresult: u32, bytes: usize| {
                    // Force the closure to capture (and thus keep alive until
                    // the real completion fires, whether natural or
                    // cancelled) whatever `keep_alive` value the caller's
                    // `issue` closure wrote a kernel-visible pointer into.
                    let _ = &keep_alive;
                    let out = out_agile.resolve().expect("resolve agile IBuffer");
                    // `ERROR_MORE_DATA` is a message-mode pipe handing over
                    // part of a message longer than the buffer: data, not a
                    // failure. The rest comes with the next read.
                    let res = if ioresult == 0 || ioresult == ERROR_MORE_DATA.0 {
                        let _ = out.SetLength(bytes as u32);
                        Ok(out)
                    } else if ioresult == ERROR_HANDLE_EOF.0 {
                        let _ = out.SetLength(0);
                        Ok(out)
                    } else {
                        Err(Error::from_hresult(HRESULT::from_win32(ioresult)))
                    };
                    cb_object.complete(res);
                },
                issue,
            )
        };
        object.0.lock().unwrap().op = Some(handle);
        iface
    }
}

/// Largest single read `start_read_message` issues before it knows a
/// message's size; a longer one is finished with one read of exactly the rest.
const FIRST_READ: usize = 64 * 1024;

impl IoAsyncOp<IBuffer> {
    /// Reads one whole message from a message-mode pipe, of at most `max`
    /// bytes. See `IMessageStream` in the metadata for the contract.
    ///
    /// A chain of overlapped reads rather than one: the first reads up to
    /// `FIRST_READ`; if the message goes on (`ERROR_MORE_DATA`), the rest is
    /// sized with `PeekNamedPipe` and read in one more. A message that will
    /// not fit is read on to its end and dropped, so the pipe is left on a
    /// boundary. Each step runs from the previous one's completion, and
    /// `Cancel` reaches whichever is in flight.
    pub(crate) fn start_read_message(
        reg: &Arc<IoReg>,
        max: u32,
    ) -> IAsyncOperationWithProgress<IBuffer, u32> {
        let object = new_object::<IBuffer>();
        let iface: IAsyncOperationWithProgress<IBuffer, u32> = object.to_interface();
        read_message_step(
            object,
            reg.clone(),
            MessageRead {
                max: max as usize,
                message: Vec::new(),
                discarding: None,
            },
            (max as usize).min(FIRST_READ),
            None,
        );
        iface
    }

    /// Writes `request` as one message and reads the reply, as
    /// `start_read_message` would, starting with one `TransactNamedPipe`.
    ///
    /// The request is copied: the kernel reads it until the transaction
    /// completes, and an owned copy is simpler to keep alive that long than
    /// the caller's `IBuffer`.
    pub(crate) fn start_transact(
        reg: &Arc<IoReg>,
        request: Vec<u8>,
        max: u32,
    ) -> IAsyncOperationWithProgress<IBuffer, u32> {
        let object = new_object::<IBuffer>();
        let iface: IAsyncOperationWithProgress<IBuffer, u32> = object.to_interface();
        read_message_step(
            object,
            reg.clone(),
            MessageRead {
                max: max as usize,
                message: Vec::new(),
                discarding: None,
            },
            (max as usize).min(FIRST_READ),
            Some(request),
        );
        iface
    }
}

/// Where a `start_read_message` chain has got to.
struct MessageRead {
    max: usize,
    message: Vec<u8>,
    /// Set once the message is known not to fit: its full size, for the error.
    discarding: Option<usize>,
}

/// One step of a message read. With `request`, the step is the transaction
/// that sends it and reads the start of the reply; every later step is a read.
fn read_message_step(
    object: ComObject<IoAsyncOp<IBuffer>>,
    reg: Arc<IoReg>,
    mut read: MessageRead,
    chunk: usize,
    request: Option<Vec<u8>>,
) {
    let step = {
        let mut guard = object.0.lock().unwrap();
        guard.step += 1;
        guard.step
    };
    // Owned by the completion closure, which the op block keeps until the
    // kernel is done writing into it.
    let mut part = vec![0u8; chunk];
    let ptr = part.as_mut_ptr();
    let sent = request.as_ref().map(|r| (r.as_ptr(), r.len()));
    let handle = reg.handle;
    let cb_object = object.clone();
    let cb_reg = reg.clone();
    let op = unsafe {
        submit(
            &reg,
            move |ioresult: u32, bytes: usize| {
                // The kernel reads the request until here.
                let _ = &request;
                let more = ioresult == ERROR_MORE_DATA.0;
                if ioresult != 0 && !more {
                    let res = if ioresult == ERROR_HANDLE_EOF.0 {
                        to_buffer(&[])
                    } else {
                        Err(Error::from_hresult(HRESULT::from_win32(ioresult)))
                    };
                    cb_object.complete(res);
                    return;
                }
                if read.discarding.is_none() {
                    read.message.extend_from_slice(&part[..bytes]);
                }
                if !more {
                    let res = match read.discarding {
                        Some(size) => Err(Error::new(
                            HRESULT::from_win32(ERROR_MORE_DATA.0),
                            format!(
                                "a {size}-byte message does not fit in {}; it was discarded",
                                read.max
                            ),
                        )),
                        None => to_buffer(&read.message),
                    };
                    cb_object.complete(res);
                    return;
                }
                let left = match bytes_left_in_message(cb_reg.handle) {
                    Ok(left) => left,
                    Err(e) => {
                        cb_object.complete(Err(e));
                        return;
                    }
                };
                if read.discarding.is_none() && read.message.len() + left > read.max {
                    read.discarding = Some(read.message.len() + left);
                    read.message = Vec::new();
                }
                let next = match read.discarding {
                    Some(_) => left.clamp(1, FIRST_READ),
                    None => left.max(1),
                };
                read_message_step(cb_object, cb_reg, read, next, None);
            },
            move |ovl| match match sent {
                Some((request, len)) => windows::Win32::System::Pipes::TransactNamedPipe(
                    handle,
                    Some(request.cast()),
                    len as u32,
                    Some(ptr.cast()),
                    chunk as u32,
                    // Overlapped: the count comes with the completion.
                    core::ptr::null_mut(),
                    Some(ovl),
                ),
                None => windows::Win32::Storage::FileSystem::ReadFile(
                    handle,
                    Some(std::slice::from_raw_parts_mut(ptr, chunk)),
                    None,
                    Some(ovl),
                ),
            } {
                Ok(()) => Issue::SyncSuccess,
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                // A partial message is a warning, not a failure: its packet
                // is queued like any other. See `tp::io::pipe`'s `read_part`.
                Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => Issue::Pending,
                Err(e) => Issue::Failed(e.code()),
            },
        )
    };
    let mut guard = object.0.lock().unwrap();
    if guard.step == step && guard.result.is_none() {
        if guard.cancel_requested {
            op.cancel();
        }
        guard.op = Some(op);
    }
}

/// What remains of the message the last read stopped part-way through.
fn bytes_left_in_message(handle: windows::Win32::Foundation::HANDLE) -> Result<usize> {
    let mut left = 0u32;
    unsafe {
        windows::Win32::System::Pipes::PeekNamedPipe(handle, None, 0, None, None, Some(&mut left))?
    };
    Ok(left as usize)
}

/// A WinRT buffer holding `bytes`.
fn to_buffer(bytes: &[u8]) -> Result<IBuffer> {
    let buffer: IBuffer =
        windows_core::Interface::cast(&super::buffer::new_buffer(bytes.len() as u32)?)?;
    let ptr = super::buffer::buffer_ptr(&buffer)?;
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
    buffer.SetLength(bytes.len() as u32)?;
    Ok(buffer)
}

impl IoAsyncOp<u32> {
    /// Starts an overlapped write whose result is the number of bytes
    /// written. `out` is the source `IBuffer` whose backing memory the
    /// overlapped write reads from directly (via a raw pointer captured by
    /// `issue`); it is retained (via an agile reference, exactly as
    /// `start_read` retains its destination buffer) for the lifetime of the
    /// operation so the kernel never writes through a dangling pointer into
    /// freed WinRT buffer memory, even though the completion callback itself
    /// has no need to read from it.
    pub(crate) fn start_write(
        reg: &Arc<IoReg>,
        out: IBuffer,
        _count_hint: u32,
        // See `start_read`: `issue` runs synchronously and need not be `Send`.
        issue: impl FnOnce(*mut OVERLAPPED) -> Issue,
    ) -> IAsyncOperationWithProgress<u32, u32> {
        let object = new_object::<u32>();
        let iface: IAsyncOperationWithProgress<u32, u32> = object.to_interface();
        // The completion callback runs on a thread-pool thread, so it may not
        // capture the non-agile `IBuffer` directly; hold an agile reference
        // purely to keep the buffer's backing memory alive until the
        // overlapped write completes.
        let out_agile = check(AgileReference::new(&out));
        let cb_object = object.clone();
        let handle = unsafe {
            submit(
                reg,
                move |ioresult: u32, bytes: usize| {
                    // Force the closure to capture (and thus keep alive until
                    // this point) the agile reference to the source buffer.
                    let _ = &out_agile;
                    let res = if ioresult == 0 {
                        Ok(bytes as u32)
                    } else {
                        Err(Error::from_hresult(HRESULT::from_win32(ioresult)))
                    };
                    cb_object.complete(res);
                },
                issue,
            )
        };
        object.0.lock().unwrap().op = Some(handle);
        iface
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::buffer::{buffer_ptr, new_buffer};
    use crate::comio::tpio::{IoReg, Issue};
    use crate::common::block_on;
    use std::future::IntoFuture;
    use windows::Storage::Streams::IBuffer;
    use windows::Win32::Foundation::ERROR_IO_PENDING;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED,
        FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_WAIT};

    #[test]
    fn async_read_completes() {
        let name = format!(r"\\.\pipe\comio_asyncop_{}", std::process::id());
        let wname = windows_core::HSTRING::from(&name);
        let server = unsafe {
            CreateNamedPipeW(
                &wname,
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0),
                PIPE_TYPE_BYTE | PIPE_WAIT,
                1,
                4096,
                4096,
                0,
                None,
            )
        };
        assert!(!server.is_invalid(), "CreateNamedPipeW failed: {:?}", unsafe {
            windows::Win32::Foundation::GetLastError()
        });
        let client = unsafe {
            CreateFileW(
                &wname,
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        }
        .unwrap();
        let sreg = IoReg::register(unsafe { crate::comio::tpio::IoHandle::from_handle(server) }).unwrap();
        let creg = IoReg::register(unsafe { crate::comio::tpio::IoHandle::from_handle(client) }).unwrap();

        // Server: async write "hi".
        let wbuf = new_buffer(2).unwrap();
        let wib: IBuffer = wbuf.clone().into();
        unsafe {
            let p = buffer_ptr(&wib).unwrap();
            p.copy_from_nonoverlapping(b"hi".as_ptr(), 2);
        }
        wbuf.SetLength(2).unwrap();
        let wp = buffer_ptr(&wib).unwrap();
        let sh = sreg.handle;
        let write_op = IoAsyncOp::<u32>::start_write(&sreg, wib.clone(), 2, move |ovl| {
            match unsafe { WriteFile(sh, Some(std::slice::from_raw_parts(wp, 2)), None, Some(ovl)) } {
                Ok(()) => Issue::SyncSuccess,
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                Err(e) => Issue::Failed(e.code()),
            }
        });

        // Client: async read 2 bytes.
        let rbuf = new_buffer(2).unwrap();
        let rib: IBuffer = rbuf.clone().into();
        let rp = buffer_ptr(&rib).unwrap();
        let ch = creg.handle;
        let read_op = IoAsyncOp::start_read(&creg, rib.clone(), (), move |ovl| {
            match unsafe { ReadFile(ch, Some(std::slice::from_raw_parts_mut(rp, 2)), None, Some(ovl)) } {
                Ok(()) => Issue::SyncSuccess,
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                Err(e) => Issue::Failed(e.code()),
            }
        });

        let wrote = block_on(write_op.into_future()).unwrap();
        assert_eq!(wrote, 2);
        let got = block_on(read_op.into_future()).unwrap();
        assert_eq!(got.Length().unwrap(), 2);
        unsafe {
            assert_eq!(std::slice::from_raw_parts(buffer_ptr(&got).unwrap(), 2), b"hi");
        }
    }
}
