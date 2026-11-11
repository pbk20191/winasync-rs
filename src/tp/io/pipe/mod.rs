//! Async named-pipe IO over TP_IO with a raw `(Result, Vec<u8>)` API (the
//! `tp`-native counterpart to comio's WinRT-shaped named pipe).
//!
//! Instance creation follows `common::pipe`: whenever we hold no instance the
//! next one claims the name, one listening instance is kept between accepts,
//! remote clients are refused unless asked for. [`NamedPipeOptions`] sets the
//! rest -- message mode, the instance limit, buffer sizes. The connector opens
//! at identification level, so a server can tell who we are and cannot act
//! as us, and matches its read mode to the pipe's.

use super::{submit, IoRes, Issue};
use crate::common::buffer::{mark_read, IoBuf, IoBufMut};
pub use crate::common::pipe::NamedPipeOptions;
use crate::common::pipe::{self as server, PipeServer, ReplenishOnExit, Slot, SlotWaiter};
use crate::common::security;
use crate::tp::Runtime;
use std::sync::Arc;
use std::time::Duration;
use windows::Win32::Foundation::{
    E_ILLEGAL_METHOD_CALL, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_OPERATION_ABORTED,
    ERROR_PIPE_CONNECTED, HANDLE,
};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::Pipes::{ConnectNamedPipe, PeekNamedPipe, TransactNamedPipe};
use windows_core::{Error, HSTRING};

/// Largest single read `read_message_vec` issues before it knows a message's
/// size; anything longer is finished with one more read of exactly the rest.
const FIRST_READ: usize = 64 * 1024;

/// A connected named-pipe endpoint (server- or client-side); duplex, sequential.
pub struct NamedPipeStream {
    res: Arc<IoRes>,
    /// Which end this is, which decides how the peer is asked for.
    server_side: bool,
    message_mode: bool,
}

impl NamedPipeStream {
    /// The process id at the other end: the client, on a stream from
    /// `accept`; the server, on one from `connect`.
    pub fn peer_process_id(&self) -> windows_core::Result<u32> {
        security::pipe_peer_pid(self.res.handle, self.server_side)
    }

    /// The account at the other end, as a string SID.
    ///
    /// On an accepted stream this is read from the client's own token, so it
    /// is the account the client connected as. On a connected stream it is
    /// the server process's account, which needs the right to open that
    /// process -- usually only true when both run as the same user, which is
    /// exactly the case worth checking: that the server we reached is ours.
    pub fn peer_user_sid(&self) -> windows_core::Result<String> {
        if self.server_side {
            security::pipe_client_sid(self.res.handle)
        } else {
            security::process_user_sid(self.peer_process_id()?)
        }
    }

    /// Whether this pipe carries messages: bound with
    /// [`NamedPipeOptions::message_mode`], or connected to a pipe that was.
    pub fn is_message_mode(&self) -> bool {
        self.message_mode
    }

