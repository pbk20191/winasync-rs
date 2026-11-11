//! A RIO UDP socket: a request queue over a [`RioCq`], with futures for send
//! and receive.
//!
//! # How this differs from `tp::io::net::UdpSocket`
//!
//! * **Buffers come from a [`super::buffer::RioBufferPool`]**, not from the
//!   caller's `Vec`.
//! * **The request queue is sized up front.** `max_outstanding_*` is a hard
//!   cap; exceeding it fails the call with `WSAENOBUFS` *synchronously*
//!   rather than queueing. That is the backpressure signal, not an error to
//!   paper over.
//! * **A dropped future does not cancel.** RIO has no per-operation
//!   cancellation — no `CancelIoEx` equivalent — so dropping a send or
//!   receive future merely stops you observing the result. The operation
//!   still runs and its buffer stays held until it completes. The only way to
//!   stop outstanding work is to drop the socket, and even then completions
//!   arrive afterwards (with `WSAECONNABORTED`).
//! * **One socket, one thread at a time.** Neither the request queue nor the
//!   completion queue is internally synchronized, and this crate adds no
//!   lock of its own: the socket is `!Sync`, which is the kernel's rule
//!   stated as a type. Every post is synchronous and the future it returns
//!   borrows nothing, so a reader task and a writer task can still share
//!   one socket behind a `Mutex` taken only for the post:
//!
//!   ```ignore
//!   let fut = socket.lock().unwrap().recv(slice)?; // guard held for the post
//!   let (n, slice) = fut.await;                      // not across the await
//!   ```

use std::sync::Arc;
use windows::Win32::Networking::WinSock::{
    closesocket, RIO_MSG_DEFER, RIO_MSG_DONT_NOTIFY, SOCKADDR, SOCKADDR_INET, SOCKADDR_STORAGE, SOCKET, SOCK_DGRAM,
};

use super::buffer::RioSlice;
use super::cq::RioCq;
use super::rq::{
    bind_socket, new_rio_socket, resolve, Finish, RequestQueue, RqConfig, WSAENOBUFS_HRESULT,
};
use crate::common::winsock::sock_check;
use crate::common::oneshot;

use std::net::SocketAddr;

/// A UDP socket driven by Registered I/O.
pub struct RioUdpSocket {
    sock: SOCKET,
    /// Registered slots for the `RIO_BUF` addresses that `RIOReceiveEx` /
    /// `RIOSendEx` need. Sized to the request queue, so a post that the queue
    /// would accept always finds a slot.
    addrs: Arc<super::buffer::RioBufferPool>,
    /// Posts operations and keeps the completion queue — with its pump —
    /// alive for this socket's whole life, including the completions that
    /// arrive after close.
    rq: RequestQueue,
}

// A socket handle, a request queue and shared pools, usable from any one
// thread at a time; `Sync` is deliberately not implemented (the request
// queue is `!Sync`).
unsafe impl Send for RioUdpSocket {}

