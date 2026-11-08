//! `RandomAccessFileStream`: a COM object implementing WinRT
//! `IRandomAccessStream` (and its required `IInputStream`/`IOutputStream`/
//! `IClosable`) over an overlapped file `HANDLE` registered with
//! [`super::tpio::IoReg`].
//!
//! Unlike [`super::stream::OverlappedDuplexStream`] (sequential; for
//! pipes/sockets), this is *positional*: every `ReadAsync`/`WriteAsync` issues
//! its overlapped op at the stream's current `Position` (written into the
//! `OVERLAPPED` offset, as `FILE_FLAG_OVERLAPPED` files have no implicit file
//! pointer) and advances `Position` by the bytes actually transferred, reported
//! from the completion via `IoAsyncOp`'s `on_bytes` hook. That makes it a
//! drop-in `IRandomAccessStream` for the many WinRT Storage/Imaging/Media APIs
//! that demand one, and correct under sequential `DataReader`/`DataWriter` use.
//!
//! `GetInputStreamAt`/`GetOutputStreamAt`/`CloneStream` return new stream
//! objects that **share the same file** (via the refcounted `IoReg`) but own an
//! **independent position cursor** — each overlapped op carries its own offset,
//! so independent cursors never interfere. The file closes once the last such
//! object (and any in-flight op) releases its `Arc<IoReg>`.
//!
//! Concurrent positional ops issued on the *same* object race on the shared
//! cursor; for independent concurrent access, take a `GetInputStreamAt` /
//! `CloneStream` view per logical cursor.

use super::asyncop::IoAsyncOp;
use super::buffer::buffer_ptr;
use super::listener::RO_E_CLOSED;
use super::tpio::{IoReg, Issue};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use windows::Foundation::IClosable_Impl;
use windows::Storage::Streams::{
    IBuffer, IInputStream, IInputStream_Impl, IOutputStream, IOutputStream_Impl,
    IRandomAccessStream, IRandomAccessStream_Impl, InputStreamOptions,
};
use windows::Win32::Foundation::{CloseHandle, ERROR_IO_PENDING, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetFileSizeEx, ReadFile, SetFileInformationByHandle, WriteFile, CREATE_ALWAYS,
    FILE_END_OF_FILE_INFO, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FileEndOfFileInfo, OPEN_EXISTING,
};
use windows_core::{
    implement, ComObject, Error, HRESULT, HSTRING, Interface, Ref, Result,
};
use windows_future::{IAsyncOperation, IAsyncOperationWithProgress};

/// E_ACCESSDENIED — a read on a write-only view, or vice versa.
const E_ACCESSDENIED: HRESULT = HRESULT(0x80070005u32 as i32);

/// A positional WinRT `IRandomAccessStream` over one overlapped file handle.
///
/// The required interfaces are listed explicitly so `QueryInterface` succeeds
/// for each — `#[implement(IRandomAccessStream)]` alone does not register QI for
/// `IInputStream`/`IOutputStream`/`IClosable` (they are independent WinRT
/// interfaces, not vtable bases).
#[implement(
    IRandomAccessStream,
    IInputStream,
    IOutputStream,
    windows::Foundation::IClosable
)]
pub struct RandomAccessFileStream {
    reg: Arc<IoReg>,
    /// This view's own cursor (shared only with its own in-flight completions).
    pos: Arc<AtomicU64>,
    can_read: bool,
    can_write: bool,
    closed: AtomicBool,
}

/// Builds a new stream object over `reg` positioned at `pos`.
fn new_stream(
    reg: Arc<IoReg>,
    pos: u64,
    can_read: bool,
    can_write: bool,
) -> ComObject<RandomAccessFileStream> {
    ComObject::new(RandomAccessFileStream {
        reg,
        pos: Arc::new(AtomicU64::new(pos)),
        can_read,
        can_write,
        closed: AtomicBool::new(false),
    })
}

impl RandomAccessFileStream {
    /// Creates (or truncates) a file and returns it as an `IRandomAccessStream`
    /// opened for reading and writing.
    pub fn create(path: &str) -> Result<IRandomAccessStream> {
        Self::open_impl(path, CREATE_ALWAYS)
    }

    /// Opens an existing file as an `IRandomAccessStream` for reading/writing.
    pub fn open(path: &str) -> Result<IRandomAccessStream> {
        Self::open_impl(path, OPEN_EXISTING)
    }

