//! Poll-based `AsyncRead`/`AsyncWrite` stream abstraction over the owned-buffer
//! overlapped IO types, so generic code can compose over them.
//!
//! # Why an adapter, not a bare trait impl
//! The underlying socket/file ops take *ownership* of the buffer (it must
//! outlive the overlapped operation, which can still be in flight after the
//! future that started it is dropped — cancellation is asynchronous). The
//! `AsyncRead`/`AsyncWrite` traits instead hand out *borrowed* slices per poll.
//! Bridging the two safely requires the adapter to own an internal staging
//! buffer and the in-flight operation: the kernel only ever writes into the
//! adapter-owned buffer, never the caller's slice, so dropping a combinator
//! future mid-op can never leave the kernel writing into freed/foreign memory.
//! Data is copied between the staging buffer and the caller's slice at
//! completion.
//!
//! The adapter is *unbuffered* on the write side (at most one send in flight;
//! `poll_write` resolves when that send completes) and single-op on the read
//! side (one receive in flight; a short caller buffer keeps the unread
//! remainder for the next `poll_read`). This preserves the
//! one-poll-to-one-syscall cancellation story of the owned-buffer API.
//!
//! # One adapter, every stream
//! [`StreamIo`] is generic over [`RawStream`]: what the adapter needs of a
//! stream is one nameable, unboxed future for a read and one for a write of a
//! `Vec<u8>`, a second handle onto the same endpoint for [`split`], and a way
//! to end the write side. TCP, AF_UNIX and Hyper-V sockets, named pipes and
//! serial ports implement it; each gets an `into_io`. The trait is sealed:
//! its futures are the crate's own op futures.
//!
//! # Full duplex
//! [`StreamIo`] is a [`ReadHalf`] + [`WriteHalf`] pair sharing the endpoint
//! (via the refcounted `IoRes`). [`StreamIo::split`] hands the two halves out
//! so a reader and a writer can run concurrently (each owns its own in-flight
//! op and staging; a concurrent read and write on one handle is supported) —
//! the combinator methods need `&mut self`, so without splitting you cannot
//! read and write the same adapter at once.
//!
//! [`split`]: StreamIo::split

use super::net::TcpStream;
use crate::common::buffer::BufResult;
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

// ---------------------------------------------------------------------------
// What a stream gives the adapter
// ---------------------------------------------------------------------------

mod sealed {
    pub trait Sealed {}
}

/// A stream the adapter can wrap: see the [module docs](self#one-adapter-every-stream).
///
/// Sealed; implemented by the crate's own stream types. The methods are the
/// adapter's, not an API -- read and write through the type's own `read` /
/// `write`, or through [`StreamIo`].
pub trait RawStream: sealed::Sealed + Send + Sized + Unpin + 'static {
    /// One receive into `buf`'s spare room, as the nameable future it is;
    /// the buffer comes back with its length marking what arrived.
    #[doc(hidden)]
    fn start_read(&self, buf: Vec<u8>) -> StagedRead;
    /// One send of `buf`, nameable.
    #[doc(hidden)]
    fn start_write(&self, buf: Vec<u8>) -> StagedWrite;
    /// A second handle onto the same endpoint, which closes only once every
    /// handle is gone.
    #[doc(hidden)]
    fn share(&self) -> Self;
    /// Ends the write side so the peer reads EOF; a no-op where the transport
    /// has no half-close (then only dropping the last handle ends it).
    #[doc(hidden)]
    fn shutdown_send(&self) -> windows_core::Result<()>;
}

type OpFuture<P> = crate::common::opfuture::OpFuture<P>;

/// The receive in flight, by the shape of the kernel call under it. One enum
/// rather than an associated type, so a stream's op future (crate-private)
/// stays out of the public trait.
enum ReadOp {
    /// `WSARecv`: the flags word rides along in the payload.
    Recv(OpFuture<super::net::RecvPayload<Vec<u8>>>),
    /// `ReadFile`.
    File(OpFuture<Vec<u8>>),
    /// `ReadFile` on a pipe that may carry messages: a partial message is
    /// bytes like any other, and the "more follows" bit is dropped.
    Message(crate::common::opfuture::MessageOp<Vec<u8>>),
}

