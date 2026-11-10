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
        let res = IoRes::register(&self.rt.inner, instance)?;
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
        let res = IoRes::register(&rt.inner, unsafe { crate::common::overlapped::IoHandle::from_handle(client.handle) })?;
        Ok(NamedPipeStream {
            res,
            server_side: false,
            message_mode: client.message_mode,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::common::security::SecurityDescriptor;
    use super::super::super::{block_on, timeout, Runtime};
    use super::*;

    #[test]
    fn named_pipe_echo() {
        let rt = Runtime::new();
        let name = format!(r"\\.\pipe\tp_pipe_echo_{}", std::process::id());
        let rt2 = rt.clone();
        let sname = name.clone();
        let server = std::thread::spawn(move || {
            let listener = NamedPipeListener::bind(&rt2, &sname, &NamedPipeOptions::new()).unwrap();
            let s = block_on(listener.accept()).unwrap();
            let (n, b) = block_on(s.read(vec![0u8; 16]));
            let n = n.unwrap();
            block_on(s.write(b[..n].to_vec())).0.unwrap();
            n
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let client = NamedPipeConnector::connect(&rt, &name).unwrap();
        block_on(client.write(b"hello".to_vec())).0.unwrap();
        let (n, b) = block_on(client.read(vec![0u8; 16]));
        assert_eq!(&b[..n.unwrap()], b"hello");
        assert_eq!(server.join().unwrap(), 5);
    }

    fn unique(tag: &str) -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        format!(
            r"\\.\pipe\tp_pipe_{tag}_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        )
    }

    /// Accepts one client of `name` on a thread, returning the server end.
    fn accept_one(rt: &Runtime, listener: NamedPipeListener) -> std::thread::JoinHandle<windows_core::Result<NamedPipeStream>> {
        let _ = rt;
        std::thread::spawn(move || block_on(listener.accept()))
    }

    /// Both ends name each other, and the server reads the client's account
    /// from its token -- this process, on both counts.
    #[test]
    fn each_end_identifies_the_other() {
        let rt = Runtime::new();
        let name = unique("peer");
        let server = accept_one(&rt, NamedPipeListener::bind(
            &rt,
            &name,
            &NamedPipeOptions::new(),
        ).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(100));
        let client = NamedPipeConnector::connect(&rt, &name).unwrap();
        let server = server.join().unwrap().unwrap();

        let me = std::process::id();
        let my_sid = security::process_user_sid(me).unwrap();
        assert_eq!(server.peer_process_id().unwrap(), me);
        assert_eq!(server.peer_user_sid().unwrap(), my_sid);
        assert_eq!(client.peer_process_id().unwrap(), me);
        assert_eq!(client.peer_user_sid().unwrap(), my_sid);
    }

    /// A pipe someone else created first is not joined: the first instance
    /// asks to be first, and is refused.
    #[test]
    fn a_squatted_name_is_refused() {
        use windows::Win32::Storage::FileSystem::{FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX};
        use windows::Win32::System::Pipes::{
            CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        };
        let rt = Runtime::new();
        let name = unique("squat");
        let squatter = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(&name),
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0),
                PIPE_TYPE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                0,
                0,
                0,
                None,
            )
        };
        assert!(!squatter.is_invalid());
        let err = block_on(NamedPipeListener::bind(
            &rt,
            &name,
            &NamedPipeOptions::new(),
        ).unwrap().accept()).err().unwrap();
        assert_eq!(err.code(), windows::Win32::Foundation::ERROR_ACCESS_DENIED.to_hresult());
        unsafe { windows::Win32::Foundation::CloseHandle(squatter).unwrap() };
    }

    /// Once a client has connected and gone, the name is still ours: the
    /// next instance was created before the connected one was handed on.
    #[test]
    fn the_name_is_held_between_accepts() {
        use windows::Win32::Storage::FileSystem::{
            FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX,
        };
        use windows::Win32::System::Pipes::{
            CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        };
        let rt = Runtime::new();
        let name = unique("held");
        let listener = Arc::new(NamedPipeListener::bind(
            &rt,
            &name,
            &NamedPipeOptions::new(),
        ).unwrap());
        let l2 = listener.clone();
        let server = std::thread::spawn(move || block_on(l2.accept()));
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(NamedPipeConnector::connect(&rt, &name).unwrap());
        drop(server.join().unwrap().unwrap());

        let squatter = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(&name),
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_FIRST_PIPE_INSTANCE.0),
                PIPE_TYPE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                0,
                0,
                0,
                None,
            )
        };
        assert!(squatter.is_invalid(), "the name was free between accepts");
        drop(listener);
    }

    /// An accept dropped before anyone connects -- here by a timeout -- still
    /// leaves the name held: its instance is replaced on the way out.
    #[test]
    fn a_dropped_accept_keeps_the_name() {
        use windows::Win32::Storage::FileSystem::{
            FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX,
        };
        use windows::Win32::System::Pipes::{
            CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        };
        let rt = Runtime::new();
        let name = unique("dropped");
        let listener = NamedPipeListener::bind(&rt, &name, &NamedPipeOptions::new()).unwrap();
        let timed_out = block_on(crate::tp::timeout(
            &rt,
            std::time::Duration::from_millis(100),
            listener.accept(),
        ))
        .is_err();
        assert!(timed_out);
        let squatter = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(&name),
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_FIRST_PIPE_INSTANCE.0),
                PIPE_TYPE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                0,
                0,
                0,
                None,
            )
        };
        assert!(squatter.is_invalid(), "the name was free after a dropped accept");
        // And the reserved instance still serves the next accept.
        let server = accept_one(&rt, listener);
        let _client = NamedPipeConnector::connect(&rt, &name).unwrap();
        server.join().unwrap().unwrap();
    }

    /// A descriptor that does not grant this account keeps it out.
    #[test]
    fn a_descriptor_keeps_out_who_it_does_not_grant() {
        let rt = Runtime::new();
        let name = unique("deny");
        let system_only = SecurityDescriptor::from_sddl("D:P(A;;GA;;;SY)").unwrap();
        let listener = NamedPipeListener::bind(
            &rt,
            &name,
            &NamedPipeOptions::new().security(&system_only),
        ).unwrap();
        // Nobody is let in, so the accept only ends by timing out -- which
        // drops it and cancels its operation, rather than leaving one live
        // for the leak checks elsewhere in this process to trip over.
        let rt2 = rt.clone();
        let server = std::thread::spawn(move || {
            block_on(crate::tp::timeout(
                &rt2,
                std::time::Duration::from_millis(500),
                listener.accept(),
            ))
            .is_err()
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let err = NamedPipeConnector::connect(&rt, &name).err().unwrap();
        assert_eq!(err.code(), windows::Win32::Foundation::ERROR_ACCESS_DENIED.to_hresult());
        assert!(server.join().unwrap(), "the accept completed for a client it should refuse");
    }

    /// And one that does grant it lets it in.
    #[test]
    fn a_descriptor_admits_who_it_grants() {
        let rt = Runtime::new();
        let name = unique("allow");
        let owner_only = SecurityDescriptor::from_sddl("D:P(A;;GA;;;OW)").unwrap();
        let server = accept_one(&rt, NamedPipeListener::bind(
            &rt,
            &name,
            &NamedPipeOptions::new().security(&owner_only),
        ).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(100));
        let _client = NamedPipeConnector::connect(&rt, &name).unwrap();
        server.join().unwrap().unwrap();
    }

    /// Opens `name` read-only as the anonymous account, the way any local
    /// process could under `CreateNamedPipeW`'s own default descriptor.
    pub(crate) fn open_read_only_as_anonymous(name: &str) -> windows_core::Result<()> {
        use windows::Win32::Security::{ImpersonateAnonymousToken, RevertToSelf};
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_SHARE_NONE, OPEN_EXISTING,
        };
        use windows::Win32::System::Threading::GetCurrentThread;
        let name = name.to_owned();
        std::thread::spawn(move || unsafe {
            ImpersonateAnonymousToken(GetCurrentThread()).unwrap();
            let opened = CreateFileW(
                &HSTRING::from(&name),
                FILE_GENERIC_READ.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            );
            RevertToSelf().unwrap();
            opened.map(|h| {
                let _ = windows::Win32::Foundation::CloseHandle(h);
            })
        })
        .join()
        .unwrap()
    }

    /// The default descriptor is the one the docs promise. The system's own
    /// default would let this read-only, anonymous open through.
    #[test]
    fn the_default_descriptor_keeps_out_anonymous_readers() {
        let rt = Runtime::new();
        let name = unique("anon");
        let listener = NamedPipeListener::bind(&rt, &name, &NamedPipeOptions::new()).unwrap();
        let rt2 = rt.clone();
        let server = std::thread::spawn(move || {
            block_on(crate::tp::timeout(
                &rt2,
                std::time::Duration::from_millis(500),
                listener.accept(),
            ))
            .is_err()
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let err = open_read_only_as_anonymous(&name).unwrap_err();
        assert_eq!(err.code(), windows::Win32::Foundation::ERROR_ACCESS_DENIED.to_hresult());
        assert!(server.join().unwrap(), "the accept completed for an anonymous reader");
    }

    /// A message-mode pair: the server end, and a client that found out the
    /// mode by itself.
    fn message_pair(rt: &Runtime, options: NamedPipeOptions) -> (NamedPipeStream, NamedPipeStream) {
        let name = unique("msg");
        let listener =
            NamedPipeListener::bind(rt, &name, &options.message_mode(true)).unwrap();
        let server = accept_one(rt, listener);
        std::thread::sleep(std::time::Duration::from_millis(100));
        let client = NamedPipeConnector::connect(rt, &name).unwrap();
        (server.join().unwrap().unwrap(), client)
    }

    /// Messages arrive whole and one at a time, whatever their size --
    /// including empty, and including larger than the first read.
    #[test]
    fn message_mode_keeps_boundaries() {
        let rt = Runtime::new();
        let (server, client) = message_pair(&rt, NamedPipeOptions::new());
        assert!(server.is_message_mode());
        assert!(client.is_message_mode(), "the connector did not notice message mode");

        let big: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
        let sent = [b"hello".to_vec(), Vec::new(), big.clone(), b"!".to_vec()];
        let writer = {
            let sent = sent.clone();
            let rt = rt.clone();
            std::thread::spawn(move || {
                let _ = &rt;
                for m in sent {
                    block_on(client.write(m)).0.unwrap();
                }
                client
            })
        };
        for m in &sent {
            assert_eq!(&block_on(server.read_message_vec(1 << 20)).unwrap(), m);
        }
        drop(writer.join().unwrap());
    }

    /// A plain `read` still works on a message pipe; it just cannot say
    /// where a message ends.
    #[test]
    fn a_byte_read_returns_a_message_in_parts() {
        let rt = Runtime::new();
        let (server, client) = message_pair(&rt, NamedPipeOptions::new());
        block_on(client.write(b"hello".to_vec())).0.unwrap();
        let (n, b) = block_on(server.read(vec![0u8; 3]));
        assert_eq!(&b[..n.unwrap()], b"hel");
        let (n, b) = block_on(server.read(vec![0u8; 3]));
        assert_eq!(&b[..n.unwrap()], b"lo");
    }

    /// Too long for the caller: read to its end, discarded, and the message
    /// after it is untouched.
    #[test]
    fn an_oversized_message_is_discarded_whole() {
        let rt = Runtime::new();
        let (server, client) = message_pair(&rt, NamedPipeOptions::new());
        // On a thread: a message larger than the pipe's buffer is not written
        // until it is read.
        let writer = std::thread::spawn(move || {
            block_on(client.write(vec![1u8; 100_000])).0.unwrap();
            block_on(client.write(b"next".to_vec())).0.unwrap();
            client
        });
        let err = block_on(server.read_message_vec(10)).unwrap_err();
        assert_eq!(err.code(), ERROR_MORE_DATA.to_hresult());
        assert_eq!(block_on(server.read_message_vec(10)).unwrap(), b"next");
        drop(writer.join().unwrap());
    }

    #[test]
    fn a_byte_pipe_has_no_messages() {
        let rt = Runtime::new();
        let name = unique("bytes");
        let server = accept_one(&rt, NamedPipeListener::bind(
            &rt,
            &name,
            &NamedPipeOptions::new(),
        ).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(100));
        let client = NamedPipeConnector::connect(&rt, &name).unwrap();
        let server = server.join().unwrap().unwrap();
        assert!(!client.is_message_mode());
        assert_eq!(
            block_on(server.read_message_vec(16)).unwrap_err().code(),
            E_ILLEGAL_METHOD_CALL
        );
    }

    #[test]
    fn options_no_pipe_can_have_are_refused() {
        let rt = Runtime::new();
        let invalid = windows::Win32::Foundation::E_INVALIDARG;
        for options in [
            NamedPipeOptions::new().max_instances(0),
            NamedPipeOptions::new().max_instances(256),
            NamedPipeOptions::new().accept_remote_clients(true),
        ] {
            let err = NamedPipeListener::bind(&rt, &unique("bad"), &options)
                .err()
                .unwrap();
            assert_eq!(err.code(), invalid, "{options:?}");
        }
    }

    /// `\\localhost\pipe\...` goes through the SMB redirector even on the
    /// same machine, so it is what a remote client looks like.
    fn remote_name(local: &str) -> String {
        local.replacen(r"\\.\", r"\\localhost\", 1)
    }

    #[test]
    fn remote_clients_are_refused_by_default() {
        let rt = Runtime::new();
        let name = unique("remote_no");
        let listener = NamedPipeListener::bind(&rt, &name, &NamedPipeOptions::new()).unwrap();
        let rt2 = rt.clone();
        let server = std::thread::spawn(move || {
            block_on(crate::tp::timeout(
                &rt2,
                std::time::Duration::from_millis(500),
                listener.accept(),
            ))
            .is_err()
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let err = NamedPipeConnector::connect_with_timeout(&rt, &remote_name(&name), Duration::ZERO)
            .err()
            .unwrap();
        eprintln!("remote connect to a local-only pipe -> {err:?}");
        assert!(server.join().unwrap(), "the accept completed for a remote client");
    }

    #[test]
    fn remote_clients_are_admitted_when_asked() {
        let rt = Runtime::new();
        let name = unique("remote_yes");
        let me = security::process_user_sid(std::process::id()).unwrap();
        let only_me = SecurityDescriptor::from_sddl(&format!("D:P(A;;GA;;;{me})")).unwrap();
        let options = NamedPipeOptions::new()
            .security(&only_me)
            .accept_remote_clients(true);
        let listener = NamedPipeListener::bind(&rt, &name, &options).unwrap();
        let server = accept_one(&rt, listener);
        std::thread::sleep(std::time::Duration::from_millis(100));
        let client = NamedPipeConnector::connect(&rt, &remote_name(&name)).unwrap();
        let server = server.join().unwrap().unwrap();
        block_on(client.write(b"over smb".to_vec())).0.unwrap();
        let (n, b) = block_on(server.read(vec![0u8; 16]));
        assert_eq!(&b[..n.unwrap()], b"over smb");
        eprintln!(
            "remote peer: pid {:?} (this process is {}), sid {:?}",
            server.peer_process_id(),
            std::process::id(),
            server.peer_user_sid()
        );
        assert_eq!(server.peer_user_sid().unwrap(), me);
    }

    /// At the limit an accept waits for an instance to close, and a client
    /// is told the pipe is busy.
    #[test]
    fn the_instance_limit_holds_and_frees() {
        let rt = Runtime::new();
        let name = unique("limit");
        let options = NamedPipeOptions::new().max_instances(1);
        let listener =
            Arc::new(NamedPipeListener::bind(&rt, &name, &options).unwrap());

        let l = listener.clone();
        let first = std::thread::spawn(move || block_on(l.accept()));
        std::thread::sleep(std::time::Duration::from_millis(100));
        let client1 = NamedPipeConnector::connect(&rt, &name).unwrap();
        let server1 = first.join().unwrap().unwrap();

        // The one instance is taken, so the next accept waits for it.
        let l = listener.clone();
        let second = std::thread::spawn(move || block_on(l.accept()));
        std::thread::sleep(std::time::Duration::from_millis(100));
        let err = NamedPipeConnector::connect_with_timeout(&rt, &name, Duration::ZERO)
            .err()
            .unwrap();
        assert_eq!(err.code(), windows::Win32::Foundation::ERROR_PIPE_BUSY.to_hresult());
        assert!(!second.is_finished(), "an accept went past the instance limit");

        // Closing the first frees the slot; the waiting accept takes it.
        // Until it has, no instance exists and the name is not found -- the
        // gap a limit allows -- so the client retries briefly.
        drop(server1);
        drop(client1);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let _client2 = loop {
            match NamedPipeConnector::connect_with_timeout(&rt, &name, Duration::from_secs(1)) {
                Err(e)
                    if e.code() == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND.to_hresult()
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                other => break other.unwrap(),
            }
        };
        second.join().unwrap().unwrap();
    }

    /// With a limit there may be no instance in reserve, so the name can be
    /// free between accepts. Someone who takes it then is not joined: with
    /// no instance of ours left, the next one asks to be first.
    #[test]
    fn a_name_lost_at_the_limit_is_not_joined() {
        use windows::Win32::Storage::FileSystem::{FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX};
        use windows::Win32::System::Pipes::{
            CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        };
        let rt = Runtime::new();
        let name = unique("limit_squat");
        let options = NamedPipeOptions::new().max_instances(1);
        let listener =
            Arc::new(NamedPipeListener::bind(&rt, &name, &options).unwrap());
        let l = listener.clone();
        let first = std::thread::spawn(move || block_on(l.accept()));
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(NamedPipeConnector::connect(&rt, &name).unwrap());
        drop(first.join().unwrap().unwrap());

        let squatter = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(&name),
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0),
                PIPE_TYPE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                0,
                0,
                0,
                None,
            )
        };
        assert!(!squatter.is_invalid(), "the name was still held");
        let err = block_on(listener.accept()).err().unwrap();
        assert_eq!(err.code(), windows::Win32::Foundation::ERROR_ACCESS_DENIED.to_hresult());
        unsafe { windows::Win32::Foundation::CloseHandle(squatter).unwrap() };
    }

    /// A connect gives up when its time is spent, not before and not never.
    #[test]
    fn a_connect_waits_its_timeout_for_a_busy_pipe() {
        let rt = Runtime::new();
        let name = unique("busy");
        let options = NamedPipeOptions::new().max_instances(1);
        let listener = NamedPipeListener::bind(&rt, &name, &options).unwrap();
        let server = accept_one(&rt, listener);
        std::thread::sleep(std::time::Duration::from_millis(100));
        let _client = NamedPipeConnector::connect(&rt, &name).unwrap();
        let _server = server.join().unwrap().unwrap();

        let started = std::time::Instant::now();
        let err = NamedPipeConnector::connect_with_timeout(&rt, &name, Duration::from_millis(300))
            .err()
            .unwrap();
        let waited = started.elapsed();
        assert_eq!(err.code(), windows::Win32::Foundation::ERROR_PIPE_BUSY.to_hresult());
        assert!(
            waited >= Duration::from_millis(250) && waited < Duration::from_secs(3),
            "waited {waited:?} for a 300 ms timeout"
        );
    }

    /// A server that answers each message with `reply(message)` until the
    /// client goes away.
    fn answering(
        server: NamedPipeStream,
        reply: impl Fn(&[u8]) -> Vec<u8> + Send + 'static,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            while let Ok(m) = block_on(server.read_message_vec(1 << 20)) {
                if block_on(server.write(reply(&m))).0.is_err() {
                    break;
                }
            }
        })
    }

    /// One call writes the request and reads the reply, whatever its size.
    #[test]
    fn transact_round_trips() {
        let rt = Runtime::new();
        let (server, client) = message_pair(&rt, NamedPipeOptions::new());
        let server = answering(server, |m| {
            if m == b"big" {
                (0..150_000u32).map(|i| i as u8).collect()
            } else {
                m.to_ascii_uppercase()
            }
        });
        assert_eq!(block_on(client.transact_vec(b"ping".to_vec(), 64)).unwrap(), b"PING");
        let big = block_on(client.transact_vec(b"big".to_vec(), 1 << 20)).unwrap();
        assert_eq!(big, (0..150_000u32).map(|i| i as u8).collect::<Vec<_>>());
        assert_eq!(block_on(client.transact_vec(b"again".to_vec(), 64)).unwrap(), b"AGAIN");
        drop(client);
        server.join().unwrap();
    }

    /// A reply too long for the caller is discarded whole, and the pipe is
    /// ready for the next transaction.
    #[test]
    fn an_oversized_reply_is_discarded_whole() {
        let rt = Runtime::new();
        let (server, client) = message_pair(&rt, NamedPipeOptions::new());
        let server = answering(server, |m| {
            if m == b"big" { vec![9u8; 100_000] } else { m.to_vec() }
        });
        let err = block_on(client.transact_vec(b"big".to_vec(), 10)).unwrap_err();
        assert_eq!(err.code(), ERROR_MORE_DATA.to_hresult());
        assert_eq!(block_on(client.transact_vec(b"small".to_vec(), 10)).unwrap(), b"small");
        drop(client);
        server.join().unwrap();
    }

    #[test]
    fn a_byte_pipe_has_no_transactions() {
        let rt = Runtime::new();
        let name = unique("bytes_tx");
        let server = accept_one(&rt, NamedPipeListener::bind(
            &rt,
            &name,
            &NamedPipeOptions::new(),
        ).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(100));
        let client = NamedPipeConnector::connect(&rt, &name).unwrap();
        let _server = server.join().unwrap().unwrap();
        assert_eq!(
            block_on(client.transact_vec(b"x".to_vec(), 16)).unwrap_err().code(),
            E_ILLEGAL_METHOD_CALL
        );
    }

    /// A connected byte-mode pair, server end first.
    fn byte_pair(rt: &Runtime) -> (NamedPipeStream, NamedPipeStream) {
        let name = unique("own");
        let server = accept_one(rt, NamedPipeListener::bind(
            rt,
            &name,
            &NamedPipeOptions::new(),
        ).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(100));
        let client = NamedPipeConnector::connect(rt, &name).unwrap();
        (server.join().unwrap().unwrap(), client)
    }

    /// A read lands in spare capacity: no zeroed buffer, and the length ends
    /// where the data does.
    #[test]
    fn a_read_fills_spare_capacity() {
        let rt = Runtime::new();
        let (server, client) = byte_pair(&rt);
        block_on(client.write(b"hello".to_vec())).0.unwrap();
        let (n, got) = block_on(server.read(Vec::with_capacity(64)));
        assert_eq!(n.unwrap(), 5);
        assert_eq!(got, b"hello", "the length did not follow the data");

        // A zero-filled buffer keeps its length, as callers have relied on.
        block_on(client.write(b"abc".to_vec())).0.unwrap();
        let (n, got) = block_on(server.read(vec![0u8; 16]));
        assert_eq!(n.unwrap(), 3);
        assert_eq!(got.len(), 16);
        assert_eq!(&got[..3], b"abc");
    }

    /// Reading into, and writing out of, part of one buffer.
    #[test]
    fn slices_read_and_write_part_of_a_buffer() {
        use crate::common::buffer::IoBuf;
        let rt = Runtime::new();
        let (server, client) = byte_pair(&rt);

        // Send bytes 2..5 of a larger frame.
        let (n, frame) = block_on(client.write(b"xxPAYyy".to_vec().slice(2..5)));
        assert_eq!(n.unwrap(), 3);
        assert_eq!(frame.into_inner(), b"xxPAYyy");

        // Append the read after what the buffer already holds.
        let mut buf = Vec::with_capacity(32);
        buf.extend_from_slice(b"head:");
        let (n, rest) = block_on(server.read(buf.slice(5..)));
        assert_eq!(n.unwrap(), 3);
        assert_eq!(rest.into_inner(), b"head:PAY");
    }

    /// One shared allocation out on two connections, and memory that is
    /// never freed.
    #[test]
    fn shared_and_static_buffers_can_be_sent() {
        let rt = Runtime::new();
        let (server_a, client_a) = byte_pair(&rt);
        let (server_b, client_b) = byte_pair(&rt);
        let shared: Arc<[u8]> = Arc::from(&b"shared"[..]);
        block_on(client_a.write(shared.clone())).0.unwrap();
        block_on(client_b.write(shared.clone())).0.unwrap();
        static BANNER: &[u8] = b"!";
        block_on(client_a.write(BANNER)).0.unwrap();
        let (n, got) = block_on(server_a.read(Vec::with_capacity(16)));
        assert_eq!(&got[..n.unwrap()], b"shared!");
        let (_, got) = block_on(server_b.read(Vec::with_capacity(16)));
        assert_eq!(got, b"shared");
    }

    /// Memory the crate does not own, kept alive by what does.
    #[test]
    fn a_raw_buffer_wraps_foreign_memory() {
        use crate::common::buffer::{RawBuf, Region};
        let rt = Runtime::new();
        let (server, client) = byte_pair(&rt);
        let region = Region::new(4096).unwrap();
        let raw = unsafe { RawBuf::from_raw_parts(region.base(), 0, 4096, region) };
        block_on(client.write(b"mapped".to_vec())).0.unwrap();
        let (n, raw) = block_on(server.read(raw));
        assert_eq!(n.unwrap(), 6);
        assert_eq!(raw.as_slice(), b"mapped");
    }

    #[cfg(feature = "bytes")]
    #[test]
    fn bytes_buffers_go_both_ways() {
        let rt = Runtime::new();
        let (server, client) = byte_pair(&rt);
        block_on(client.write(bytes::Bytes::from_static(b"from bytes"))).0.unwrap();
        let (n, got) = block_on(server.read(bytes::BytesMut::with_capacity(64)));
        assert_eq!(n.unwrap(), 10);
        assert_eq!(&got[..], b"from bytes");
    }

    /// A message into a fixed buffer of the caller's: it fits, or it is
    /// discarded whole and the next one is untouched.
    #[test]
    fn messages_into_the_callers_buffer() {
        use crate::common::buffer::SlabPool;
        let rt = Runtime::new();
        let (server, client) = message_pair(&rt, NamedPipeOptions::new());
        let writer = std::thread::spawn(move || {
            for m in [&b"fits"[..], &[3u8; 100], b"next"] {
                block_on(client.write(m.to_vec())).0.unwrap();
            }
            client
        });
        let pool = SlabPool::new(16, 1).unwrap();
        let (n, buf) = block_on(server.read_message(pool.acquire().unwrap()));
        assert_eq!(&buf[..n.unwrap()], b"fits");
        let (err, buf) = block_on(server.read_message(buf));
        assert_eq!(err.unwrap_err().code(), ERROR_MORE_DATA.to_hresult());
        let (n, _) = block_on(server.read_message(buf));
        assert_eq!(n.unwrap(), 4);
        drop(writer.join().unwrap());
    }

    #[test]
    fn transact_into_the_callers_buffers() {
        let rt = Runtime::new();
        let (server, client) = message_pair(&rt, NamedPipeOptions::new());
        let server = answering(server, |m| m.to_ascii_uppercase());
        let request: Arc<[u8]> = Arc::from(&b"shout"[..]);
        let (n, _request, reply) =
            block_on(client.transact(request, Vec::with_capacity(32)));
        assert_eq!(n.unwrap(), 5);
        assert_eq!(reply, b"SHOUT");
        drop(client);
        server.join().unwrap();
    }

    /// Borrowed buffers both ways: a stack array in, a slice out.
    #[test]
    fn unchecked_ops_take_borrowed_buffers() {
        let rt = Runtime::new();
        let (server, client) = byte_pair(&rt);
        let request = *b"borrowed";
        let mut reply = [0u8; 16];
        // SAFETY: each future is awaited to completion right here.
        let (n, _) = block_on(unsafe { client.write_unchecked(&request[..]) });
        assert_eq!(n.unwrap(), 8);
        let (n, _) = block_on(unsafe { server.read_unchecked(&mut reply[..]) });
        assert_eq!(&reply[..n.unwrap()], b"borrowed");
    }

    /// The point of the waiting drop: once a cancelled read's future is gone,
    /// the kernel no longer writes into the buffer -- data that arrives later
    /// is left in the pipe for the next read, and the buffer keeps what the
    /// caller put there.
    #[test]
    fn a_dropped_unchecked_read_lets_go_of_the_buffer() {
        let rt = Runtime::new();
        let (server, client) = byte_pair(&rt);
        let mut buf = [0u8; 16];
        // SAFETY: the future is dropped by the timeout, never forgotten.
        let timed_out = block_on(timeout(
            &rt,
            std::time::Duration::from_millis(100),
            unsafe { server.read_unchecked(&mut buf[..]) },
        ))
        .is_err();
        assert!(timed_out);
        buf.fill(0xAA);
        block_on(client.write(b"late".to_vec())).0.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(buf.iter().all(|&b| b == 0xAA), "the kernel wrote after the drop: {buf:?}");
        let (n, got) = block_on(server.read(Vec::with_capacity(16)));
        assert_eq!(&got[..n.unwrap()], b"late", "the late data was lost");
    }

    /// Dropping on the pool's only thread does not deadlock: the drop waits
    /// on the operation's status, not on a callback that thread would have
    /// to run.
    #[test]
    fn a_dropped_unchecked_read_does_not_need_a_pool_thread() {
        let rt = crate::tp::PoolBuilder::new().thread_limits(1, 1).build_runtime();
        let (server, _client) = byte_pair(&rt);
        let (done, finished) = std::sync::mpsc::channel();
        let rt2 = rt.clone();
        let task = rt.spawn(async move {
            let mut buf = [0u8; 16];
            // SAFETY: dropped by the timeout, inside this task.
            let r = timeout(&rt2, std::time::Duration::from_millis(50), unsafe {
                server.read_unchecked(&mut buf[..])
            })
            .await;
            r.is_err()
        });
        std::thread::spawn(move || {
            let _ = done.send(block_on(task));
        });
        let timed_out = finished
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("deadlocked dropping an unchecked read on a one-thread pool");
        assert!(timed_out.unwrap());
    }

    /// Cancellation racing completion, many times over: whatever wins, the
    /// drop returns, and nothing arrives twice or out of order.
    ///
    /// Bytes *can* be lost, and that is not a defect of this API: when the
    /// read completes in the same instant its future is dropped, the kernel
    /// has already taken the bytes out of the pipe, and the future that would
    /// have returned them is gone. Every completion-based read has that
    /// property (an owned-buffer read dropped the same way loses its buffer
    /// and the data in it). CI caught a 299-of-300 run of an earlier version
    /// of this test that asserted otherwise.
    #[test]
    fn unchecked_drops_race_completions_cleanly() {
        let rt = Runtime::new();
        let (server, client) = byte_pair(&rt);
        let writer = std::thread::spawn(move || {
            for i in 0..300u32 {
                block_on(client.write(vec![(i % 251) as u8])).0.unwrap();
                if i % 7 == 0 {
                    std::thread::yield_now();
                }
            }
            client
        });
        let mut seen: Vec<u8> = Vec::new();
        for i in 0..2000u32 {
            let mut buf = [0u8; 4];
            let wait = std::time::Duration::from_micros(u64::from(i % 5) * 200);
            // SAFETY: dropped by the timeout or completed, never forgotten.
            if let Ok((n, _)) = block_on(timeout(&rt, wait, unsafe { server.read_unchecked(&mut buf[..]) })) {
                seen.extend_from_slice(&buf[..n.unwrap()]);
            }
            if seen.len() >= 300 {
                break;
            }
        }
        let client = writer.join().unwrap();
        drop(client);
        // Whatever was left unread arrives now.
        loop {
            let (n, rest) = block_on(server.read(Vec::with_capacity(512)));
            match n {
                Ok(0) | Err(_) => break,
                Ok(n) => seen.extend_from_slice(&rest[..n]),
            }
        }
        // The values were written in order, 0..251 then again from 0: every
        // byte that arrived must continue that order without repeating one.
        assert!(!seen.is_empty() && seen.len() <= 300, "{} bytes", seen.len());
        let mut expected = 0usize;
        for &b in &seen {
            while (expected % 251) as u8 != b {
                expected += 1;
                assert!(expected < 300, "a byte arrived twice or out of order");
            }
            expected += 1;
        }
    }
}
