//! Poll-based `AsyncRead`/`AsyncWrite` stream abstraction over the owned-buffer
//! overlapped IO types, so generic code can compose over them.
//!
//! # Why an adapter, not a bare trait impl
//! The underlying socket/file ops take *ownership* of the buffer (it must
//! outlive the overlapped operation, which can still be in flight after the
//! future that started it is dropped — cancellation is asynchronous). The
//! `AsyncRead`/`AsyncWrite` traits instead hand out *borrowed* slices per poll.
//! Bridging the two safely requires the adapter to own an internal staging
//! buffer and the in-flight `OpFuture` (boxed): the kernel
//! only ever writes into the adapter-owned buffer, never the caller's slice, so
//! dropping a combinator future mid-op can never leave the kernel writing into
//! freed/foreign memory. Data is copied between the staging buffer and the
//! caller's slice at completion.
//!
//! The adapter is *unbuffered* on the write side (at most one WSASend in flight;
//! `poll_write` resolves when that send completes) and single-op on the read
//! side (one WSARecv in flight; a short caller buffer keeps the unread remainder
//! for the next `poll_read`). This preserves the one-poll-to-one-syscall
//! cancellation story of the owned-buffer API.
//!
//! # Full duplex
//! [`TcpStreamIo`] is a [`ReadHalf`] + [`WriteHalf`] pair sharing the socket
//! (via the refcounted `IoRes`). [`TcpStreamIo::split`] hands the two halves out
//! so a reader and a writer can run concurrently (each owns its own in-flight
//! op and staging; concurrent WSARecv + WSASend on one socket is supported) —
//! the combinator methods need `&mut self`, so without splitting you cannot read
//! and write the same adapter at once.

use super::net::TcpStream;
use std::pin::Pin;
use std::task::{Context, Poll};
use windows::Win32::Foundation::WIN32_ERROR;

/// `ERROR_HANDLE_EOF` as an `Error`, used by `read_exact` when the stream ends
/// before the buffer is filled.
fn eof_error() -> windows_core::Error {
    windows_core::Error::from_hresult(WIN32_ERROR(38).to_hresult())
}

/// `ERROR_WRITE_FAULT` as an `Error`, used by `write_all` if an underlying write
/// makes no progress (reports 0 bytes on a non-empty buffer).
fn write_zero_error() -> windows_core::Error {
    windows_core::Error::from_hresult(WIN32_ERROR(29).to_hresult())
}

/// Poll out a `Poll<Result<T>>`, returning early on `Pending` / `Err`.
macro_rules! ready_ok {
    ($e:expr) => {
        match $e {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Ok(v)) => v,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
        }
    };
}

/// A readable byte stream driven by `poll`.
pub trait AsyncRead {
    /// Reads into `buf`, resolving to the byte count (0 = end of stream).
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<windows_core::Result<usize>>;
}

/// A writable byte stream driven by `poll`.
pub trait AsyncWrite {
    /// Writes from `buf`, resolving to the byte count accepted.
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<windows_core::Result<usize>>;

    /// Drives any in-flight write to completion.
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<windows_core::Result<()>>;

    /// Flushes, then shuts the write half down (the peer's next read sees EOF).
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<windows_core::Result<()>>;
}

/// The receive in flight: the stream's own future, which is `Unpin` and
/// nameable, so nothing is boxed to keep it in a field.
type ReadFut = crate::common::opfuture::OpFuture<super::net::RecvPayload<Vec<u8>>>;
type WriteFut = crate::common::opfuture::OpFuture<Vec<u8>>;

const DEFAULT_READ_CAP: usize = 16 * 1024;

/// The readable half of a split [`TcpStreamIo`] (also usable standalone).
pub struct ReadHalf {
    stream: TcpStream,
    read_cap: usize,
    /// In-flight WSARecv (owns the staging buffer the kernel writes into).
    read_fut: Option<ReadFut>,
    /// A recycled read staging buffer (len == `read_cap`) awaiting reuse.
    read_spare: Option<Vec<u8>>,
    /// Bytes read but not yet handed to a caller: `(buffer, start, end)`.
    read_rem: Option<(Vec<u8>, usize, usize)>,
}

