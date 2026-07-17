//! `OverlappedDuplexStream`: a COM object implementing WinRT
//! `IInputStream`/`IOutputStream`/`IClosable` over a Win32 `HANDLE`/`SOCKET`
//! registered with [`super::tpio::IoReg`]. `ReadAsync`/`WriteAsync` start
//! overlapped IO via [`super::asyncop::IoAsyncOp`] and return the same WinRT
//! async-operation types `StreamSocket` returns, so transports built on this
//! type are drop-in compatible with `windows-future`'s awaiting and with
//! `DataReader`/`DataWriter`.

use super::asyncop::IoAsyncOp;
use super::buffer::buffer_ptr;
use super::tpio::{IoReg, Issue};
use std::sync::Arc;
use windows::Foundation::IClosable_Impl;
use windows::Storage::Streams::{
    IBuffer, IInputStream, IInputStream_Impl, IOutputStream, IOutputStream_Impl,
    InputStreamOptions,
};
use windows::Win32::Foundation::ERROR_IO_PENDING;
use windows::Win32::Networking::WinSock::{WSARecv, WSASend, WSAGetLastError, WSA_IO_PENDING, SOCKET, WSABUF};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows_core::{implement, ComObject, Ref, Result};
use windows_future::{IAsyncOperation, IAsyncOperationWithProgress};

/// Classifies a WinSock overlapped call's return the way `src/tp/io/net.rs`
/// does: `r == 0` or `WSA_IO_PENDING` both mean "a completion is coming".
fn classify_wsa(r: i32) -> Issue {
    if r == 0 {
        Issue::Pending
    } else if unsafe { WSAGetLastError() } == WSA_IO_PENDING {
        Issue::Pending
    } else {
        let err = unsafe { WSAGetLastError() };
        Issue::Failed(windows::Win32::Foundation::WIN32_ERROR(err.0 as u32).to_hresult())
    }
}

/// A duplex WinRT stream over one overlapped-registered handle/socket.
#[implement(IInputStream, IOutputStream, windows::Foundation::IClosable)]
pub struct OverlappedDuplexStream {
    reg: Arc<IoReg>,
}

/// Wraps an already-registered handle/socket as a WinRT stream object.
/// Callers `.cast()` the returned `IInspectable` to `IInputStream` and/or
/// `IOutputStream` as needed.
pub(crate) fn from_reg(reg: Arc<IoReg>) -> windows_core::IInspectable {
    ComObject::new(OverlappedDuplexStream { reg }).to_interface()
}

impl IClosable_Impl for OverlappedDuplexStream_Impl {
    fn Close(&self) -> Result<()> {
        // Dropping the last `Arc<IoReg>` (when this COM object itself is
        // released) closes the underlying handle/socket; nothing else to do.
        Ok(())
    }
}

impl IInputStream_Impl for OverlappedDuplexStream_Impl {
    fn ReadAsync(
        &self,
        buffer: Ref<IBuffer>,
        count: u32,
        _options: InputStreamOptions,
    ) -> Result<IAsyncOperationWithProgress<IBuffer, u32>> {
        let ib = buffer.ok()?.clone();
        let ptr = buffer_ptr(&ib)?;
        let n = count.min(ib.Capacity()?);
        let reg = self.reg.clone();
        let is_socket = reg.is_socket();
        let handle = reg.handle;
        let sock = SOCKET(handle.0 as usize);
        Ok(IoAsyncOp::start_read(&reg, ib, move |ovl| unsafe {
            if is_socket {
                let mut flags = 0u32;
                let wsabuf = WSABUF {
                    len: n,
                    buf: windows_core::PSTR(ptr),
                };
                let r = WSARecv(sock, &[wsabuf], None, &mut flags, Some(ovl), None);
                classify_wsa(r)
            } else {
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
            }
        }))
    }
}

impl IOutputStream_Impl for OverlappedDuplexStream_Impl {
    fn WriteAsync(&self, buffer: Ref<IBuffer>) -> Result<IAsyncOperationWithProgress<u32, u32>> {
        let ib = buffer.ok()?.clone();
        let ptr = buffer_ptr(&ib)?;
        let len = ib.Length()?;
        let reg = self.reg.clone();
        let is_socket = reg.is_socket();
        let handle = reg.handle;
        let sock = SOCKET(handle.0 as usize);
        Ok(IoAsyncOp::start_write(&reg, len, move |ovl| unsafe {
            if is_socket {
                let wsabuf = WSABUF {
                    len,
                    buf: windows_core::PSTR(ptr),
                };
                let r = WSASend(sock, &[wsabuf], None, 0, Some(ovl), None);
                classify_wsa(r)
            } else {
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
            }
        }))
    }

    fn FlushAsync(&self) -> Result<IAsyncOperation<bool>> {
        Ok(IAsyncOperation::<bool>::spawn(|| Ok(true)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::buffer::{buffer_ptr, new_buffer};
    use crate::comio::tpio::IoReg;
    use crate::tp::block_on;
    use std::future::IntoFuture;
    use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ,
        FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_WAIT};
    use windows_core::Interface;

    #[test]
    fn stream_read_write_roundtrip() {
        let name = format!(r"\\.\pipe\comio_stream_{}", std::process::id());
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
        let s = from_reg(IoReg::register(server, false).unwrap());
        let c = from_reg(IoReg::register(client, false).unwrap());
        let s_out: IOutputStream = s.cast().unwrap();
        let c_in: IInputStream = c.cast().unwrap();

        let wbuf = new_buffer(3).unwrap();
        unsafe {
            buffer_ptr(&wbuf.clone().into())
                .unwrap()
                .copy_from_nonoverlapping(b"abc".as_ptr(), 3);
        }
        wbuf.SetLength(3).unwrap();
        let wrote = block_on(
            s_out
                .WriteAsync(&wbuf.cast::<IBuffer>().unwrap())
                .unwrap()
                .into_future(),
        )
        .unwrap();
        assert_eq!(wrote, 3);

        let rbuf = new_buffer(3).unwrap();
        let got: IBuffer = block_on(
            c_in.ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), 3, InputStreamOptions(0))
                .unwrap()
                .into_future(),
        )
        .unwrap();
        assert_eq!(got.Length().unwrap(), 3);
        unsafe {
            assert_eq!(std::slice::from_raw_parts(buffer_ptr(&got).unwrap(), 3), b"abc");
        }
    }
}