    /// Reads into `buf`; resolves to the byte count and hands the buffer back.
    ///
    /// On a message-mode pipe this returns at most one message, and a message
    /// longer than `buf` arrives over several reads with nothing to say where
    /// it ends. Use [`read_message_vec`](Self::read_message_vec) to keep the
    /// boundaries.
    pub fn read<B: IoBufMut + 'static>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let part = self.read_part(buf);
        async move {
            let (result, buf) = part.await;
            let result = result.map(|(n, _more)| n);
            let n = result.as_ref().ok().copied();
            (result, crate::common::buffer::mark_read(n, buf))
        }
    }

    /// [`read`](Self::read), with a buffer that need not be `'static`:
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
    pub unsafe fn read_unchecked<B: IoBufMut>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let handle = self.res.handle.0 as usize;
        let fut = super::evented(&self.res, buf, move |mem, buf| unsafe {
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
            let n = result.as_ref().ok().copied();
            (result, crate::common::buffer::mark_read(n, buf))
        }
    }

    /// One `ReadFile`, reporting whether the message it read from goes on.
    fn read_part<B: IoBufMut>(&self, buf: B) -> super::MessageOp<B> {
        let handle = self.res.handle;
        unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                let slice =
                    core::slice::from_raw_parts_mut(buf.stable_mut_ptr(), IoBuf::bytes_total(buf));
                match ReadFile(handle, Some(slice), None, Some(ovl)) {
                    Ok(()) => Issue::SyncSuccess,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    // A partial message is a warning status, not an error:
                    // the request completed and its packet is queued like any
                    // other. (Measured: an overlapped read of a queued
                    // over-long message reports `ERROR_IO_PENDING`, and the
                    // packet carries `ERROR_MORE_DATA`. Should one ever come
                    // back synchronously, the packet still follows.)
                    Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        }
        .message()
    }

    /// Reads one whole message, of at most `max_len` bytes.
    ///
    /// A longer message is read to its end and discarded, and this fails with
    /// `ERROR_MORE_DATA`: leaving its tail in the pipe would make the next
    /// read return it as if it were a message of its own. Fails with
    /// `E_ILLEGAL_METHOD_CALL` on a byte-mode pipe, which has no messages to
    /// read.
    ///
    /// Allocates the result; [`read_message`](Self::read_message)
    /// reads into a buffer of the caller's.
    pub async fn read_message_vec(&self, max_len: usize) -> windows_core::Result<Vec<u8>> {
        self.message_mode()?;
        let (result, message) = self.read_part(Vec::with_capacity(max_len.min(FIRST_READ))).await;
        let (n, more) = result?;
        self.finish_message(mark_read(Some(n), message), more, max_len).await
    }

    /// Reads one whole message into `buf`, which must have room for it
    /// ([`IoBuf::bytes_total`]); resolves to its length and hands `buf` back.
    ///
    /// A message too long for `buf` is read to its end and discarded, as in
    /// [`read_message_vec`](Self::read_message_vec), and this fails with
    /// `ERROR_MORE_DATA` -- `buf` then holds its first part.
    pub async fn read_message<B: IoBufMut + 'static>(
        &self,
        buf: B,
    ) -> (windows_core::Result<usize>, B) {
        if let Err(e) = self.message_mode() {
            return (Err(e), buf);
        }
        let (result, buf) = self.read_part(buf).await;
        let (result, buf) = self.whole_or_discarded(result, buf).await;
        (result, buf)
    }

    /// Writes `request` as one message and reads the reply, of at most
    /// `max_response` bytes, in one `TransactNamedPipe`: the round trip of a
    /// request/response protocol in a single kernel call.
    ///
    /// For a message-mode pipe only (`E_ILLEGAL_METHOD_CALL` otherwise), and
    /// only with nothing unread in the pipe -- the kernel refuses a
    /// transaction that would read a stale message as its reply. A reply
    /// longer than `max_response` is read to its end and discarded, as in
    /// [`read_message_vec`](Self::read_message_vec).
    ///
    /// Allocates the reply; [`transact`](Self::transact) reads it
    /// into a buffer of the caller's.
    pub async fn transact_vec<I: IoBuf + 'static>(
        &self,
        request: I,
        max_response: usize,
    ) -> windows_core::Result<Vec<u8>> {
        self.message_mode()?;
        let reply = Vec::with_capacity(max_response.min(FIRST_READ));
        let (result, (_request, reply)) = self.transact_part(request, reply).await;
        let (n, more) = result?;
        self.finish_message(mark_read(Some(n), reply), more, max_response).await
    }

    /// [`transact_vec`](Self::transact_vec), with the reply read into `reply`, which
    /// must have room for it; resolves to the reply's length and hands both
    /// buffers back. A reply too long for `reply` is discarded as in
    /// [`read_message`](Self::read_message).
    pub async fn transact<I: IoBuf + 'static, O: IoBufMut + 'static>(
        &self,
        request: I,
        reply: O,
    ) -> (windows_core::Result<usize>, I, O) {
        if let Err(e) = self.message_mode() {
            return (Err(e), request, reply);
        }
        let (result, (request, reply)) = self.transact_part(request, reply).await;
        let (result, reply) = self.whole_or_discarded(result, reply).await;
        (result, request, reply)
    }

    fn message_mode(&self) -> windows_core::Result<()> {
        if self.message_mode {
            Ok(())
        } else {
            Err(Error::new(E_ILLEGAL_METHOD_CALL, "not a message-mode pipe"))
        }
    }

    /// One `TransactNamedPipe`, reporting whether the reply goes on.
    fn transact_part<I: IoBuf, O: IoBufMut>(&self, request: I, reply: O) -> super::MessageOp<(I, O)> {
        let handle = self.res.handle;
        unsafe {
            submit(&self.res, (request, reply), move |ovl, (request, reply)| {
                match TransactNamedPipe(
                    handle,
                    Some(request.stable_ptr().cast()),
                    IoBuf::len(request) as u32,
                    Some(reply.stable_mut_ptr().cast()),
                    IoBuf::bytes_total(reply) as u32,
                    // Overlapped: the count comes with the completion, and
                    // the documentation allows null here for that case.
                    core::ptr::null_mut(),
                    Some(ovl),
                ) {
                    Ok(()) => Issue::SyncSuccess,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    // A reply longer than the buffer: the same warning a read
                    // reports, and the same packet follows.
                    Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        }
        .message()
    }

    /// The tail of a read into a caller's buffer: the message fit, or it did
    /// not and the rest of it is discarded.
    async fn whole_or_discarded<B: IoBufMut>(
        &self,
        result: windows_core::Result<(usize, bool)>,
        buf: B,
    ) -> (windows_core::Result<usize>, B) {
        let (n, more) = match result {
            Ok(r) => r,
            Err(e) => return (Err(e), buf),
        };
        let buf = mark_read(Some(n), buf);
        if !more {
            return (Ok(n), buf);
        }
        let size = bytes_left_in_message(self.res.handle).map(|left| n + left);
        if let Err(e) = self.discard_rest_of_message().await {
            return (Err(e), buf);
        }
        let error = Error::new(
            ERROR_MORE_DATA.to_hresult(),
            match size {
                Ok(size) => format!("a {size}-byte message does not fit in {n}; it was discarded"),
                Err(_) => format!("a message does not fit in {n}; it was discarded"),
            },
        );
        (Err(error), buf)
    }

    /// Reads the rest of a message whose first `message.len()` bytes are in
    /// hand, if `more` says there is a rest; discards it, and fails, if the
    /// whole would exceed `max_len`.
    ///
    /// The rest is read straight into `message`'s spare capacity -- a
    /// [`Slice`](crate::common::buffer::Slice) past its end -- so nothing is
    /// copied twice.
    async fn finish_message(
        &self,
        mut message: Vec<u8>,
        mut more: bool,
        max_len: usize,
    ) -> windows_core::Result<Vec<u8>> {
        while more {
            let left = bytes_left_in_message(self.res.handle)?;
            if message.len() + left > max_len {
                self.discard_rest_of_message().await?;
                return Err(Error::new(
                    ERROR_MORE_DATA.to_hresult(),
                    format!(
                        "a {}-byte message does not fit in {max_len}; it was discarded",
                        message.len() + left
                    ),
                ));
            }
            let start = message.len();
            message.reserve_exact(left.max(1));
            let (result, rest) = self.read_part(message.slice(start..start + left.max(1))).await;
            let (n, m) = result?;
            message = mark_read(Some(n), rest).into_inner();
            more = m;
        }
        Ok(message)
    }

    async fn discard_rest_of_message(&self) -> windows_core::Result<()> {
        loop {
            let (result, _) = self.read_part(Vec::with_capacity(FIRST_READ)).await;
            if !result?.1 {
                return Ok(());
            }
        }
    }

    /// Writes `buf`; resolves to the byte count and hands the buffer back.
    ///
    /// On a message-mode pipe each write is one message.
    pub fn write<B: IoBuf + 'static>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let handle = self.res.handle;

        unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                let slice = core::slice::from_raw_parts(buf.stable_ptr(), IoBuf::len(buf));
                match WriteFile(handle, Some(slice), None, Some(ovl)) {
                    Ok(()) => Issue::SyncSuccess,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        }
    }

    /// [`write`](Self::write), with a buffer that need not be `'static`:
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
    pub unsafe fn write_unchecked<B: IoBuf>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let handle = self.res.handle.0 as usize;
        super::evented(&self.res, buf, move |mem, buf| unsafe {
            let slice = core::slice::from_raw_parts(buf.stable_ptr(), IoBuf::len(buf));
            super::classify_file(WriteFile(
                windows::Win32::Foundation::HANDLE(handle as *mut core::ffi::c_void),
                Some(slice),
                None,
                Some(&mut mem.ovl),
            ))
        })
    }
}