impl ReadHalf {
    fn new(stream: TcpStream, read_cap: usize) -> Self {
        Self {
            stream,
            read_cap: read_cap.max(1),
            read_fut: None,
            read_spare: None,
            read_rem: None,
        }
    }

    /// Borrows the underlying stream (e.g. for `peer_addr`).
    pub fn get_ref(&self) -> &TcpStream {
        &self.stream
    }
}

impl AsyncRead for ReadHalf {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<windows_core::Result<usize>> {
        let this = self.get_mut();
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        // 1. Serve from a previous read's leftover before issuing a new one.
        if let Some((data, start, end)) = &mut this.read_rem {
            let n = buf.len().min(*end - *start);
            buf[..n].copy_from_slice(&data[*start..*start + n]);
            *start += n;
            if *start == *end {
                let spare = std::mem::take(data);
                this.read_spare = Some(spare);
                this.read_rem = None;
            }
            return Poll::Ready(Ok(n));
        }

        // 2. Ensure a read is in flight (staging buffer owned by the future).
        if this.read_fut.is_none() {
            let mut staging = this.read_spare.take().unwrap_or_else(|| vec![0u8; this.read_cap]);
            if staging.len() != this.read_cap {
                staging.resize(this.read_cap, 0);
            }
            this.read_fut = Some(this.stream.start_read(staging));
        }

        // 3. Poll it; on completion copy into the caller's slice.
        match Pin::new(this.read_fut.as_mut().unwrap()).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready((res, payload)) => {
                this.read_fut = None;
                let staging = crate::common::buffer::mark_read(res.as_ref().ok().copied(), payload.buf);
                match res {
                    Ok(n) => {
                        let m = buf.len().min(n);
                        buf[..m].copy_from_slice(&staging[..m]);
                        if m < n {
                            this.read_rem = Some((staging, m, n));
                        } else {
                            this.read_spare = Some(staging);
                        }
                        Poll::Ready(Ok(m))
                    }
                    Err(e) => {
                        this.read_spare = Some(staging);
                        Poll::Ready(Err(e))
                    }
                }
            }
        }
    }
}

/// The writable half of a split [`TcpStreamIo`] (also usable standalone).
pub struct WriteHalf {
    stream: TcpStream,
    /// In-flight WSASend (owns the copied bytes the kernel reads from).
    write_fut: Option<WriteFut>,
    /// A recycled write staging buffer awaiting reuse.
    write_spare: Option<Vec<u8>>,
}

impl WriteHalf {
    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            write_fut: None,
            write_spare: None,
        }
    }

    /// Borrows the underlying stream (e.g. for `peer_addr`).
    pub fn get_ref(&self) -> &TcpStream {
        &self.stream
    }
}

impl AsyncWrite for WriteHalf {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<windows_core::Result<usize>> {
        let this = self.get_mut();

        // Start one send of a private copy of `buf` if none is in flight. A
        // well-behaved caller re-polls with the same slice until Ready, so an
        // in-flight future always corresponds to this call's data.
        if this.write_fut.is_none() {
            let mut staging = this.write_spare.take().unwrap_or_default();
            staging.clear();
            staging.extend_from_slice(buf);
            this.write_fut = Some(this.stream.start_write(staging));
        }