/// A staging read in flight. The adapter's; see [`RawStream`].
#[doc(hidden)]
pub struct StagedRead(ReadOp);

impl StagedRead {
    pub(in crate::tp::io) fn recv(op: OpFuture<super::net::RecvPayload<Vec<u8>>>) -> Self {
        StagedRead(ReadOp::Recv(op))
    }
    pub(in crate::tp::io) fn file(op: OpFuture<Vec<u8>>) -> Self {
        StagedRead(ReadOp::File(op))
    }
    pub(in crate::tp::io) fn message(op: crate::common::opfuture::MessageOp<Vec<u8>>) -> Self {
        StagedRead(ReadOp::Message(op))
    }
}

impl Future for StagedRead {
    type Output = BufResult<usize, Vec<u8>>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let (result, buf) = match &mut self.get_mut().0 {
            ReadOp::Recv(op) => {
                let (result, payload) = std::task::ready!(Pin::new(op).poll(cx));
                (result, payload.buf)
            }
            ReadOp::File(op) => std::task::ready!(Pin::new(op).poll(cx)),
            ReadOp::Message(op) => {
                let (result, buf) = std::task::ready!(Pin::new(op).poll(cx));
                (result.map(|(n, _more)| n), buf)
            }
        };
        let n = result.as_ref().ok().copied();
        Poll::Ready((result, crate::common::buffer::mark_read(n, buf)))
    }
}

/// A staging write in flight. The adapter's; see [`RawStream`].
#[doc(hidden)]
pub struct StagedWrite(OpFuture<Vec<u8>>);

impl StagedWrite {
    pub(in crate::tp::io) fn new(op: OpFuture<Vec<u8>>) -> Self {
        StagedWrite(op)
    }
}

impl Future for StagedWrite {
    type Output = BufResult<usize, Vec<u8>>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.get_mut().0).poll(cx)
    }
}

// ---------------------------------------------------------------------------
// The adapter
// ---------------------------------------------------------------------------

const DEFAULT_READ_CAP: usize = 16 * 1024;

/// The readable half of a split [`StreamIo`] (also usable standalone).
pub struct ReadHalf<S: RawStream = TcpStream> {
    stream: S,
    read_cap: usize,
    /// In-flight receive (owns the staging buffer the kernel writes into).
    read_fut: Option<StagedRead>,
    /// A recycled read staging buffer (len == `read_cap`) awaiting reuse.
    read_spare: Option<Vec<u8>>,
    /// Bytes read but not yet handed to a caller: `(buffer, start, end)`.
    read_rem: Option<(Vec<u8>, usize, usize)>,
}

impl<S: RawStream> ReadHalf<S> {
    fn new(stream: S, read_cap: usize) -> Self {
        Self {
            stream,
            read_cap: read_cap.max(1),
            read_fut: None,
            read_spare: None,
            read_rem: None,
        }
    }

    /// Borrows the underlying stream (e.g. for `peer_addr`).
    pub fn get_ref(&self) -> &S {
        &self.stream
    }
}