/// What remains of the message the last read stopped part-way through.
pub(crate) fn bytes_left_in_message(handle: HANDLE) -> windows_core::Result<usize> {
    let mut left = 0u32;
    unsafe { PeekNamedPipe(handle, None, 0, None, None, Some(&mut left))? };
    Ok(left as usize)
}

/// Server side: each `accept` waits for a client on a listening instance.
pub struct NamedPipeListener {
    rt: Runtime,
    server: PipeServer,
}

impl NamedPipeListener {
    /// Binds a listener to `name` (e.g. `\\.\pipe\foo`) with `options` --
    /// [`NamedPipeOptions::new()`] for the defaults, including the default
    /// security: this process's account, administrators and SYSTEM.
    ///
    /// No instance is created until the first `accept`, which is also where a
    /// name already held by another server is reported. Fails here with
    /// `E_INVALIDARG` for options no pipe can have: an instance limit outside
    /// 1..=255, or remote clients without a descriptor.
    pub fn bind(
        rt: &Runtime,
        name: &str,
        options: &NamedPipeOptions,
    ) -> windows_core::Result<NamedPipeListener> {
        options.validate()?;
        Ok(NamedPipeListener {
            rt: rt.clone(),
            server: PipeServer::new(name, options.clone()),
        })
    }

