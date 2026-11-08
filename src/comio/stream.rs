//! `OverlappedDuplexStream`: a COM object implementing WinRT
//! `IInputStream`/`IOutputStream`/`IClosable` over a Win32 `HANDLE`/`SOCKET`
//! registered with `super::tpio::IoReg`. `ReadAsync`/`WriteAsync` start
//! overlapped IO via `super::asyncop::IoAsyncOp` and return the same WinRT
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

/// Classifies a WinSock overlapped call's return: `r == 0` is a synchronous
/// success, `WSA_IO_PENDING` means the op is in flight. Whether either one
/// yields a completion packet is `submit`'s call, not this one's.
fn classify_wsa(r: i32) -> Issue {
    if r == 0 {
        return Issue::SyncSuccess;
    }
    let err = unsafe { WSAGetLastError() };
    if err == WSA_IO_PENDING {
        Issue::Pending
    } else {
        Issue::Failed(windows::Win32::Foundation::WIN32_ERROR(err.0 as u32).to_hresult())
    }
}

/// Holds a pre-created accept socket while `AcceptEx` is in flight, closing
/// it if the accept is abandoned before the op resolves.
///
/// Sound as a plain frame-local guard, unlike the address buffer beside it:
/// closing a socket with a pending overlapped op aborts the op and releases
/// the handle; there is no kernel-owned *memory* behind the socket value that
/// a late completion could still write into.
pub(crate) struct AcceptSockGuard(pub(crate) SOCKET);

impl Drop for AcceptSockGuard {
    fn drop(&mut self) {
        if self.0 != windows::Win32::Networking::WinSock::INVALID_SOCKET {
            unsafe { windows::Win32::Networking::WinSock::closesocket(self.0) };
        }
    }
}

// SOCKET is a plain kernel handle value.
unsafe impl Send for AcceptSockGuard {}

/// A duplex WinRT stream over one overlapped-registered handle/socket.
#[implement(IInputStream, IOutputStream, windows::Foundation::IClosable)]
pub struct OverlappedDuplexStream {
    reg: Arc<IoReg>,
    closed: std::sync::atomic::AtomicBool,
}

/// Wraps an already-registered handle/socket as a WinRT stream object.
/// Callers `.cast()` the returned `IInspectable` to `IInputStream` and/or
/// `IOutputStream` as needed.
pub(crate) fn from_reg(reg: Arc<IoReg>) -> windows_core::IInspectable {
    ComObject::new(OverlappedDuplexStream {
        reg,
        closed: std::sync::atomic::AtomicBool::new(false),
    })
    .to_interface()
}

impl IClosable_Impl for OverlappedDuplexStream_Impl {
    /// Releases this stream's own reference to the underlying registration;
    /// it does NOT force-cancel in-flight reads/writes. Each in-flight op
    /// (`IoAsyncOp`, via `tpio::submit`) holds its own `Arc<IoReg>`, so the
    /// handle/socket stays open — and any op already issued keeps running —
    /// until every such op completes or is individually cancelled, even
    /// after `Close` returns and even if this was the caller's last
    /// reference to the stream object itself. To cancel a specific pending
    /// read/write, call `IAsyncInfo::Cancel` on the `IAsyncOperationWithProgress`
    /// that `ReadAsync`/`WriteAsync` returned for it.
    fn Close(&self) -> Result<()> {
        // Dropping the last `Arc<IoReg>` (when this COM object itself is
        // released) closes the underlying handle/socket; nothing else to do
        // beyond refusing new ops (see the field docs / module docs above).
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
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
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(windows_core::Error::from_hresult(
                crate::comio::listener::RO_E_CLOSED,
            ));
        }
        let ib = buffer.ok()?.clone();
        let ptr = buffer_ptr(&ib)?;
        let n = count.min(ib.Capacity()?);
        let reg = self.reg.clone();
        let is_socket = reg.is_socket();
        let handle = reg.handle;
        let sock = SOCKET(handle.0 as usize);
        if is_socket {
            // `WSARecv`'s `lpFlags` out-param is written by the provider at
            // completion time for a pending overlapped recv — which may be
            // well after this synchronous `issue` closure (and this whole
            // function) has returned. A plain stack local here would be
            // freed long before that deferred write, a write-after-free.
            // Box it and hand it to `start_read` as the `keep_alive` value,
            // which retains it in the same completion-gated `on_complete`
            // closure that already retains the destination buffer via
            // `out_agile` (see `IoAsyncOp::start_read`), so it lives until
            // the op's real completion regardless of cancellation.
            let mut flags = Box::new(0u32);
            let flags_ptr: *mut u32 = &mut *flags;
            Ok(IoAsyncOp::start_read(&reg, ib, flags, move |ovl| unsafe {
                let wsabuf = WSABUF {
                    len: n,
                    buf: windows_core::PSTR(ptr),
                };
                let r = WSARecv(sock, &[wsabuf], None, flags_ptr, Some(ovl), None);
                classify_wsa(r)
            }))
        } else {
            Ok(IoAsyncOp::start_read(&reg, ib, (), move |ovl| unsafe {
                match ReadFile(
                    handle,
                    Some(std::slice::from_raw_parts_mut(ptr, n as usize)),
                    None,
                    Some(ovl),
                ) {
                    Ok(()) => Issue::SyncSuccess,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            }))
        }
    }
}

