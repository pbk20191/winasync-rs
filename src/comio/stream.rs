//! `TransportStream`: a COM object implementing WinRT
//! `IInputStream`/`IOutputStream`/`IClosable` over a Win32 `HANDLE`/`SOCKET`
//! registered with `super::tpio::IoReg`. `ReadAsync`/`WriteAsync` start
//! overlapped IO via `super::asyncop::IoAsyncOp` and return the same WinRT
//! async-operation types `StreamSocket` returns, so transports built on this
//! type are drop-in compatible with `windows-future`'s awaiting and with
//! `DataReader`/`DataWriter`.

use crate::comio::unload::ModuleRef;
use super::asyncop::IoAsyncOp;
use super::buffer::buffer_ptr;
use super::bindings;
use super::bindings::Pbk20191::WinAsync::Comio::{
    IMessageStream_Impl, ITransportPeer_Impl, ITransportStream_Impl,
};
use crate::common::security;
use super::tpio::{IoReg, Issue};
use std::sync::Arc;
use windows::Foundation::IClosable_Impl;
use windows::Storage::Streams::{
    IBuffer, IInputStream, IInputStream_Impl, IOutputStream, IOutputStream_Impl,
    InputStreamOptions,
};
use windows::Win32::Foundation::{ERROR_IO_PENDING, ERROR_MORE_DATA};
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
///
/// `Pbk20191.WinAsync.Comio.TransportStream` in metadata; the generated class type
/// leads the list so `GetRuntimeClassName` reports it, and its
/// `Interface::Vtable` *is* the class's default interface, `ITransportStream`.
///
/// `IInputStream` has to be listed explicitly now. It used to be the default
/// interface, so the class entry provided its chain for free; once
/// `ITransportStream` took that place, `QueryInterface` for `IInputStream`
/// started failing — an interface the metadata still promised. The lesson is
/// that `requires` in metadata does not create a vtable chain: every
/// interface an object answers has to appear in this list.
#[implement(
    bindings::Pbk20191::WinAsync::Comio::TransportStream,
    IInputStream,
    IOutputStream,
    windows::Foundation::IClosable,
    bindings::Pbk20191::WinAsync::Comio::ITransportPeer,
    bindings::Pbk20191::WinAsync::Comio::IMessageStream
)]
pub struct TransportStream {
    reg: Arc<IoReg>,
    closed: std::sync::atomic::AtomicBool,
    peer: Peer,
    /// A message-mode named pipe, which `IMessageStream` reads by message.
    message_mode: bool,
    _module: ModuleRef,
}

/// What kind of handle a stream wraps, which decides how `ITransportPeer`
/// asks who is at the other end.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Peer {
    /// The server end of a named pipe: asks for the client.
    PipeServer,
    /// The client end of a named pipe: asks for the server.
    PipeClient,
    /// Either end of an AF_UNIX connection.
    Unix,
    /// Nothing to ask (Hyper-V): 0 and an empty SID.
    Opaque,
}

/// Wraps an already-registered handle/socket as a WinRT stream object.
///
/// Returns `ITransportStream`, which is what the metadata says a connect or
/// an accept produces -- the interface rather than this class, so that a
/// composed stream (a TLS wrapper, say) travels the same path. A caller that
/// wants `IInputStream`/`IOutputStream` casts from it as before.
pub(crate) fn from_reg(
    reg: Arc<IoReg>,
    peer: Peer,
) -> bindings::Pbk20191::WinAsync::Comio::ITransportStream {
    from_reg_with_mode(reg, peer, false)
}

/// [`from_reg`], for a named pipe that may be in message mode.
pub(crate) fn from_reg_with_mode(
    reg: Arc<IoReg>,
    peer: Peer,
    message_mode: bool,
) -> bindings::Pbk20191::WinAsync::Comio::ITransportStream {
    let object = ComObject::new(TransportStream {
        reg,
        closed: std::sync::atomic::AtomicBool::new(false),
        peer,
        message_mode,
        _module: ModuleRef::new(),
    });
    // A `QueryInterface` that cannot fail: `ITransportStream` is this class's
    // default interface, so it is the vtable the object already hands out.
    // `#[implement]` only generates the infallible conversion for types named
    // in its list, and naming the interface there as well would buy a second,
    // identical vtable chain to avoid one QI per accepted connection.
    windows_core::Interface::cast(
        &object.to_interface::<bindings::Pbk20191::WinAsync::Comio::TransportStream>(),
    )
    .expect("TransportStream does not answer its own default interface")
}

// `ITransportStream` adds no members -- it names the combination of the three
// below, so that a composed stream can be substituted anywhere ours is used.
// The generated trait requires exactly those three, which this type already
// implements, so there is nothing to write.
impl ITransportStream_Impl for TransportStream_Impl {}

// Asked of the live handle each time, so it answers for as long as the
// connection exists -- including after `Close`, which only refuses new I/O.
// A peer the transport cannot name answers 0 and an empty string, as in
// `IConnectionReceivedArgs`; the transport's error is not surfaced, because a
// script asking "who is this" wants an answer it can compare, not an
// exception for the ordinary case of a process it may not open.
impl ITransportPeer_Impl for TransportStream_Impl {
    fn RemoteProcessId(&self) -> Result<u32> {
        let handle = self.reg.handle;
        Ok(match self.peer {
            Peer::PipeServer => security::pipe_peer_pid(handle, true),
            Peer::PipeClient => security::pipe_peer_pid(handle, false),
            Peer::Unix => security::unix_peer_pid(SOCKET(handle.0 as usize)),
            Peer::Opaque => Ok(0),
        }
        .unwrap_or(0))
    }