    /// Waits for a client on a listening instance.
    ///
    /// At the instance limit this first waits for one of this listener's
    /// instances to close.
    pub async fn accept(&self) -> windows_core::Result<NamedPipeStream> {
        let instance = loop {
            let (freed, slot) = crate::common::oneshot::channel::<bool>();
            match self.server.instance_or_wait(SlotWaiter::Signal(freed))? {
                Slot::Ready(instance) => break instance,
                // `false` (or a dropped sender) would mean the server is
                // gone, which cannot happen while `&self` is borrowed -- but
                // say so rather than spin if it ever does.
                Slot::Wait => {
                    if slot.await != Ok(true) {
                        return Err(Error::from_hresult(ERROR_OPERATION_ABORTED.to_hresult()));
                    }
                }
                Slot::Retry => crate::tp::sleep(&self.rt, server::RETRY_AFTER).await,
            }
        };
        let res = crate::tp::io::register_on(&self.rt.inner, instance)?;
        let _replenish = ReplenishOnExit(&self.server);
        let h = res.handle;
        let fut = unsafe {
            submit(&res, (), move |ovl, _| {
                // ERROR_PIPE_CONNECTED (a client connected between
                // CreateNamedPipeW and here) reports success but queues NO
                // completion, so classify it as Failed — submit then balances
                // the StartThreadpoolIo and delivers it synchronously; accept
                // treats it as "connected" below.
                match ConnectNamedPipe(h, Some(ovl)) {
                    Ok(()) => Issue::SyncSuccess,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        };
        let (result, ()) = fut.await;
        match result {
            Ok(_) => {}
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {}
            Err(e) => return Err(e),
        }
        Ok(NamedPipeStream {
            res,
            server_side: true,
            message_mode: self.server.message_mode(),
        })
    }
}

/// Client side.
pub struct NamedPipeConnector;

impl NamedPipeConnector {
    /// Opens the client end of an existing named pipe, waiting up to five
    /// seconds in all if every instance is busy.
    pub fn connect(rt: &Runtime, name: &str) -> windows_core::Result<NamedPipeStream> {
        Self::connect_with_timeout(rt, name, server::DEFAULT_CONNECT_TIMEOUT)
    }

    /// [`connect`](Self::connect), waiting up to `timeout` in all while every
    /// instance is busy; zero does not wait. Running out of time is
    /// `ERROR_PIPE_BUSY`.
    ///
    /// Blocks the calling thread while it waits: the wait (`WaitNamedPipeW`)
    /// has no overlapped form. A pipe that does not exist fails at once.
    pub fn connect_with_timeout(
        rt: &Runtime,
        name: &str,
        timeout: Duration,
    ) -> windows_core::Result<NamedPipeStream> {
        let client = server::open_client_waiting(&HSTRING::from(name), timeout)?;
        let res = crate::tp::io::register_on(&rt.inner, unsafe { crate::common::overlapped::IoHandle::from_handle(client.handle) })?;
        Ok(NamedPipeStream {
            res,
            server_side: false,
            message_mode: client.message_mode,
        })
    }
}

#[cfg(test)]
mod tests;