impl<S: RawStream> AsyncRead for ReadHalf<S> {
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
            Poll::Ready((res, staging)) => {
                this.read_fut = None;
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

/// The writable half of a split [`StreamIo`] (also usable standalone).
pub struct WriteHalf<S: RawStream = TcpStream> {
    stream: S,
    /// In-flight send (owns the copied bytes the kernel reads from).
    write_fut: Option<StagedWrite>,
    /// A recycled write staging buffer awaiting reuse.
    write_spare: Option<Vec<u8>>,
}

impl<S: RawStream> WriteHalf<S> {
    fn new(stream: S) -> Self {
        Self {
            stream,
            write_fut: None,
            write_spare: None,
        }
    }

    /// Borrows the underlying stream (e.g. for `peer_addr`).
    pub fn get_ref(&self) -> &S {
        &self.stream
    }
}

impl<S: RawStream> AsyncWrite for WriteHalf<S> {
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

/// An [`AsyncRead`] + [`AsyncWrite`] adapter around a stream, composed of a
/// [`ReadHalf`] and a [`WriteHalf`]. Obtain one with the stream's `into_io`
/// ([`TcpStream::into_io`] and the like), and [`split`](Self::split) it for
/// concurrent full-duplex use.
pub struct StreamIo<S: RawStream = TcpStream> {
    read: ReadHalf<S>,
    write: WriteHalf<S>,
}

/// [`StreamIo`] over a [`TcpStream`].
pub type TcpStreamIo = StreamIo<TcpStream>;

impl<S: RawStream> StreamIo<S> {
    /// Wraps `stream` with a default read staging capacity.
    pub fn new(stream: S) -> Self {
        Self::with_capacity(stream, DEFAULT_READ_CAP)
    }

    /// Wraps `stream` with an explicit per-read staging capacity (the largest
    /// amount a single underlying read will request).
    pub fn with_capacity(stream: S, read_cap: usize) -> Self {
        let write = WriteHalf::new(stream.share());
        let read = ReadHalf::new(stream, read_cap);
        Self { read, write }
    }

    /// Borrows the underlying stream (e.g. for `peer_addr`).
    pub fn get_ref(&self) -> &S {
        self.read.get_ref()
    }

    /// Recovers a handle to the stream. Any in-flight staging is dropped (which
    /// cancels an in-flight op, as with dropping an owned-buffer future).
    pub fn into_inner(self) -> S {
        self.read.stream
    }

    /// Splits into independently-ownable read and write halves for concurrent
    /// full-duplex use.
    pub fn split(self) -> (ReadHalf<S>, WriteHalf<S>) {
        (self.read, self.write)
    }
}

impl<S: RawStream> AsyncRead for StreamIo<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<windows_core::Result<usize>> {
        Pin::new(&mut self.get_mut().read).poll_read(cx, buf)
    }
}

impl<S: RawStream> AsyncWrite for StreamIo<S> {
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

// ---------------------------------------------------------------------------
// The streams
// ---------------------------------------------------------------------------

impl sealed::Sealed for TcpStream {}
impl RawStream for TcpStream {
    fn start_read(&self, buf: Vec<u8>) -> StagedRead {
        StagedRead::recv(TcpStream::start_read(self, buf))
    }
    fn start_write(&self, buf: Vec<u8>) -> StagedWrite {
        StagedWrite::new(TcpStream::start_write(self, buf))
    }
    fn share(&self) -> Self {
        TcpStream::share(self)
    }
    fn shutdown_send(&self) -> windows_core::Result<()> {
        TcpStream::shutdown_send(self)
    }
}

impl TcpStream {
    /// Wraps this stream in an [`AsyncRead`]/[`AsyncWrite`] adapter.
    pub fn into_io(self) -> TcpStreamIo {
        StreamIo::new(self)
    }
}

impl sealed::Sealed for super::unix::UnixStream {}
impl RawStream for super::unix::UnixStream {
    fn start_read(&self, buf: Vec<u8>) -> StagedRead {
        StagedRead::recv(super::unix::UnixStream::start_read(self, buf))
    }
    fn start_write(&self, buf: Vec<u8>) -> StagedWrite {
        StagedWrite::new(super::unix::UnixStream::start_write(self, buf))
    }
    fn share(&self) -> Self {
        super::unix::UnixStream::share(self)
    }
    fn shutdown_send(&self) -> windows_core::Result<()> {
        super::unix::UnixStream::shutdown_send(self)
    }
}

impl super::unix::UnixStream {
    /// Wraps this stream in an [`AsyncRead`]/[`AsyncWrite`] adapter.
    pub fn into_io(self) -> StreamIo<Self> {
        StreamIo::new(self)
    }
}

impl sealed::Sealed for super::hvsock::HvStream {}
impl RawStream for super::hvsock::HvStream {
    fn start_read(&self, buf: Vec<u8>) -> StagedRead {
        StagedRead::recv(super::hvsock::HvStream::start_read(self, buf))
    }
    fn start_write(&self, buf: Vec<u8>) -> StagedWrite {
        StagedWrite::new(super::hvsock::HvStream::start_write(self, buf))
    }
    fn share(&self) -> Self {
        super::hvsock::HvStream::share(self)
    }
    fn shutdown_send(&self) -> windows_core::Result<()> {
        super::hvsock::HvStream::shutdown_send(self)
    }
}

impl super::hvsock::HvStream {
    /// Wraps this stream in an [`AsyncRead`]/[`AsyncWrite`] adapter.
    pub fn into_io(self) -> StreamIo<Self> {
        StreamIo::new(self)
    }
}

impl sealed::Sealed for super::pipe::NamedPipeStream {}
impl RawStream for super::pipe::NamedPipeStream {
    fn start_read(&self, buf: Vec<u8>) -> StagedRead {
        StagedRead::message(super::pipe::NamedPipeStream::start_read(self, buf))
    }
    fn start_write(&self, buf: Vec<u8>) -> StagedWrite {
        StagedWrite::new(super::pipe::NamedPipeStream::start_write(self, buf))
    }
    fn share(&self) -> Self {
        super::pipe::NamedPipeStream::share(self)
    }
    /// A pipe has no half-close: the peer reads EOF once the last handle is
    /// closed, so `close` only flushes.
    fn shutdown_send(&self) -> windows_core::Result<()> {
        Ok(())
    }
}

impl super::pipe::NamedPipeStream {
    /// Wraps this stream in an [`AsyncRead`]/[`AsyncWrite`] adapter.
    ///
    /// On a message-mode pipe the adapter reads bytes without boundaries, as
    /// [`read`](Self::read) does. A pipe has no half-close, so the adapter's
    /// `close` only flushes; the peer reads EOF once the stream (every half of
    /// it) is dropped.
    pub fn into_io(self) -> StreamIo<Self> {
        StreamIo::new(self)
    }
}

impl sealed::Sealed for super::serial::SerialPort {}
impl RawStream for super::serial::SerialPort {
    fn start_read(&self, buf: Vec<u8>) -> StagedRead {
        StagedRead::file(super::serial::SerialPort::start_read(self, buf))
    }
    fn start_write(&self, buf: Vec<u8>) -> StagedWrite {
        StagedWrite::new(super::serial::SerialPort::start_write(self, buf))
    }
    fn share(&self) -> Self {
        super::serial::SerialPort::share(self)
    }
    /// A line has no close to send; `close` only flushes.
    fn shutdown_send(&self) -> windows_core::Result<()> {
        Ok(())
    }
}

impl super::serial::SerialPort {
    /// Wraps this port in an [`AsyncRead`]/[`AsyncWrite`] adapter. Each
    /// `poll_read` is one read as the port's timeouts decide it
    /// ([`set_timeouts`](Self::set_timeouts)); there is nothing to close, so
    /// the adapter's `close` only flushes.
    pub fn into_io(self) -> StreamIo<Self> {
        StreamIo::new(self)
    }
}

// ---------------------------------------------------------------------------
// Combinators
// ---------------------------------------------------------------------------

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
        ($t:ident) => {
            impl<S: RawStream> futures_io::AsyncRead for $t<S> {
                fn poll_read(
                    self: Pin<&mut Self>,
                    cx: &mut Context<'_>,
                    buf: &mut [u8],
                ) -> Poll<io::Result<usize>> {
                    map(<$t<S> as AsyncRead>::poll_read(self, cx, buf))
                }
            }
        };
    }

    macro_rules! impl_futures_write {
        ($t:ident) => {
            impl<S: RawStream> futures_io::AsyncWrite for $t<S> {
                fn poll_write(
                    self: Pin<&mut Self>,
                    cx: &mut Context<'_>,
                    buf: &[u8],
                ) -> Poll<io::Result<usize>> {
                    map(<$t<S> as AsyncWrite>::poll_write(self, cx, buf))
                }
                fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                    map(<$t<S> as AsyncWrite>::poll_flush(self, cx))
                }
                fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                    map(<$t<S> as AsyncWrite>::poll_close(self, cx))
                }
            }
        };
    }

    impl_futures_read!(StreamIo);
    impl_futures_write!(StreamIo);
    impl_futures_read!(ReadHalf);
    impl_futures_write!(WriteHalf);
}

#[cfg(test)]
#[path = "async_rw_tests.rs"]
mod tests;