        match Pin::new(this.write_fut.as_mut().unwrap()).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready((res, staging)) => {
                this.write_fut = None;
                this.write_spare = Some(staging);
                Poll::Ready(res)
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<windows_core::Result<()>> {
        let this = self.get_mut();
        let Some(fut) = this.write_fut.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        match Pin::new(fut).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready((res, staging)) => {
                this.write_fut = None;
                this.write_spare = Some(staging);
                Poll::Ready(res.map(|_| ()))
            }
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<windows_core::Result<()>> {
        ready_ok!(self.as_mut().poll_flush(cx));
        Poll::Ready(self.stream.shutdown_send())
    }
}

/// An [`AsyncRead`] + [`AsyncWrite`] adapter around a [`TcpStream`], composed of
/// a [`ReadHalf`] and a [`WriteHalf`]. Obtain one with [`TcpStream::into_io`],
/// and [`split`](Self::split) it for concurrent full-duplex use.
pub struct TcpStreamIo {
    read: ReadHalf,
    write: WriteHalf,
}

impl TcpStreamIo {
    /// Wraps `stream` with a default read staging capacity.
    pub fn new(stream: TcpStream) -> Self {
        Self::with_capacity(stream, DEFAULT_READ_CAP)
    }

    /// Wraps `stream` with an explicit per-read staging capacity (the largest
    /// amount a single underlying WSARecv will request).
    pub fn with_capacity(stream: TcpStream, read_cap: usize) -> Self {
        let write = WriteHalf::new(stream.share());
        let read = ReadHalf::new(stream, read_cap);
        Self { read, write }
    }

    /// Borrows the underlying stream (e.g. for `peer_addr`).
    pub fn get_ref(&self) -> &TcpStream {
        self.read.get_ref()
    }

    /// Recovers a handle to the stream. Any in-flight staging is dropped (which
    /// cancels an in-flight op, as with dropping an owned-buffer future).
    pub fn into_inner(self) -> TcpStream {
        self.read.stream
    }

    /// Splits into independently-ownable read and write halves for concurrent
    /// full-duplex use.
    pub fn split(self) -> (ReadHalf, WriteHalf) {
        (self.read, self.write)
    }
}

impl AsyncRead for TcpStreamIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<windows_core::Result<usize>> {
        Pin::new(&mut self.get_mut().read).poll_read(cx, buf)
    }
}

impl AsyncWrite for TcpStreamIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<windows_core::Result<usize>> {
        Pin::new(&mut self.get_mut().write).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<windows_core::Result<()>> {
        Pin::new(&mut self.get_mut().write).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<windows_core::Result<()>> {
        Pin::new(&mut self.get_mut().write).poll_close(cx)
    }
}

impl TcpStream {
    /// Wraps this stream in an [`AsyncRead`]/[`AsyncWrite`] adapter.
    pub fn into_io(self) -> TcpStreamIo {
        TcpStreamIo::new(self)
    }
}

/// Ergonomic `async` combinators over any [`AsyncRead`].
pub trait AsyncReadExt: AsyncRead + Unpin {
    /// Reads once into `buf`.
    fn read<'a>(&'a mut self, buf: &'a mut [u8]) -> Read<'a, Self> {
        Read { reader: self, buf }
    }

    /// Reads exactly `buf.len()` bytes, erroring (`ERROR_HANDLE_EOF`) if the
    /// stream ends first.
    fn read_exact<'a>(&'a mut self, buf: &'a mut [u8]) -> ReadExact<'a, Self> {
        ReadExact {
            reader: self,
            buf,
            pos: 0,
        }
    }
}

impl<R: AsyncRead + Unpin + ?Sized> AsyncReadExt for R {}

/// Ergonomic `async` combinators over any [`AsyncWrite`].
pub trait AsyncWriteExt: AsyncWrite + Unpin {
    /// Writes once from `buf`.
    fn write<'a>(&'a mut self, buf: &'a [u8]) -> Write<'a, Self> {
        Write { writer: self, buf }
    }

    /// Writes all of `buf`, re-issuing until it is fully sent.
    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> WriteAll<'a, Self> {
        WriteAll {
            writer: self,
            buf,
            pos: 0,
        }
    }

    /// Drives any in-flight write to completion.
    fn flush(&mut self) -> FlushFut<'_, Self> {
        FlushFut { writer: self }
    }

    /// Flushes and shuts the write half down.
    fn close(&mut self) -> CloseFut<'_, Self> {
        CloseFut { writer: self }
    }
}

impl<W: AsyncWrite + Unpin + ?Sized> AsyncWriteExt for W {}

/// Future of [`AsyncReadExt::read`].
pub struct Read<'a, R: ?Sized> {
    reader: &'a mut R,
    buf: &'a mut [u8],
}

impl<R: AsyncRead + Unpin + ?Sized> Future for Read<'_, R> {
    type Output = windows_core::Result<usize>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Pin::new(&mut *this.reader).poll_read(cx, this.buf)
    }
}

/// Future of [`AsyncReadExt::read_exact`].
pub struct ReadExact<'a, R: ?Sized> {
    reader: &'a mut R,
    buf: &'a mut [u8],
    pos: usize,
}