    fn open_impl(
        path: &str,
        disposition: windows::Win32::Storage::FileSystem::FILE_CREATION_DISPOSITION,
    ) -> Result<IRandomAccessStream> {
        let handle = unsafe {
            CreateFileW(
                &HSTRING::from(path),
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                disposition,
                FILE_FLAG_OVERLAPPED,
                None,
            )?
        };
        match IoReg::register(handle, false) {
            Ok(reg) => Ok(new_stream(reg, 0, true, true).to_interface()),
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                Err(e)
            }
        }
    }
}

impl IClosable_Impl for RandomAccessFileStream_Impl {
    /// Refuses new ops and releases this view's `Arc<IoReg>` when the COM object
    /// is released; in-flight ops keep the handle alive until they finish (same
    /// posture as `OverlappedDuplexStream::Close`).
    fn Close(&self) -> Result<()> {
        self.closed.store(true, Ordering::Release);
        Ok(())
    }
}

impl IInputStream_Impl for RandomAccessFileStream_Impl {
    fn ReadAsync(
        &self,
        buffer: Ref<IBuffer>,
        count: u32,
        _options: InputStreamOptions,
    ) -> Result<IAsyncOperationWithProgress<IBuffer, u32>> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }
        if !self.can_read {
            return Err(Error::from_hresult(E_ACCESSDENIED));
        }
        let ib = buffer.ok()?.clone();
        let ptr = buffer_ptr(&ib)?;
        let n = count.min(ib.Capacity()?);
        let reg = self.reg.clone();
        let handle = reg.handle;
        let start = self.pos.load(Ordering::Acquire);
        let pos_cb = self.pos.clone();
        Ok(IoAsyncOp::start_read(
            &reg,
            ib,
            (),
            move |nread| {
                pos_cb.fetch_add(nread as u64, Ordering::Release);
            },
            move |ovl| unsafe {
                (*ovl).Anonymous.Anonymous.Offset = start as u32;
                (*ovl).Anonymous.Anonymous.OffsetHigh = (start >> 32) as u32;
                match ReadFile(
                    handle,
                    Some(std::slice::from_raw_parts_mut(ptr, n as usize)),
                    None,
                    Some(ovl),
                ) {
                    Ok(()) => Issue::Pending,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            },
        ))
    }
}

impl IOutputStream_Impl for RandomAccessFileStream_Impl {
    fn WriteAsync(&self, buffer: Ref<IBuffer>) -> Result<IAsyncOperationWithProgress<u32, u32>> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::from_hresult(RO_E_CLOSED));
        }
        if !self.can_write {
            return Err(Error::from_hresult(E_ACCESSDENIED));
        }
        let ib = buffer.ok()?.clone();
        let ptr = buffer_ptr(&ib)?;
        let len = ib.Length()?;
        let reg = self.reg.clone();
        let handle = reg.handle;
        let start = self.pos.load(Ordering::Acquire);
        let pos_cb = self.pos.clone();
        Ok(IoAsyncOp::start_write(
            &reg,
            ib,
            len,
            move |nwrote| {
                pos_cb.fetch_add(nwrote as u64, Ordering::Release);
            },
            move |ovl| unsafe {
                (*ovl).Anonymous.Anonymous.Offset = start as u32;
                (*ovl).Anonymous.Anonymous.OffsetHigh = (start >> 32) as u32;
                match WriteFile(
                    handle,
                    Some(std::slice::from_raw_parts(ptr, len as usize)),
                    None,
                    Some(ovl),
                ) {
                    Ok(()) => Issue::Pending,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            },
        ))
    }

    /// Overlapped writes already reach the OS cache; a WinRT `FlushAsync` here
    /// is a no-op returning `true` (matching `OverlappedDuplexStream`). Durable
    /// flush to disk would be `FlushFileBuffers`; add it if a caller needs it.
    fn FlushAsync(&self) -> Result<IAsyncOperation<bool>> {
        Ok(IAsyncOperation::<bool>::spawn(|| Ok(true)))
    }
}

impl IRandomAccessStream_Impl for RandomAccessFileStream_Impl {
    fn Size(&self) -> Result<u64> {
        let mut size: i64 = 0;
        unsafe { GetFileSizeEx(self.reg.handle, &mut size)? };
        Ok(size as u64)
    }

    fn SetSize(&self, value: u64) -> Result<()> {
        let info = FILE_END_OF_FILE_INFO {
            EndOfFile: value as i64,
        };
        unsafe {
            SetFileInformationByHandle(
                self.reg.handle,
                FileEndOfFileInfo,
                &info as *const _ as *const core::ffi::c_void,
                core::mem::size_of::<FILE_END_OF_FILE_INFO>() as u32,
            )?
        };
        Ok(())
    }

