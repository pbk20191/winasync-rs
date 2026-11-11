//! Async file IO over TP_IO: files are opened with `FILE_FLAG_OVERLAPPED`
//! and read/written positionally with owned buffers (the buffer must live in
//! the op block so cancellation can hand it to the completion callback).

use std::os::windows::io::AsHandle;
use super::{submit, IoRes, Issue, OpFuture};
use crate::common::buffer::{page_size, IoBuf, IoBufMut, Region};
use crate::tp::Runtime;
use core::ffi::c_void;
use std::sync::Arc;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_HANDLE_EOF, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
use windows::Win32::System::IO::OVERLAPPED;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, ReadFileScatter, WriteFile, WriteFileGather, CREATE_ALWAYS,
    FILE_CREATION_DISPOSITION, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_NO_BUFFERING,
    FILE_FLAG_OVERLAPPED, FILE_SEGMENT_ELEMENT, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};


/// An overlapped file handle bound to a [`Runtime`]'s pool.
pub struct File {
    res: Arc<IoRes>,
}

impl File {
    fn open_impl(
        rt: &Runtime,
        path: &str,
        access: u32,
        disposition: FILE_CREATION_DISPOSITION,
        extra_flags: FILE_FLAGS_AND_ATTRIBUTES,
    ) -> windows_core::Result<File> {
        let handle = unsafe {
            CreateFileW(
                &windows_core::HSTRING::from(path),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                disposition,
                FILE_FLAG_OVERLAPPED | extra_flags,
                None,
            )?
        };
        let res = crate::tp::io::register_on(&rt.inner, unsafe { crate::common::overlapped::IoHandle::from_handle(handle) })?;
        Ok(File { res })
    }

    /// Opens an existing file for reading and writing.
    pub fn open(rt: &Runtime, path: &str) -> windows_core::Result<File> {
        Self::open_impl(
            rt,
            path,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
        )
    }

    /// Creates (or truncates) a file for reading and writing.
    pub fn create(rt: &Runtime, path: &str) -> windows_core::Result<File> {
        Self::open_impl(
            rt,
            path,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            CREATE_ALWAYS,
            FILE_FLAGS_AND_ATTRIBUTES(0),
        )
    }

    /// Creates (or truncates) a file opened with `FILE_FLAG_NO_BUFFERING`, as
    /// required by [`read_scatter`](Self::read_scatter) /
    /// [`write_gather`](Self::write_gather). With no buffering, all offsets and
    /// transfer sizes must be sector-aligned — the page-aligned, page-sized
    /// [`PageBuffers`] and whole-page offsets satisfy this.
    pub fn create_unbuffered(rt: &Runtime, path: &str) -> windows_core::Result<File> {
        Self::open_impl(
            rt,
            path,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            CREATE_ALWAYS,
            FILE_FLAG_NO_BUFFERING,
        )
    }

    /// Opens an existing file with `FILE_FLAG_NO_BUFFERING` for scatter/gather
    /// IO (see [`create_unbuffered`](Self::create_unbuffered)).
    pub fn open_unbuffered(rt: &Runtime, path: &str) -> windows_core::Result<File> {
        Self::open_impl(
            rt,
            path,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            OPEN_EXISTING,
            FILE_FLAG_NO_BUFFERING,
        )
    }

    /// Skips the completion packet whenever an op on this handle completes
    /// **synchronously**, finishing it on the issuing thread with no pool
    /// hand-off and no context switch.
    ///
    /// Worth little on most file workloads: measured on NTFS, even a re-read
    /// of just-written (cached) data comes back `ERROR_IO_PENDING`, so there
    /// is nothing to skip. It costs nothing to enable and the benefit depends
    /// entirely on whether the file system answers synchronously — profile
    /// before assuming it helps. The socket transports are where this mode
    /// actually pays.
    ///
    /// Enable it before issuing ops. Issue closures report a synchronous
    /// success as a fact and the core applies this mode, so there is no
    /// per-op contract to get wrong; semantics are unchanged, only sooner.
    pub fn set_skip_completion_on_success(&self) -> windows_core::Result<()> {
        self.res.set_skip_completion_on_success()
    }

    /// The raw file handle (for `TcpStream::transmit_file`). The returned handle
    /// is only valid while this `File` (or a shared [`res_arc`](Self::res_arc)
    /// clone) is alive.
    pub(crate) fn handle(&self) -> windows::Win32::Foundation::HANDLE {
        self.res.handle
    }

    /// A clone of the refcounted IO resource, used to keep the file handle alive
    /// for the duration of a `TransmitFile` op issued on a *socket*.
    pub(crate) fn res_arc(&self) -> Arc<IoRes> {
        self.res.clone()
    }

