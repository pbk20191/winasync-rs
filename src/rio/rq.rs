//! Request queues — the per-socket half of the engine, shared by the UDP and
//! TCP sockets.
//!
//! A request queue is where operations are posted; the completion queue is
//! where their results land. Both sockets need exactly the same plumbing
//! around a post — build a `Request` that owns the buffer, hand its pointer to
//! RIO as the request context, and reclaim it if the call is rejected — so
//! that lives here once rather than per socket type.
//!
//! The sentinels below are from `mswsockdef.h`. `windows` binds RIO's types
//! and function pointers but not these macros, so they are declared here, in
//! one place, for every RIO module.

use std::sync::{Arc, Mutex};
use windows::Win32::Networking::WinSock::{
    RIO_BUF, RIO_BUFFERID, RIO_CQ, RIO_RQ, SOCKADDR, SOCKADDR_STORAGE, SOCKET, WSASocketW,
    WSA_FLAG_OVERLAPPED, WSA_FLAG_REGISTERED_IO,
};

use super::buffer::RioSlice;
use super::cq::{CompleteFn, Request, RioCq};
use super::table::rio_table;
use crate::common::oneshot;
use crate::common::winsock::{wsa_init, wsa_last_err};
use std::net::SocketAddr;

/// `RIO_INVALID_BUFFERID` — `(ULONG_PTR)0xFFFFFFFF`, so not sign-extended on
/// 64-bit.
pub(crate) const RIO_INVALID_BUFFERID: RIO_BUFFERID = RIO_BUFFERID(0xFFFF_FFFF);
/// `RIO_INVALID_CQ`.
pub(crate) const RIO_INVALID_CQ: RIO_CQ = RIO_CQ(0);
/// `RIO_INVALID_RQ`.
pub(crate) const RIO_INVALID_RQ: RIO_RQ = RIO_RQ(0);

/// `WSAENOBUFS` (10055), RIO's backpressure signal: a full request queue, or
/// here an exhausted address pool.
pub(crate) const WSAENOBUFS: i32 = 10055;
pub(crate) const WSAENOBUFS_HRESULT: windows_core::HRESULT =
    windows_core::HRESULT(0x8007_2747u32 as i32);

/// How many outstanding operations a socket's request queue allows.
///
/// RIO requires that the sum of every attached socket's limits not exceed the
/// completion queue's size — honouring that is what makes the completion
/// queue impossible to overflow.
#[derive(Clone, Copy, Debug)]
pub struct RqConfig {
    pub max_outstanding_recv: u32,
    pub max_outstanding_send: u32,
}

impl Default for RqConfig {
    fn default() -> Self {
        Self {
            max_outstanding_recv: 64,
            max_outstanding_send: 64,
        }
    }
}

/// Creates a socket able to take a request queue.
///
/// `WSA_FLAG_REGISTERED_IO` is accepted only by the TCP/IP families: AF_UNIX,
/// AF_HYPERV and raw sockets fail here with `WSAEINVAL`, at creation rather
/// than at queue setup.
pub(crate) fn new_rio_socket(
    addr: &SocketAddr,
    sock_type: i32,
    protocol: i32,
) -> windows_core::Result<SOCKET> {
    wsa_init();
    let family = match addr {
        SocketAddr::V4(_) => windows::Win32::Networking::WinSock::AF_INET,
        SocketAddr::V6(_) => windows::Win32::Networking::WinSock::AF_INET6,
    };
    unsafe {
        WSASocketW(
            family.0 as i32,
            sock_type,
            protocol,
            None,
            0,
            WSA_FLAG_OVERLAPPED | WSA_FLAG_REGISTERED_IO,
        )
    }
}

/// Binds `sock` to `addr`.
pub(crate) fn bind_socket(sock: SOCKET, addr: &SocketAddr) -> windows_core::Result<()> {
    let mut storage = unsafe { core::mem::zeroed::<SOCKADDR_STORAGE>() };
    let len = crate::common::sockaddr::write_sockaddr(addr, &mut storage);
    crate::common::winsock::sock_check(unsafe {
        windows::Win32::Networking::WinSock::bind(sock, &storage as *const _ as *const SOCKADDR, len)
    })
}

/// A socket's request queue, attached to a completion queue.
///
/// Holds the `Arc<RioCq>` so the queue and its pump outlive the socket — they
/// have to, since completions keep arriving after `closesocket`.
pub(crate) struct RequestQueue {
    rq: Mutex<RIO_RQ>,
    cq: Arc<RioCq>,
}

impl RequestQueue {
    /// Attaches a queue to an already-created RIO socket.
    pub(crate) fn attach(
        sock: SOCKET,
        cq: &Arc<RioCq>,
        config: RqConfig,
    ) -> windows_core::Result<Self> {
        let table = rio_table()?;
        debug_assert_ne!(cq.raw(), RIO_INVALID_CQ);
        let rq = unsafe {
            (table.raw().RIOCreateRequestQueue.unwrap())(
                sock,
                config.max_outstanding_recv,
                1,
                config.max_outstanding_send,
                1,
                cq.raw(),
                cq.raw(),
                core::ptr::null(),
            )
        };
        if rq == RIO_INVALID_RQ {
            return Err(wsa_last_err());
        }
        Ok(Self {
            rq: Mutex::new(rq),
            cq: cq.clone(),
        })
    }

    pub(crate) fn raw(&self) -> RIO_RQ {
        *self.rq.lock().unwrap()
    }