    fn GetInputStreamAt(&self, position: u64) -> Result<IInputStream> {
        let s: IRandomAccessStream =
            new_stream(self.reg.clone(), position, self.can_read, self.can_write).to_interface();
        s.cast()
    }

    fn GetOutputStreamAt(&self, position: u64) -> Result<IOutputStream> {
        let s: IRandomAccessStream =
            new_stream(self.reg.clone(), position, self.can_read, self.can_write).to_interface();
        s.cast()
    }

    fn Position(&self) -> Result<u64> {
        Ok(self.pos.load(Ordering::Acquire))
    }

    fn Seek(&self, position: u64) -> Result<()> {
        self.pos.store(position, Ordering::Release);
        Ok(())
    }

    fn CloneStream(&self) -> Result<IRandomAccessStream> {
        Ok(new_stream(
            self.reg.clone(),
            self.pos.load(Ordering::Acquire),
            self.can_read,
            self.can_write,
        )
        .to_interface())
    }

    fn CanRead(&self) -> Result<bool> {
        Ok(self.can_read)
    }

    fn CanWrite(&self) -> Result<bool> {
        Ok(self.can_write)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::buffer::{buffer_ptr, new_buffer};
    use crate::tp::block_on;
    use std::future::IntoFuture;
    use windows::Foundation::IClosable;
    use windows::Storage::Streams::{IInputStream, IOutputStream, InputStreamOptions};

    fn temp_path(tag: &str) -> String {
        let mut p = std::env::temp_dir();
        p.push(format!("comio_ras_{}_{tag}.bin", std::process::id()));
        p.to_string_lossy().into_owned()
    }

    fn write_bytes(out: &IOutputStream, data: &[u8]) -> u32 {
        let buf = new_buffer(data.len() as u32).unwrap();
        let ib: IBuffer = buf.clone().into();
        unsafe {
            buffer_ptr(&ib)
                .unwrap()
                .copy_from_nonoverlapping(data.as_ptr(), data.len());
        }
        buf.SetLength(data.len() as u32).unwrap();
        block_on(out.WriteAsync(&ib).unwrap().into_future()).unwrap()
    }

    fn read_bytes(inp: &IInputStream, n: u32) -> Vec<u8> {
        let got: IBuffer = block_on(
            inp.ReadAsync(
                &new_buffer(n).unwrap().cast::<IBuffer>().unwrap(),
                n,
                InputStreamOptions(0),
            )
            .unwrap()
            .into_future(),
        )
        .unwrap();
        let len = got.Length().unwrap() as usize;
        unsafe { std::slice::from_raw_parts(buffer_ptr(&got).unwrap(), len).to_vec() }
    }

    #[test]
    fn random_access_write_seek_read() {
        let path = temp_path("rws");
        let s = RandomAccessFileStream::create(&path).unwrap();

        // QI for the required interfaces must succeed.
        let out: IOutputStream = s.cast().unwrap();
        let inp: IInputStream = s.cast().unwrap();
        let _closable: IClosable = s.cast().unwrap();

        assert!(s.CanRead().unwrap() && s.CanWrite().unwrap());

        // Write advances Position and grows Size.
        assert_eq!(write_bytes(&out, b"abcdef"), 6);
        assert_eq!(s.Position().unwrap(), 6);
        assert_eq!(s.Size().unwrap(), 6);

        // Seek back to 0 and read it all; Position advances by bytes read.
        s.Seek(0).unwrap();
        assert_eq!(read_bytes(&inp, 6), b"abcdef");
        assert_eq!(s.Position().unwrap(), 6);

        // Independent cursor: a view at offset 3 reads "def" without moving the
        // parent's Position.
        let at3: IInputStream = s.GetInputStreamAt(3).unwrap();
        assert_eq!(read_bytes(&at3, 3), b"def");
        assert_eq!(s.Position().unwrap(), 6, "parent cursor must be independent");

        // CloneStream copies the current position but is otherwise independent.
        let clone = s.CloneStream().unwrap();
        assert_eq!(clone.Position().unwrap(), 6);
        clone.Seek(0).unwrap();
        assert_eq!(s.Position().unwrap(), 6, "clone cursor must be independent");

        // SetSize truncates.
        s.SetSize(3).unwrap();
        assert_eq!(s.Size().unwrap(), 3);

        s.cast::<IClosable>().unwrap().Close().unwrap();
        drop(s);
        let _ = std::fs::remove_file(&path);
    }
}