    fn start_read<B: IoBufMut>(&self, buf: B, pos: u64) -> OpFuture<B> {
        let handle = self.res.handle;
        unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                (*ovl).Anonymous.Anonymous.Offset = pos as u32;
                (*ovl).Anonymous.Anonymous.OffsetHigh = (pos >> 32) as u32;
                let slice =
                    core::slice::from_raw_parts_mut(buf.stable_mut_ptr(), IoBuf::bytes_total(buf));
                match ReadFile(handle, Some(slice), None, Some(ovl)) {
                    Ok(()) => Issue::SyncSuccess,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        }
    }

    /// Reads into `buf` at absolute offset `pos`. Resolves to the number of
    /// bytes read (0 at end of file) and hands the buffer back.
    pub fn read_at<B: IoBufMut + 'static>(
        &self,
        buf: B,
        pos: u64,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let fut = self.start_read(buf, pos);
        async move {
            let (result, buf) = fut.await;
            match result {
                Err(e) if e.code() == ERROR_HANDLE_EOF.to_hresult() => (Ok(0), buf),
                other => {
                    let n = other.as_ref().ok().copied();
                    (other, crate::common::buffer::mark_read(n, buf))
                }
            }
        }
    }

    /// [`read_at`](Self::read_at), with a buffer that need not be `'static`:
    /// a borrowed `&mut [u8]` included. See
    /// [borrowed buffers](crate::common::buffer::IoBuf#borrowed-buffers).
    ///
    /// The operation completes through an event rather than the pool's
    /// completion port, and dropping the future before it finishes cancels
    /// it and waits for that event -- so the borrow cannot end while the
    /// kernel still uses the buffer, and the wait needs no pool thread.
    ///
    /// # Safety
    /// The future must be dropped or run to completion -- never forgotten or
    /// leaked: not `mem::forget`, not a reference cycle, not `Box::leak`, not
    /// an executor that frees tasks without dropping them. A future that
    /// disappears without its `Drop` running leaves the kernel writing into
    /// memory the borrow has already given back.
    /// `event` is a manual-reset event, unsignaled and used by nothing
    /// else until the future is done or dropped; it is left signaled. Owned
    /// (an `OwnedHandle`) it moves into the future; borrowed, the future
    /// cannot outlive it. See [`io_event`](crate::tp::io_event).
    pub unsafe fn read_at_unchecked<B: IoBufMut, E: AsHandle + Send>(
        &self,
        buf: B,
        pos: u64,
        event: E,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B, E> {
        let handle = self.res.handle.0 as usize;
        let fut = super::evented(&self.res, buf, event, move |mem, buf| unsafe {
            mem.ovl.Anonymous.Anonymous.Offset = pos as u32;
            mem.ovl.Anonymous.Anonymous.OffsetHigh = (pos >> 32) as u32;
            let slice =
                core::slice::from_raw_parts_mut(buf.stable_mut_ptr(), IoBuf::bytes_total(buf));
            super::classify_file(ReadFile(
                windows::Win32::Foundation::HANDLE(handle as *mut core::ffi::c_void),
                Some(slice),
                None,
                Some(&mut mem.ovl),
            ))
        });
        async move {
            let (result, buf) = fut.await;
            let result = match result {
                Err(e) if e.code() == windows::Win32::Foundation::ERROR_HANDLE_EOF.to_hresult() => Ok(0),
                other => other,
            };
            let n = result.as_ref().ok().copied();
            (result, crate::common::buffer::mark_read(n, buf))
        }
    }

    /// Writes `buf` at absolute offset `pos`. Resolves to the number of bytes
    /// written and hands the buffer back.
    pub fn write_at<B: IoBuf + 'static>(
        &self,
        buf: B,
        pos: u64,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let handle = self.res.handle;
        unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                (*ovl).Anonymous.Anonymous.Offset = pos as u32;
                (*ovl).Anonymous.Anonymous.OffsetHigh = (pos >> 32) as u32;
                let slice = core::slice::from_raw_parts(buf.stable_ptr(), IoBuf::len(buf));
                match WriteFile(handle, Some(slice), None, Some(ovl)) {
                    Ok(()) => Issue::SyncSuccess,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        }
    }

    /// [`write_at`](Self::write_at), with a buffer that need not be `'static`:
    /// a borrowed `&[u8]` included. See
    /// [borrowed buffers](crate::common::buffer::IoBuf#borrowed-buffers).
    ///
    /// The operation completes through an event rather than the pool's
    /// completion port, and dropping the future before it finishes cancels
    /// it and waits for that event -- so the borrow cannot end while the
    /// kernel still uses the buffer, and the wait needs no pool thread.
    ///
    /// # Safety
    /// The future must be dropped or run to completion -- never forgotten or
    /// leaked: not `mem::forget`, not a reference cycle, not `Box::leak`, not
    /// an executor that frees tasks without dropping them. A future that
    /// disappears without its `Drop` running leaves the kernel reading from
    /// memory the borrow has already given back.
    /// `event` is a manual-reset event, unsignaled and used by nothing
    /// else until the future is done or dropped; it is left signaled. Owned
    /// (an `OwnedHandle`) it moves into the future; borrowed, the future
    /// cannot outlive it. See [`io_event`](crate::tp::io_event).
    pub unsafe fn write_at_unchecked<B: IoBuf, E: AsHandle + Send>(
        &self,
        buf: B,
        pos: u64,
        event: E,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B, E> {
        let handle = self.res.handle.0 as usize;
        super::evented(&self.res, buf, event, move |mem, buf| unsafe {
            mem.ovl.Anonymous.Anonymous.Offset = pos as u32;
            mem.ovl.Anonymous.Anonymous.OffsetHigh = (pos >> 32) as u32;
            let slice = core::slice::from_raw_parts(buf.stable_ptr(), IoBuf::len(buf));
            super::classify_file(WriteFile(
                windows::Win32::Foundation::HANDLE(handle as *mut core::ffi::c_void),
                Some(slice),
                None,
                Some(&mut mem.ovl),
            ))
        })
    }

    /// Scatter-reads `buffers.len()` bytes at absolute offset `pos` into the
    /// page-aligned `buffers` via `ReadFileScatter` (one overlapped op filling
    /// every page). `pos` and the length are whole pages. Resolves to the number
    /// of bytes read (0 at end of file) and hands the buffers back.
    ///
    /// The file must be opened unbuffered (see
    /// [`open_unbuffered`](Self::open_unbuffered)); a buffered handle fails these
    /// with `ERROR_INVALID_PARAMETER`.
    pub fn read_scatter(
        &self,
        buffers: PageBuffers,
        pos: u64,
    ) -> impl Future<Output = (windows_core::Result<usize>, PageBuffers)> + Send + use<> {
        let handle = self.res.handle;
        let count = buffers.len() as u32;
        let cap = buffers.pages() + 1;
        let fut = unsafe {
            submit(
                &self.res,
                VectorPayload {
                    buffers,
                    segments: Vec::with_capacity(cap),
                },
                move |ovl, p| {
                    (*ovl).Anonymous.Anonymous.Offset = pos as u32;
                    (*ovl).Anonymous.Anonymous.OffsetHigh = (pos >> 32) as u32;
                    p.build_segments();
                    match ReadFileScatter(handle, p.segments.as_ptr(), count, None, ovl) {
                        Ok(()) => Issue::SyncSuccess,
                        Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                        Err(e) => Issue::Failed(e.code()),
                    }
                },
            )
        };
        async move {
            let (result, payload) = fut.await;
            let r = match result {
                Err(e) if e.code() == ERROR_HANDLE_EOF.to_hresult() => Ok(0),
                other => other,
            };
            (r, payload.buffers)
        }
    }

    /// Gather-writes `buffers.len()` bytes at absolute offset `pos` from the
    /// page-aligned `buffers` via `WriteFileGather` (one overlapped op draining
    /// every page). `pos` and the length are whole pages. Resolves to the number
    /// of bytes written and hands the buffers back.
    pub fn write_gather(
        &self,
        buffers: PageBuffers,
        pos: u64,
    ) -> impl Future<Output = (windows_core::Result<usize>, PageBuffers)> + Send + use<> {
        let handle = self.res.handle;
        let count = buffers.len() as u32;
        let cap = buffers.pages() + 1;
        let fut = unsafe {
            submit(
                &self.res,
                VectorPayload {
                    buffers,
                    segments: Vec::with_capacity(cap),
                },
                move |ovl, p| {
                    (*ovl).Anonymous.Anonymous.Offset = pos as u32;
                    (*ovl).Anonymous.Anonymous.OffsetHigh = (pos >> 32) as u32;
                    p.build_segments();
                    match WriteFileGather(handle, p.segments.as_ptr(), count, None, ovl) {
                        Ok(()) => Issue::SyncSuccess,
                        Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                        Err(e) => Issue::Failed(e.code()),
                    }
                },
            )
        };
        async move {
            let (result, payload) = fut.await;
            (result, payload.buffers)
        }
    }
}

// ---------------------------------------------------------------------------
// Byte-range locks (LockFileEx / UnlockFileEx)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;

mod lock;
mod page_buffers;
pub use lock::*;
pub use page_buffers::*;