    /// Posts a plain send or receive.
    ///
    /// The returned future resolves with the byte count and the buffer.
    /// Dropping it does not cancel: RIO has no per-operation cancellation, so
    /// the operation runs to completion either way and the buffer is released
    /// only then.
    pub(crate) fn post(
        &self,
        slice: RioSlice,
        is_send: bool,
        flags: u32,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        let table = rio_table()?;
        let (tx, rx) = oneshot::channel::<(windows_core::Result<usize>, Option<RioSlice>)>();
        let buf = slice.rio_buf();

        let complete: CompleteFn = Box::new(move |status, bytes, returned| {
            let _ = tx.send((status_to_result(status, bytes), returned));
        });
        let ctx = Request {
            complete,
            // The request owns the buffer until the completion carries it
            // back — the only way to satisfy RIO's "buffer outlives the op"
            // rule when operations cannot be cancelled.
            slice: Some(slice),
        }
        .into_context();

        let rq = self.raw();
        let ok = unsafe {
            if is_send {
                (table.raw().RIOSend.unwrap())(rq, &buf, 1, flags, ctx)
            } else {
                (table.raw().RIOReceive.unwrap())(rq, &buf, 1, flags, ctx)
            }
        };
        if !ok.as_bool() {
            return Err(reclaim(ctx));
        }
        Ok(async move { resolve(rx).await })
    }

    /// Resizes the queue's outstanding-operation budget.
    ///
    /// Growing is live. Shrinking below the number currently outstanding is
    /// refused with `WSAETOOMANYREFS`, and the sum across every socket on a
    /// completion queue must still fit that queue — grow the completion queue
    /// first if it will not.
    pub(crate) fn resize(&self, max_recv: u32, max_send: u32) -> windows_core::Result<()> {
        let table = rio_table()?;
        let rq = self.raw();
        let ok = unsafe { (table.raw().RIOResizeRequestQueue.unwrap())(rq, max_recv, max_send) };
        if ok.as_bool() {
            Ok(())
        } else {
            Err(wsa_last_err())
        }
    }

    /// Dispatches everything queued with `RIO_MSG_DEFER`.
    pub(crate) fn commit_sends(&self) -> windows_core::Result<()> {
        let table = rio_table()?;
        let rq = self.raw();
        let ok = unsafe {
            (table.raw().RIOSend.unwrap())(
                rq,
                core::ptr::null(),
                0,
                windows::Win32::Networking::WinSock::RIO_MSG_COMMIT_ONLY,
                core::ptr::null(),
            )
        };
        if ok.as_bool() {
            Ok(())
        } else {
            Err(wsa_last_err())
        }
    }

    /// Posts an addressed send or receive (`RIOSendEx` / `RIOReceiveEx`).
    ///
    /// `addr_buf` must describe a registered buffer that stays alive until the
    /// completion; the caller keeps it by capturing it in `complete`.
    pub(crate) fn post_ex(
        &self,
        data_buf: &RIO_BUF,
        addr_buf: &RIO_BUF,
        is_send: bool,
        slice: RioSlice,
        complete: CompleteFn,
    ) -> windows_core::Result<()> {
        let table = rio_table()?;
        let ctx = Request {
            complete,
            slice: Some(slice),
        }
        .into_context();

        let rq = self.raw();
        let ok = unsafe {
            if is_send {
                (table.raw().RIOSendEx.unwrap())(
                    rq,
                    data_buf,
                    1,
                    core::ptr::null(),
                    addr_buf,
                    core::ptr::null(),
                    core::ptr::null(),
                    0,
                    ctx,
                )
                .as_bool()
            } else {
                // RIOReceiveEx reports success as a non-zero INT rather than
                // a BOOL, unlike its send counterpart.
                (table.raw().RIOReceiveEx.unwrap())(
                    rq,
                    data_buf,
                    1,
                    core::ptr::null(),
                    addr_buf,
                    core::ptr::null(),
                    core::ptr::null(),
                    0,
                    ctx,
                ) != 0
            }
        };
        if !ok {
            return Err(reclaim(ctx));
        }
        Ok(())
    }

    /// Keeps the completion queue alive for as long as this socket lives.
    pub(crate) fn cq(&self) -> &Arc<RioCq> {
        &self.cq
    }
}

/// Turns a completion status into a result.
pub(crate) fn status_to_result(status: i32, bytes: u32) -> windows_core::Result<usize> {
    if status == 0 {
        Ok(bytes as usize)
    } else {
        Err(windows_core::Error::from_hresult(
            windows::Win32::Foundation::WIN32_ERROR(status as u32).to_hresult(),
        ))
    }
}

/// Reclaims a request RIO refused to queue. No completion is coming for it,
/// so its buffer would otherwise be held forever.
fn reclaim(ctx: *const core::ffi::c_void) -> windows_core::Error {
    let err = wsa_last_err();
    let _ = unsafe { Box::from_raw(ctx as *mut Request) };
    err
}

/// Awaits a posted operation's result.
pub(crate) async fn resolve<T>(
    rx: oneshot::Receiver<(windows_core::Result<T>, Option<RioSlice>)>,
) -> (windows_core::Result<T>, RioSlice) {
    match rx.await {
        Ok((result, slice)) => (result, slice.expect("the pump returns the buffer")),
        Err(_) => unreachable!("every posted request completes exactly once"),
    }
}