impl RioUdpSocket {
    /// Binds a RIO UDP socket and attaches a request queue to `cq`.
    ///
    /// The socket is created with `WSA_FLAG_REGISTERED_IO`, which only the
    /// TCP/IP families accept — AF_UNIX and AF_HYPERV fail here with
    /// `WSAEINVAL`, at creation rather than at queue setup.
    pub fn bind(
        cq: &Arc<RioCq>,
        addr: SocketAddr,
        config: RqConfig,
    ) -> windows_core::Result<Self> {
        let sock = new_rio_socket(
            &addr,
            SOCK_DGRAM.0,
            windows::Win32::Networking::WinSock::IPPROTO_UDP.0,
        )?;

        let build = || -> windows_core::Result<RequestQueue> {
            bind_socket(sock, &addr)?;
            RequestQueue::attach(sock, cq, config)
        };

        let addrs = match super::buffer::RioBufferPool::new(
            size_of::<SOCKADDR_INET>(),
            config.max_outstanding_recv + config.max_outstanding_send,
        ) {
            Ok(p) => p,
            Err(e) => {
                unsafe { closesocket(sock) };
                return Err(e);
            }
        };

        match build() {
            Ok(rq) => Ok(Self { sock, addrs, rq }),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }

    /// The bound address (useful after binding port 0).
    pub fn local_addr(&self) -> windows_core::Result<SocketAddr> {
        local_addr_of(self.sock)
    }

    /// Posts a receive into `slice`.
    ///
    /// Resolves to the byte count and the slice. Fails synchronously with
    /// `WSAENOBUFS` when the request queue is already at
    /// `max_outstanding_recv` — back off and retry rather than treating it as
    /// fatal.
    ///
    /// Dropping the returned future does **not** cancel the receive; see the
    /// module note.
    pub fn recv(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.rq.post(slice, false, 0)
    }

    /// Posts a send of `slice`'s current length to the connected peer.
    ///
    /// Same queue-full and cancellation semantics as
    /// [`recv`](Self::recv).
    pub fn send(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.rq.post(slice, true, 0)
    }

    /// Connects the socket so [`send`](Self::send) has a destination. RIO has
    /// no connect of its own; this is the ordinary Winsock one.
    pub fn connect(&self, addr: SocketAddr) -> windows_core::Result<()> {
        let mut storage = unsafe { core::mem::zeroed::<SOCKADDR_STORAGE>() };
        let len = write_sockaddr(&addr, &mut storage);
        sock_check(unsafe {
            windows::Win32::Networking::WinSock::connect(
                self.sock,
                &storage as *const _ as *const SOCKADDR,
                len,
            )
        })
    }

    /// Receives from any peer, reporting the source address — the unconnected
    /// counterpart of [`recv`](Self::recv), over `RIOReceiveEx`.
    ///
    /// RIO takes the address as a `RIO_BUF` too, so it must live in a
    /// registered buffer: the socket keeps a small internal pool for that,
    /// sized to its request queue, and the slot is released with the
    /// completion.
    pub fn recv_from(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<
        impl Future<Output = (windows_core::Result<(usize, SocketAddr)>, RioSlice)> + use<>,
    > {
        self.post_ex(slice, None)
    }

    /// Sends `slice` to `dest` without connecting, over `RIOSendEx`.
    pub fn send_to(
        &self,
        slice: RioSlice,
        dest: SocketAddr,
    ) -> windows_core::Result<
        impl Future<Output = (windows_core::Result<(usize, SocketAddr)>, RioSlice)> + use<>,
    > {
        self.post_ex(slice, Some(dest))
    }

    /// Queues a send without executing it (`RIO_MSG_DEFER`), to be released
    /// later by [`commit_sends`](Self::commit_sends).
    ///
    /// Deferring lets several sends be handed to the provider and then
    /// dispatched with one crossing, which is the send-side half of what a
    /// completion queue does for results. Each deferred send still completes
    /// individually.
    pub fn send_deferred(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.rq.post(slice, true, RIO_MSG_DEFER)
    }

    /// Posts a receive whose completion will **not** raise a notification
    /// (`RIO_MSG_DONT_NOTIFY`).
    ///
    /// The result still lands in the completion queue; it just does not wake
    /// anyone. That is the building block for a hybrid loop: poll the queue
    /// while traffic is heavy and pay nothing per completion, and arm a
    /// notification only when it runs dry. Published measurements have that
    /// arrangement doing nearly all the work on a couple of threads where a
    /// notify-per-completion design kept every thread busy.
    ///
    /// On a pool-driven queue this means the completion may sit undispatched
    /// until some *other* operation's notification fires — so use it with
    /// [`RioCq::polling`](super::cq::RioCq::polling) or an IOCP loop that
    /// drains on its own schedule.
    pub fn recv_without_notify(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.rq.post(slice, false, RIO_MSG_DONT_NOTIFY)
    }

    /// Send counterpart of
    /// [`recv_without_notify`](Self::recv_without_notify).
    pub fn send_without_notify(
        &self,
        slice: RioSlice,
    ) -> windows_core::Result<impl Future<Output = (windows_core::Result<usize>, RioSlice)> + use<>>
    {
        self.rq.post(slice, true, RIO_MSG_DONT_NOTIFY)
    }

    /// Resizes this socket's outstanding-operation budget; see
    /// `RequestQueue::resize`.
    pub fn resize_queue(&self, max_recv: u32, max_send: u32) -> windows_core::Result<()> {
        self.rq.resize(max_recv, max_send)
    }

    /// Dispatches everything queued by
    /// [`send_deferred`](Self::send_deferred) (`RIO_MSG_COMMIT_ONLY`).
    ///
    /// Microsoft documents this as the one request-queue entry point that
    /// does *not* need to be serialized with other sends — it may run on
    /// another thread concurrently with a `RIOSend` on the same queue. The
    /// socket being `!Sync` does not offer that, and it is not worth a
    /// second type: the commit is one call next to the dispatch it releases.
    pub fn commit_sends(&self) -> windows_core::Result<()> {
        self.rq.commit_sends()
    }

    fn post_ex(
        &self,
        slice: RioSlice,
        dest: Option<SocketAddr>,
    ) -> windows_core::Result<
        impl Future<Output = (windows_core::Result<(usize, SocketAddr)>, RioSlice)> + use<>,
    > {
        let (tx, rx) = oneshot::channel::<(
            windows_core::Result<(usize, SocketAddr)>,
            Option<RioSlice>,
        )>();

        // The address travels in its own registered slice, because RIO names
        // it with a RIO_BUF like any other buffer.
        let mut addr_slice = self
            .addrs
            .acquire()
            .ok_or_else(|| windows_core::Error::from_hresult(WSAENOBUFS_HRESULT))?;
        addr_slice.set_len(size_of::<SOCKADDR_INET>());
        if let Some(dest) = dest {
            let mut storage = unsafe { core::mem::zeroed::<SOCKADDR_STORAGE>() };
            write_sockaddr(&dest, &mut storage);
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    &storage as *const _ as *const u8,
                    size_of::<SOCKADDR_INET>(),
                )
            };
            addr_slice.copy_from_slice(bytes);
        } else {
            addr_slice.fill(0);
        }
        let addr_buf = addr_slice.rio_buf();
        let data_buf = slice.rio_buf();

        let finish = Finish::Addressed { tx, addr: addr_slice };
        self.rq
            .post_ex(&data_buf, &addr_buf, dest.is_some(), slice, finish)?;

        Ok(async move { resolve(rx).await })
    }

}

impl Drop for RioUdpSocket {
    /// Closing is the only way to stop outstanding operations. Their
    /// completions still arrive — with `WSAECONNABORTED` — and the completion
    /// queue stays alive to receive them because this socket holds an `Arc`
    /// of it.
    fn drop(&mut self) {
        unsafe { closesocket(self.sock) };
    }
}

fn write_sockaddr(addr: &SocketAddr, storage: &mut SOCKADDR_STORAGE) -> i32 {
    crate::common::sockaddr::write_sockaddr(addr, storage)
}

fn local_addr_of(sock: SOCKET) -> windows_core::Result<SocketAddr> {
    crate::common::sockaddr::addr_of_socket(sock)
}

#[cfg(test)]
#[path = "socket_tests.rs"]
mod tests;
