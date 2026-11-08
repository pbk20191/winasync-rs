//! Custom `IAsyncOperationWithProgress<T, u32>` whose result is produced by a
//! TP_IO overlapped completion (see [`super::tpio`]) rather than by a Rust
//! closure run on the thread pool (as in `windows-future`'s `spawn`).
//!
//! The design mirrors `windows-future`'s `async_spawn.rs`: a `Mutex`-guarded
//! state holding the pending `result`, the `Completed` handler, and a flag
//! tracking whether the handler has been assigned. The differences are:
//!
//! * the progress type `P` is fixed to `u32`;
//! * the result is filled by a [`tpio::submit`] completion callback instead of
//!   a synchronous closure; and
//! * the [`OpHandle`] returned by `submit` is stored in the state so
//!   [`IAsyncInfo::Cancel`] can cancel the in-flight IO.

use super::check;
use super::tpio::{submit, IoReg, Issue, OpHandle};
use std::sync::{Arc, Mutex};
use windows::Storage::Streams::IBuffer;
use windows::Win32::Foundation::ERROR_HANDLE_EOF;
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

struct OpState<T: RuntimeType + 'static> {
    result: Option<Result<T>>,
    completed: Option<AsyncOperationWithProgressCompletedHandler<T, u32>>,
    completed_assigned: bool,
    op: Option<OpHandle>,
}

impl<T: RuntimeType + 'static> OpState<T> {
    fn status(&self) -> AsyncStatus {
        match &self.result {
            None => AsyncStatus::Started,
            Some(Ok(_)) => AsyncStatus::Completed,
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
pub(crate) struct IoAsyncOp<T>(Mutex<OpState<T>>)
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
            guard.result = Some(res);
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
        if let Some(op) = self.0.lock().unwrap().op.as_ref() {
            op.cancel();
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
    ComObject::new(IoAsyncOp(Mutex::new(OpState {
        result: None,
        completed: None,
        completed_assigned: false,
        op: None,
    })))
}

impl IoAsyncOp<IBuffer> {
    /// Starts an overlapped read whose result is the (length-adjusted) `out`
    /// buffer. `ioresult == 0` sets the buffer length to `bytes`;
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
                Box::new(move |ioresult: u32, bytes: usize| {
                    // Force the closure to capture (and thus keep alive until
                    // the real completion fires, whether natural or
                    // cancelled) whatever `keep_alive` value the caller's
                    // `issue` closure wrote a kernel-visible pointer into.
                    let _ = &keep_alive;
                    let out = out_agile.resolve().expect("resolve agile IBuffer");
                    let res = if ioresult == 0 {
                        let _ = out.SetLength(bytes as u32);
                        Ok(out)
                    } else if ioresult == ERROR_HANDLE_EOF.0 {
                        let _ = out.SetLength(0);
                        Ok(out)
                    } else {
                        Err(Error::from_hresult(HRESULT::from_win32(ioresult)))
                    };
                    cb_object.complete(res);
                }),
                issue,
            )
        };
        object.0.lock().unwrap().op = Some(handle);
        iface
    }
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
                Box::new(move |ioresult: u32, bytes: usize| {
                    // Force the closure to capture (and thus keep alive until
                    // this point) the agile reference to the source buffer.
                    let _ = &out_agile;
                    let res = if ioresult == 0 {
                        Ok(bytes as u32)
                    } else {
                        Err(Error::from_hresult(HRESULT::from_win32(ioresult)))
                    };
                    cb_object.complete(res);
                }),
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
        let sreg = IoReg::register(server, false).unwrap();
        let creg = IoReg::register(client, false).unwrap();

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
                Ok(()) => Issue::Pending,
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
                Ok(()) => Issue::Pending,
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
