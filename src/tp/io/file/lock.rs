//! Byte-range locks: `LockFileEx` overlapped, released when the last
//! holder drops.

use super::*;

/// A held byte-range lock; dropping it unlocks the range.
///
/// Windows ties a lock to the handle that took it, so this keeps the file's
/// handle open until the range is unlocked.
pub struct FileLock {
    pub(super) _state: Arc<LockState>,
}

/// One lock request, shared by its future and its completion. Whoever lets
/// go last unlocks the range if it was granted -- which is what keeps a lock
/// that is granted just as its future is dropped from being held forever:
/// the completion still sees the grant, and nobody else is left to undo it.
pub(super) struct LockState {
    pub(super) res: Arc<IoRes>,
    pub(super) offset: u64,
    pub(super) len: u64,
    pub(super) granted: std::sync::atomic::AtomicBool,
}

impl Drop for LockState {
    fn drop(&mut self) {
        if self.granted.load(std::sync::atomic::Ordering::Acquire) {
            let _ = unlock_range(self.res.handle, self.offset, self.len);
        }
    }
}

/// `UnlockFileEx`, waited for here. The handle is bound to the runtime's
/// completion port, so the request carries an event with its low bit set:
/// that keeps its completion off the port, where the pool would receive a
/// packet for an `OVERLAPPED` it never issued.
pub(super) fn unlock_range(handle: HANDLE, offset: u64, len: u64) -> windows_core::Result<()> {
    use windows::Win32::Storage::FileSystem::UnlockFileEx;
    use windows::Win32::System::IO::GetOverlappedResult;
    use windows::Win32::System::Threading::CreateEventW;
    let event = unsafe { CreateEventW(None, true, false, None)? };
    let mut ovl = OVERLAPPED::default();
    ovl.Anonymous.Anonymous.Offset = offset as u32;
    ovl.Anonymous.Anonymous.OffsetHigh = (offset >> 32) as u32;
    ovl.hEvent = HANDLE((event.0 as usize | 1) as *mut c_void);
    let issued = unsafe { UnlockFileEx(handle, None, len as u32, (len >> 32) as u32, &mut ovl) };
    let result = match issued {
        Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
            let mut n = 0u32;
            unsafe { GetOverlappedResult(handle, &ovl, &mut n, true) }
        }
        other => other,
    };
    unsafe {
        let _ = CloseHandle(event);
    }
    result
}

impl File {
    /// Locks `len` bytes at `offset`, waiting for anyone who holds an
    /// overlapping lock; resolves to a guard that unlocks on drop.
    ///
    /// `exclusive` locks keep out every other lock on the range; shared
    /// ones admit other shared ones. Windows enforces locks against reads
    /// and writes through *other* handles. Dropping the future while it waits
    /// cancels the request.
    pub fn lock(
        &self,
        offset: u64,
        len: u64,
        exclusive: bool,
    ) -> impl Future<Output = windows_core::Result<FileLock>> + Send + use<> {
        let attempt = self.lock_with(offset, len, exclusive, false);
        async move { attempt.await.map(|lock| lock.expect("a waiting lock cannot be refused")) }
    }

    /// [`lock`](Self::lock), without waiting: `None` if an overlapping lock
    /// is held.
    pub fn try_lock(
        &self,
        offset: u64,
        len: u64,
        exclusive: bool,
    ) -> impl Future<Output = windows_core::Result<Option<FileLock>>> + Send + use<> {
        self.lock_with(offset, len, exclusive, true)
    }

    pub(super) fn lock_with(
        &self,
        offset: u64,
        len: u64,
        exclusive: bool,
        immediately: bool,
    ) -> impl Future<Output = windows_core::Result<Option<FileLock>>> + Send + use<> {
        use crate::common::overlapped::{self as ov, Completion};
        use windows::Win32::Foundation::{ERROR_LOCK_VIOLATION, ERROR_OPERATION_ABORTED, WIN32_ERROR};
        use windows::Win32::Storage::FileSystem::{
            LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LOCK_FILE_FLAGS,
        };
        let state = Arc::new(LockState {
            res: self.res.clone(),
            offset,
            len,
            granted: std::sync::atomic::AtomicBool::new(false),
        });
        let (tx, rx) = crate::common::oneshot::channel::<windows_core::Result<()>>();
        let completion_state = state.clone();
        let on_complete = move |c: Completion| {
            let result = match c {
                Completion::Done(0, _) => {
                    completion_state
                        .granted
                        .store(true, std::sync::atomic::Ordering::Release);
                    Ok(())
                }
                Completion::Done(code, _) => {
                    Err(windows_core::Error::from_hresult(WIN32_ERROR(code).to_hresult()))
                }
                Completion::SyncFail(hr) => Err(windows_core::Error::from_hresult(hr)),
            };
            let _ = tx.send(result);
            // If the future is already gone, this was the last reference,
            // and a granted lock is unlocked right here.
            drop(completion_state);
        };
        let mut flags = LOCK_FILE_FLAGS(0);
        if exclusive {
            flags |= LOCKFILE_EXCLUSIVE_LOCK;
        }
        if immediately {
            flags |= LOCKFILE_FAIL_IMMEDIATELY;
        }
        let handle = self.res.handle;
        let op = unsafe {
            ov::submit(&self.res, (), on_complete, move |ovl, _| {
                (*ovl).Anonymous.Anonymous.Offset = offset as u32;
                (*ovl).Anonymous.Anonymous.OffsetHigh = (offset >> 32) as u32;
                match LockFileEx(handle, flags, None, len as u32, (len >> 32) as u32, ovl) {
                    Ok(()) => Issue::SyncSuccess,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    // The exception to "a synchronous failure queues no
                    // packet". Measured: a fail-immediately lock refused with
                    // ERROR_LOCK_VIOLATION *also* queues a failed packet for
                    // this OVERLAPPED. Treating it as a plain failure freed
                    // the op block that packet then reached -- an access
                    // violation. So it is pending: the packet carries it.
                    Err(e) if e.code() == ERROR_LOCK_VIOLATION.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        };
        /// Cancels a lock request still waiting when its future is dropped.
        struct CancelOnDrop(ov::OpHandle<()>);
        impl Drop for CancelOnDrop {
            fn drop(&mut self) {
                self.0.cancel();
            }
        }
        let op = CancelOnDrop(op);
        async move {
            let _op = op;
            match rx.await {
                Ok(Ok(())) => Ok(Some(FileLock { _state: state })),
                Ok(Err(e)) if e.code() == ERROR_LOCK_VIOLATION.to_hresult() => Ok(None),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(windows_core::Error::from_hresult(
                    ERROR_OPERATION_ABORTED.to_hresult(),
                )),
            }
        }
    }
}