impl<R: AsyncRead + Unpin + ?Sized> Future for ReadExact<'_, R> {
    type Output = windows_core::Result<()>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        while this.pos < this.buf.len() {
            let n = ready_ok!(Pin::new(&mut *this.reader).poll_read(cx, &mut this.buf[this.pos..]));
            if n == 0 {
                return Poll::Ready(Err(eof_error()));
            }
            this.pos += n;
        }
        Poll::Ready(Ok(()))
    }
}

/// Future of [`AsyncWriteExt::write`].
pub struct Write<'a, W: ?Sized> {
    writer: &'a mut W,
    buf: &'a [u8],
}

impl<W: AsyncWrite + Unpin + ?Sized> Future for Write<'_, W> {
    type Output = windows_core::Result<usize>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Pin::new(&mut *this.writer).poll_write(cx, this.buf)
    }
}

/// Future of [`AsyncWriteExt::write_all`].
pub struct WriteAll<'a, W: ?Sized> {
    writer: &'a mut W,
    buf: &'a [u8],
    pos: usize,
}

impl<W: AsyncWrite + Unpin + ?Sized> Future for WriteAll<'_, W> {
    type Output = windows_core::Result<()>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        while this.pos < this.buf.len() {
            let n = ready_ok!(Pin::new(&mut *this.writer).poll_write(cx, &this.buf[this.pos..]));
            if n == 0 {
                return Poll::Ready(Err(write_zero_error()));
            }
            this.pos += n;
        }
        Poll::Ready(Ok(()))
    }
}

/// Future of [`AsyncWriteExt::flush`].
pub struct FlushFut<'a, W: ?Sized> {
    writer: &'a mut W,
}

impl<W: AsyncWrite + Unpin + ?Sized> Future for FlushFut<'_, W> {
    type Output = windows_core::Result<()>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut *self.get_mut().writer).poll_flush(cx)
    }
}

/// Future of [`AsyncWriteExt::close`].
pub struct CloseFut<'a, W: ?Sized> {
    writer: &'a mut W,
}

impl<W: AsyncWrite + Unpin + ?Sized> Future for CloseFut<'_, W> {
    type Output = windows_core::Result<()>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut *self.get_mut().writer).poll_close(cx)
    }
}

/// Bridges the bespoke [`AsyncRead`]/[`AsyncWrite`] traits to the `futures-io`
/// ecosystem traits (enabled by the `futures-io` feature), so these types can
/// be used with `futures`/`tokio-util` combinators. Errors are mapped from
/// `windows_core::Error` into `std::io::Error`.
#[cfg(feature = "futures-io")]
mod futures_compat {
    use super::*;
    use std::io;

    fn map<T>(p: Poll<windows_core::Result<T>>) -> Poll<io::Result<T>> {
        match p {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(v)) => Poll::Ready(Ok(v)),
            Poll::Ready(Err(e)) => Poll::Ready(Err(io::Error::other(e))),
        }
    }

    macro_rules! impl_futures_read {
        ($t:ty) => {
            impl futures_io::AsyncRead for $t {
                fn poll_read(
                    self: Pin<&mut Self>,
                    cx: &mut Context<'_>,
                    buf: &mut [u8],
                ) -> Poll<io::Result<usize>> {
                    map(<$t as AsyncRead>::poll_read(self, cx, buf))
                }
            }
        };
    }

    macro_rules! impl_futures_write {
        ($t:ty) => {
            impl futures_io::AsyncWrite for $t {
                fn poll_write(
                    self: Pin<&mut Self>,
                    cx: &mut Context<'_>,
                    buf: &[u8],
                ) -> Poll<io::Result<usize>> {
                    map(<$t as AsyncWrite>::poll_write(self, cx, buf))
                }
                fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                    map(<$t as AsyncWrite>::poll_flush(self, cx))
                }
                fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                    map(<$t as AsyncWrite>::poll_close(self, cx))
                }
            }
        };
    }

    impl_futures_read!(TcpStreamIo);
    impl_futures_write!(TcpStreamIo);
    impl_futures_read!(ReadHalf);
    impl_futures_write!(WriteHalf);
}

#[cfg(test)]
#[path = "async_rw_tests.rs"]
mod tests;
