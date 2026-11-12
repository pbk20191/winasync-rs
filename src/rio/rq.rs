//! Request queues — the per-socket half of the engine, shared by the UDP and
//! TCP sockets.
//!
//! A request queue is where operations are posted; the completion queue is
//! where their results land. Both sockets need exactly the same plumbing
//! around a post — arm the buffer's `Request` slot, hand its address to RIO
//! as the request context, and disarm it if the call is rejected — so that
//! lives here once rather than per socket type.
//!
//! The sentinels below are from `mswsockdef.h`. `windows` binds RIO's types
//! and function pointers but not these macros, so they are declared here, in
//! one place, for every RIO module.

use std::sync::Arc;
use windows::Win32::Networking::WinSock::{
    RIO_BUFFERID, RIO_CQ, RIO_RQ, SOCKADDR, SOCKADDR_STORAGE, SOCKET, WSASocketW,
    WSA_FLAG_OVERLAPPED, WSA_FLAG_REGISTERED_IO,
};

use super::buffer::RioSlice;
use super::cq::{Outcome, Request, RioCq};
use super::table::rio_table;
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
/// `WSA_FLAG_REGISTERED_IO` is accepted only by the TCP/IP families: AF_UNIX
/// and AF_HYPERV fail here with `WSAEINVAL`, at creation rather than at
/// queue setup, with or without `WSA_FLAG_OVERLAPPED` beside the flag
/// (measured on 26320 by `probe_tests::rio_families`).
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
///
/// `!Sync`: `RIO_RQ` has no internal synchronization, so two threads inside
/// `RIOSend` on one queue corrupt its slot bookkeeping. Rather than a lock
/// of our own, the type says so, and the sockets built on it are `!Sync`
/// too. Every post is synchronous -- the returned future holds no reference
/// to the queue -- so a caller who does want two tasks on one socket wraps
/// it in a `Mutex` held only across the post, not across the await.
pub(crate) struct RequestQueue {
    rq: RIO_RQ,
    cq: Arc<RioCq>,
    _single_poster: std::marker::PhantomData<core::cell::Cell<()>>,
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
            rq,
            cq: cq.clone(),
            _single_poster: std::marker::PhantomData,
        })
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
        let buf = slice.rio_buf();

        // The slot owns the buffer until the completion carries it back —
        // the only way to satisfy RIO's "buffer outlives the op" rule when
        // operations cannot be cancelled.
        let slot = arm(slice, None);
        let ctx = unsafe { (*slot).context() };

        let ok = unsafe {
            if is_send {
                (table.raw().RIOSend.unwrap())(self.rq, &buf, 1, flags, ctx)
            } else {
                (table.raw().RIOReceive.unwrap())(self.rq, &buf, 1, flags, ctx)
            }
        };
        if !ok.as_bool() {
            return Err(reclaim(slot));
        }
        let op = OpFuture { slot, resolved: false };
        Ok(async move {
            let (result, slice) = op.await;
            (result.map(|(n, _)| n), slice)
        })
    }

    /// Resizes the queue's outstanding-operation budget.
    ///
    /// Growing is live. Shrinking below the number currently outstanding is
    /// refused with `WSAETOOMANYREFS`, and the sum across every socket on a
    /// completion queue must still fit that queue — grow the completion queue
    /// first if it will not.
    pub(crate) fn resize(&self, max_recv: u32, max_send: u32) -> windows_core::Result<()> {
        let table = rio_table()?;
        let ok = unsafe { (table.raw().RIOResizeRequestQueue.unwrap())(self.rq, max_recv, max_send) };
        if ok.as_bool() {
            Ok(())
        } else {
            Err(wsa_last_err())
        }
    }

    /// Dispatches everything queued with `RIO_MSG_DEFER`.
    pub(crate) fn commit_sends(&self) -> windows_core::Result<()> {
        let table = rio_table()?;
        let ok = unsafe {
            (table.raw().RIOSend.unwrap())(
                self.rq,
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
    /// `addr` is the registered slice holding the destination (a send) or
    /// receiving the source (a receive); the request keeps it until the
    /// completion, reads the peer from it then, and releases it.
    pub(crate) fn post_ex(
        &self,
        is_send: bool,
        slice: RioSlice,
        addr: RioSlice,
    ) -> windows_core::Result<
        impl Future<Output = (windows_core::Result<(usize, SocketAddr)>, RioSlice)> + use<>,
    > {
        let table = rio_table()?;
        let data_buf = slice.rio_buf();
        let addr_buf = addr.rio_buf();
        let slot = arm(slice, Some(addr));
        let ctx = unsafe { (*slot).context() };
        let (data_buf, addr_buf) = (&data_buf, &addr_buf);

        let ok = {
            let rq = self.rq;
            unsafe {
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
            }
        };
        if !ok {
            return Err(reclaim(slot));
        }
        let op = OpFuture { slot, resolved: false };
        Ok(async move {
            let (result, slice) = op.await;
            (result.map(|(n, peer)| (n, peer.expect("an addressed completion names its peer"))), slice)
        })
    }

    /// Keeps the completion queue alive for as long as this socket lives.
    pub(crate) fn cq(&self) -> &Arc<RioCq> {
        &self.cq
    }
}

/// Arms the slot of `slice` for an operation and names it. The slot is in
/// `slice`'s pool, which the slice — now in the slot — keeps alive until
/// the outcome is taken or the slot emptied, so the pointer is good for
/// the operation's life.
fn arm(slice: RioSlice, addr: Option<RioSlice>) -> *const Request {
    let slot: *const Request = slice.pool().slot(&slice);
    // SAFETY: the pool is alive — `slice` holds it — and the slot is idle,
    // since the slice is this caller's.
    unsafe { (*slot).arm(slice, addr) };
    slot
}

/// Reclaims a request RIO refused to queue: no completion is coming for it
/// and no future was built, so the slot is disarmed here and the buffers go
/// back to their pool.
fn reclaim(slot: *const Request) -> windows_core::Error {
    let err = wsa_last_err();
    // SAFETY: both references are this caller's; the slot is alive.
    unsafe { (*slot).disarm() };
    err
}

/// The future's side of a slot.
///
/// Holds no `Arc` of its own: until the outcome is taken the slot holds
/// the slice, and the slice the pool. Resolving hands the slice out and
/// with it the slot, so a resolved future touches the slot no more; one
/// dropped unresolved [`abandon`](Request::abandon)s it.
struct OpFuture {
    slot: *const Request,
    resolved: bool,
}

// SAFETY: the slot is reached only through atomics on this side, and the
// cells only after `DONE`, from one thread — the one polling.
unsafe impl Send for OpFuture {}

impl Future for OpFuture {
    type Output = Outcome;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Outcome> {
        assert!(!self.resolved, "polled after completion");
        // SAFETY: unresolved, so the slot holds the slice and is alive.
        let outcome = std::task::ready!(unsafe { (*self.slot).poll(cx) });
        self.resolved = true;
        std::task::Poll::Ready(outcome)
    }
}

impl Drop for OpFuture {
    fn drop(&mut self) {
        if !self.resolved {
            // SAFETY: once, unresolved; the slot is alive for it.
            unsafe { Request::abandon(self.slot) };
        }
    }
}