    fn RemoteUserSid(&self) -> Result<windows_core::HSTRING> {
        let sid = match self.peer {
            Peer::PipeServer => security::pipe_client_sid(self.reg.handle),
            Peer::PipeClient | Peer::Unix => match self.RemoteProcessId()? {
                0 => Ok(String::new()),
                pid => security::process_user_sid(pid),
            },
            Peer::Opaque => Ok(String::new()),
        };
        Ok(windows_core::HSTRING::from(sid.unwrap_or_default()))
    }
}

impl IMessageStream_Impl for TransportStream_Impl {
    fn IsMessageMode(&self) -> Result<bool> {
        Ok(self.message_mode)
    }

    fn ReadMessageAsync(&self, max_size: u32) -> Result<IAsyncOperationWithProgress<IBuffer, u32>> {
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(windows_core::Error::from_hresult(
                crate::comio::listener::RO_E_CLOSED,
            ));
        }
        if !self.message_mode {
            return Err(windows_core::Error::new(
                crate::comio::listener::E_ILLEGAL_METHOD_CALL,
                "not a message-mode pipe",
            ));
        }
        Ok(IoAsyncOp::start_read_message(&self.reg, max_size))
    }

    fn TransactAsync(
        &self,
        request: Ref<IBuffer>,
        max_size: u32,
    ) -> Result<IAsyncOperationWithProgress<IBuffer, u32>> {
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(windows_core::Error::from_hresult(
                crate::comio::listener::RO_E_CLOSED,
            ));
        }
        if !self.message_mode {
            return Err(windows_core::Error::new(
                crate::comio::listener::E_ILLEGAL_METHOD_CALL,
                "not a message-mode pipe",
            ));
        }
        let request = request.ok()?;
        let ptr = buffer_ptr(request)?;
        let len = request.Length()? as usize;
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
        Ok(IoAsyncOp::start_transact(&self.reg, bytes, max_size))
    }
}

impl IClosable_Impl for TransportStream_Impl {
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

impl IInputStream_Impl for TransportStream_Impl {
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
                    // Part of a message: a warning whose packet still comes.
                    Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            }))
        }
    }
}

impl IOutputStream_Impl for TransportStream_Impl {
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
        Ok(super::settled::op_done(true))
    }
}

#[cfg(test)]
pub(crate) mod tests {
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
        let s = from_reg(IoReg::register(server, false).unwrap(), Peer::Opaque);
        let c = from_reg(IoReg::register(client, false).unwrap(), Peer::Opaque);
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

    /// A read cancelled while pending ends `Canceled`, not `Error`, and its
    /// results are `ERROR_CANCELLED` -- the `IAsyncInfo` contract, which these
    /// operations broke until `asyncop::settle`.
    #[test]
    fn a_cancelled_read_reports_canceled() {
        use windows_future::{AsyncStatus, IAsyncInfo};

        let name = format!(r"\\.\pipe\comio_cancel_{}", std::process::id());
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
        assert!(!server.is_invalid());
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
        // Both ends stay open, so nothing but the cancel can end the read.
        let _s = from_reg(IoReg::register(server, false).unwrap(), Peer::Opaque);
        let c = from_reg(IoReg::register(client, false).unwrap(), Peer::Opaque);

        let read = c
            .cast::<IInputStream>()
            .unwrap()
            .ReadAsync(&new_buffer(8).unwrap().cast::<IBuffer>().unwrap(), 8, InputStreamOptions(0))
            .unwrap();
        let info: IAsyncInfo = read.cast().unwrap();
        assert_eq!(info.Status().unwrap(), AsyncStatus::Started);
        info.Cancel().unwrap();
        let err = block_on(read.into_future()).unwrap_err();
        assert_eq!(err.code(), crate::comio::asyncop::E_CANCELLED);
        assert_eq!(info.Status().unwrap(), AsyncStatus::Canceled);
    }

    /// Every interface the metadata promises, answered.
    ///
    /// `requires` in metadata does not create a vtable chain, so a class can
    /// declare an interface and refuse it at `QueryInterface` while compiling
    /// perfectly. That is how `IInputStream` broke when `ITransportStream`
    /// took its place as the default interface: it stopped being in any chain
    /// and nothing said so until a consumer cast for it.
    #[test]
    fn a_stream_answers_everything_its_metadata_claims() {
        use windows::Foundation::IClosable;
        use windows::Storage::Streams::{IInputStream, IOutputStream};
        use windows_core::Interface;

        let (server, client) = tests::pipe_pair();
        for (what, stream) in [("server", &server), ("client", &client)] {
            assert!(stream.cast::<IInputStream>().is_ok(), "{what}: IInputStream");
            assert!(stream.cast::<IOutputStream>().is_ok(), "{what}: IOutputStream");
            assert!(stream.cast::<IClosable>().is_ok(), "{what}: IClosable");
            // And the class it actually is, reachable from the interface.
            assert!(
                stream
                    .cast::<bindings::Pbk20191::WinAsync::Comio::TransportStream>()
                    .is_ok(),
                "{what}: TransportStream"
            );
        }
    }

    /// Creates a connected overlapped named-pipe pair and wraps each end as
    /// a `Pbk20191.WinAsync.Comio.TransportStream`, mirroring the setup in
    /// `stream_read_write_roundtrip`. Returns `(server, client)`.
    pub(crate) fn pipe_pair() -> (
        bindings::Pbk20191::WinAsync::Comio::ITransportStream,
        bindings::Pbk20191::WinAsync::Comio::ITransportStream,
    ) {
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
        let s = from_reg(IoReg::register(server, false).unwrap(), Peer::Opaque);
        let c = from_reg(IoReg::register(client, false).unwrap(), Peer::Opaque);
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