impl IOutputStream_Impl for OverlappedDuplexStream_Impl {
    fn WriteAsync(&self, buffer: Ref<IBuffer>) -> Result<IAsyncOperationWithProgress<u32, u32>> {
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(windows_core::Error::from_hresult(
                crate::comio::listener::RO_E_CLOSED,
            ));
        }
        let ib = buffer.ok()?.clone();
        let ptr = buffer_ptr(&ib)?;
        let len = ib.Length()?;
        let reg = self.reg.clone();
        let is_socket = reg.is_socket();
        let handle = reg.handle;
        let sock = SOCKET(handle.0 as usize);
        Ok(IoAsyncOp::start_write(&reg, ib, len, move |ovl| unsafe {
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
                    Ok(()) => Issue::SyncSuccess,
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
    use crate::common::block_on;
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
        let wib: IBuffer = wbuf.clone().into();
        unsafe {
            buffer_ptr(&wib)
                .unwrap()
                .copy_from_nonoverlapping(b"abc".as_ptr(), 3);
        }
        wbuf.SetLength(3).unwrap();
        // `WriteAsync` clones `wib` internally and (per the fix under test)
        // `IoAsyncOp::<u32>::start_write` retains that clone via an
        // `AgileReference` for the life of the op. Get the future, then drop
        // *both* of the caller's own bindings to the write buffer before
        // awaiting, so the only thing keeping its backing memory alive
        // during the still-pending overlapped write is the op itself — not
        // incidental caller keep-alive.
        let write_future = s_out.WriteAsync(&wib).unwrap().into_future();
        drop(wib);
        drop(wbuf);
        let wrote = block_on(write_future).unwrap();
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

    /// Creates a connected overlapped named-pipe pair and wraps each end as
    /// a WinRT stream `IInspectable`, mirroring the setup in
    /// `stream_read_write_roundtrip`. Returns `(server, client)`.
    pub(crate) fn pipe_pair() -> (windows_core::IInspectable, windows_core::IInspectable) {
        let name = format!(
            r"\\.\pipe\comio_stream_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
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
        (s, c)
    }

    #[test]
    fn read_write_after_close_returns_ro_e_closed() {
        use crate::comio::listener::RO_E_CLOSED;
        use windows::Foundation::IClosable;

        let (a, _b) = pipe_pair(); // a: IInspectable stream over the server end
        let closable: IClosable = a.cast().unwrap();
        closable.Close().unwrap();
        let inp: IInputStream = a.cast().unwrap();
        let outp: IOutputStream = a.cast().unwrap();
        let rb = new_buffer(4).unwrap();
        let rerr = inp
            .ReadAsync(&rb.cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0))
            .unwrap_err();
        assert_eq!(rerr.code(), RO_E_CLOSED);
        let werr = outp.WriteAsync(&rb.cast::<IBuffer>().unwrap()).unwrap_err();
        assert_eq!(werr.code(), RO_E_CLOSED);
    }
}
