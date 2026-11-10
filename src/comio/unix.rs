//! AF_UNIX (Unix domain socket) transport: `UnixStreamListener` (`AcceptEx`)
//! and `UnixConnector` (`ConnectEx` fetched via `WSAIoctl`), producing
//! `TransportStream`s (via `stream::from_reg`) registered as
//! sockets (`is_socket = true`), so `ReadAsync`/`WriteAsync` drive
//! `WSARecv`/`WSASend`. Mirrors `src/tp/io/net.rs`'s AF_INET
//! `TcpListener`/`TcpStream`, but with `AF_UNIX`/`SOCKADDR_UN` addressing and
//! completion driven through `comio::waitop::await_io_owning` instead of
//! `tp::io`'s own future machinery. `await_io_owning` (rather than the
//! simpler `await_io`) is required here because both `AcceptEx`'s output
//! address buffer and `ConnectEx`'s input `SOCKADDR_UN` are memory the
//! kernel touches for the lifetime of the op; `await_io_owning` keeps that
//! memory alive until the op's real completion, not merely until the
//! `accept()`/`connect()` future is dropped — see its doc comment.

use crate::common::security::{PeerIdentity, SecurityDescriptor};
use crate::common::winsock::ExtFnCache;
use crate::comio::unload::ModuleRef;
use crate::common::winsock::{
    classify_wsa_overlapped, connectex, sock_check, ConnectExFn,
};
use super::bindings;
use super::bindings::Pbk20191::WinAsync::Comio::{ITransport_Impl, IUnixConnector_Impl};
use super::connector::{connect_op, ConnectSink};
use super::listener::{AcceptSink, Accepted};
use super::bindings::Pbk20191::WinAsync::Comio::ITransportStream;
use super::stream::{self, AcceptSockGuard};
use super::tpio::{self, IoHandle, IoReg, OpHandle};
use super::waitop::await_io_owning;
use core::ffi::c_void;
use std::mem;
use std::sync::Arc;
use windows::Win32::Networking::WinSock::{
    bind, closesocket, listen, setsockopt, AcceptEx, ADDRESS_FAMILY, AF_UNIX, INVALID_SOCKET,
    SOCKADDR, SOCKADDR_UN, SOCKET, SOCK_STREAM, SOL_SOCKET,
    SOMAXCONN, SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT,
};
use windows_core::{Error, Result, HRESULT};

fn new_socket() -> Result<SOCKET> {
    // Through `winsock::socket` rather than `wsa_init()` + `WSASocketW`: the
    // two-step form leaves a window for a teardown to land between them.
    crate::common::winsock::socket(AF_UNIX as i32, SOCK_STREAM.0, 0, 0)
}

/// Builds a `SOCKADDR_UN` for `path`. `sun_path` holds the NUL-terminated
/// path bytes (cast `u8` -> `i8`); the path itself must fit in the 108-byte
/// field with room for the trailing NUL, i.e. at most 107 bytes.
fn sockaddr_un(path: &str) -> Result<(SOCKADDR_UN, i32)> {
    let bytes = path.as_bytes();
    if bytes.len() > 107 {
        return Err(Error::from_hresult(
            windows::Win32::Foundation::ERROR_INVALID_PARAMETER.to_hresult(),
        ));
    }
    let mut sa = SOCKADDR_UN {
        sun_family: ADDRESS_FAMILY(AF_UNIX),
        sun_path: [0i8; 108],
    };
    for (i, &b) in bytes.iter().enumerate() {
        sa.sun_path[i] = b as i8;
    }
    // The trailing NUL is already present (zero-initialized); the address
    // length is the family field plus the path bytes plus that NUL.
    let addrlen = (size_of::<ADDRESS_FAMILY>() + bytes.len() + 1) as i32;
    Ok((sa, addrlen))
}

/// An "autobind" address: family only, no path. `ConnectEx` requires a bound
/// socket (mirrors `tp/io/net.rs` binding to `0.0.0.0:0` before `ConnectEx`);
/// for AF_UNIX the equivalent is binding with a zero-length path.
fn sockaddr_un_autobind() -> (SOCKADDR_UN, i32) {
    let sa = SOCKADDR_UN {
        sun_family: ADDRESS_FAMILY(AF_UNIX),
        sun_path: [0i8; 108],
    };
    (sa, size_of::<ADDRESS_FAMILY>() as i32)
}

/// `ConnectEx` for the **AFUNIX** provider. The signature lives in
/// `common::winsock`; only this cache is local, because the pointer belongs
/// to the provider.
fn get_connectex(sock: SOCKET) -> windows_core::Result<ConnectExFn> {
    static PTR: ExtFnCache = ExtFnCache::new();
    connectex(sock, &PTR)
}

/// AcceptEx address slot: sockaddr size + 16 bytes, per the API contract.
const ADDR_SLOT: usize = size_of::<SOCKADDR_UN>() + 16;


/// A listening AF_UNIX socket, registered with the thread pool's TP_IO so
/// `AcceptEx` can be driven through `super::waitop::await_io`.
pub struct UnixStreamListener {
    reg: Arc<IoReg>,
}

impl UnixStreamListener {
    /// Binds a new AF_UNIX socket to `path` and starts listening. AF_UNIX
    /// `bind` fails if a socket file already exists at `path`; callers that
    /// need to rebind a stale path should remove it first (as the tests do).
    ///
    /// `security`, if given, has its DACL written onto the socket file
    /// between `bind` and `listen` -- a connect before `listen` is refused
    /// anyway, so no client ever sees the inherited DACL. Without it, who may
    /// connect is decided by the DACL the file inherits from its directory.
    ///
    /// Treat that DACL as defence in depth, not as the boundary. On the
    /// development machine it refused every connect it should have; on
    /// GitHub-hosted runners it was enforced in some jobs and not in others --
    /// same image, same token, same code -- and no cause was found. A named
    /// pipe's descriptor was enforced everywhere. Where the boundary must
    /// hold, use a named pipe, or check the peer's SID at accept.
    pub fn bind(path: &str, security: Option<&SecurityDescriptor>) -> Result<Self> {
        let sock = new_socket()?;
        // Closed on any early return from here on.
        let owned = unsafe { IoHandle::from_socket(sock) };
        let register = move || -> Result<Arc<IoReg>> {
            let (sa, addrlen) = sockaddr_un(path)?;
            sock_check(unsafe { bind(sock, &sa as *const _ as *const SOCKADDR, addrlen) })?;
            if let Some(security) = security {
                security.apply_dacl_to_file(path)?;
            }
            sock_check(unsafe { listen(sock, SOMAXCONN as i32) })?;
            IoReg::register(owned)
        };
        Ok(UnixStreamListener { reg: register()? })
    }

    /// Accepts one connection, returning it as a duplex WinRT stream.
    pub async fn accept(&self) -> Result<ITransportStream> {
        let accept_sock = new_socket()?;
        let listen_sock = SOCKET(self.reg.handle.0 as usize);
        let mut guard = AcceptSockGuard(accept_sock);

        // The address buffer, in contrast to the socket above, IS kernel-
        // owned memory for the duration of the op: `AcceptEx` writes the
        // local/remote addresses into it, possibly as late as when the
        // (cancelled) op's real completion is processed. It must therefore
        // outlive not just this `.await` but the op's actual completion —
        // hence `await_io_owning` rather than a plain frame local. The
        // pointer is read out of the `Box` *before* the box is moved into
        // `await_io_owning`'s payload; that's sound because moving a `Box`
        // value never moves its heap allocation. Smuggled through as a
        // `usize` because raw pointers aren't `Send` (required by
        // `await_io_owning`'s `issue` bound).
        let mut addr_buf: Box<[u8; 2 * ADDR_SLOT]> = Box::new([0u8; 2 * ADDR_SLOT]);
        // `as_mut_ptr` (not `as_ptr`), matching the kernel writing into this
        // buffer: the `Box` is uniquely owned, so a mutable-provenance
        // pointer is both correct and required for `AcceptEx`'s write.
        let addr_ptr = addr_buf.as_mut_ptr() as usize;

        let result = await_io_owning(&self.reg, addr_buf, move |ovl| unsafe {
            let mut bytes = 0u32;
            let ok = AcceptEx(
                listen_sock,
                accept_sock,
                addr_ptr as *mut c_void,
                0, // complete on connect; do not wait for first data
                ADDR_SLOT as u32,
                ADDR_SLOT as u32,
                &mut bytes,
                ovl,
            );
            classify_wsa_overlapped(ok.as_bool())
        })
        .await;
        result?;

        // Success: take the socket out of the guard so its `Drop` no longer
        // closes it (`IoReg::register` owns it from here).
        let sock = mem::replace(&mut guard.0, INVALID_SOCKET);
        // Closed on any early return from here on.
        let owned = unsafe { IoHandle::from_socket(sock) };
        let finish = move || -> Result<ITransportStream> {
            // Until `SO_UPDATE_ACCEPT_CONTEXT` (value = the listen socket)
            // the accepted socket is half-configured.
            let listen_val = listen_sock.0.to_ne_bytes();
            sock_check(unsafe {
                setsockopt(sock, SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, Some(&listen_val))
            })?;
            let reg = IoReg::register(owned)?;
            Ok(stream::from_reg(reg, stream::Peer::Unix))
        };
        finish()
    }

    /// Callback-based counterpart to [`UnixStreamListener::accept`], used by
    /// `listener.rs`'s self-re-arming, executor-free accept chain via
    /// [`tpio::submit`] directly instead of the `Future`-based `accept()`/
    /// `await_io_owning`. Reuses `accept()`'s exact `AcceptEx` setup
    /// (fresh accept socket, `ADDR_SLOT`-sized address buffer,
    /// `SO_UPDATE_ACCEPT_CONTEXT` finish-up), but moves the address buffer
    /// *and* the accept socket into the `tpio::submit` completion closure by
    /// hand instead of going through `await_io_owning`: `tpio::submit`'s own
    /// completion-callback discipline is exactly the kernel-gated keep-alive
    /// `await_io_owning` builds on, so capturing both directly in the
    /// closure here reproduces the same soundness argument (the closure —
    /// and everything it owns — lives until the real kernel completion,
    /// whether that's a natural finish or an aborted cancellation) without
    /// going through a `Future` at all.
    ///
    /// `on_result` normally runs on a thread-pool worker thread; a
    /// synchronous `AcceptEx` failure (rare — success, even the "already
    /// connected" case, always queues a completion per
    /// `classify_wsa_overlapped`) invokes it synchronously instead, before
    /// this function returns.
    ///
    /// A synchronous failure to create the accept socket is returned as
    /// `Err` directly, without ever invoking `on_result`.
    pub(crate) fn arm_accept(
        &self,
        on_result: AcceptSink,
    ) -> Result<OpHandle> {
        let accept_sock = new_socket()?;
        let listen_sock = SOCKET(self.reg.handle.0 as usize);

        // Same kernel-owned-memory reasoning as `accept()`'s `addr_buf`:
        // moved into the completion closure below so it outlives not just
        // this call but the operation's real completion.
        let mut addr_buf: Box<[u8; 2 * ADDR_SLOT]> = Box::new([0u8; 2 * ADDR_SLOT]);
        let addr_ptr = addr_buf.as_mut_ptr() as usize;

        let op = unsafe {
            tpio::submit(
                &self.reg,
                move |ioresult: u32, _bytes: usize| {
                    // Keep the address buffer alive until this real
                    // completion (success or aborted failure) — see the
                    // doc comment above.
                    drop(addr_buf);
                    let res = if ioresult == 0 {
                        let listen_val = listen_sock.0.to_ne_bytes();
                        // Closed on any early return from here on.
                        let owned = IoHandle::from_socket(accept_sock);
                        let finish = move || -> Result<Accepted> {
                            sock_check(setsockopt(
                                accept_sock,
                                SOL_SOCKET,
                                SO_UPDATE_ACCEPT_CONTEXT,
                                Some(&listen_val),
                            ))?;
                            // Who connected, read while the socket is still
                            // ours to query (and only valid after the
                            // accept context above).
                            let peer = PeerIdentity::of_unix_peer(accept_sock);
                            let reg = IoReg::register(owned)?;
                            Ok(Accepted::local(stream::from_reg(reg, stream::Peer::Unix), peer))
                        };
                        finish()
                    } else {
                        closesocket(accept_sock);
                        Err(Error::from_hresult(HRESULT::from_win32(ioresult)))
                    };
                    on_result.deliver(res);
                },
                move |ovl| {
                    let mut bytes = 0u32;
                    let ok = AcceptEx(
                        listen_sock,
                        accept_sock,
                        addr_ptr as *mut c_void,
                        0, // complete on connect; do not wait for first data
                        ADDR_SLOT as u32,
                        ADDR_SLOT as u32,
                        &mut bytes,
                        ovl,
                    );
                    classify_wsa_overlapped(ok.as_bool())
                },
            )
        };
        Ok(op)
    }
}

/// Client side of an AF_UNIX connection.
#[windows_core::implement(
    bindings::Pbk20191::WinAsync::Comio::UnixConnector,
    bindings::Pbk20191::WinAsync::Comio::IUnixConnector
)]
pub struct UnixConnector {
    _module: ModuleRef,
}

/// The one spelling of this transport's name, as the vtable's `Transport`
/// reports it.
pub(crate) const TRANSPORT: &str = "unix";

impl UnixConnector {
    /// Connects to the AF_UNIX socket bound at `path`, returning the
    /// connection as a duplex WinRT stream.
    pub async fn connect(path: &str) -> Result<ITransportStream> {
        let sock = new_socket()?;
        // Closed on any early return from here on.
        let owned = unsafe { IoHandle::from_socket(sock) };
        let prepared = (move || -> Result<(ConnectExFn, Arc<IoReg>)> {
            // ConnectEx fails with WSAEINVAL on an unbound socket.
            let (local, local_len) = sockaddr_un_autobind();
            sock_check(unsafe { bind(sock, &local as *const _ as *const SOCKADDR, local_len) })?;
            let connect_ex = get_connectex(sock)?;
            let reg = IoReg::register(owned)?;
            Ok((connect_ex, reg))
        })();
        let (connect_ex, reg) = prepared?;

        let (remote, remote_len) = match sockaddr_un(path) {
            Ok(v) => v,
            Err(e) => return Err(e), // `reg`'s Drop closes `sock`.
        };
        // `remote` is memory `ConnectEx` reads from for the duration of the
        // op (the provider may defer reading it until the completion is
        // actually processed). Box it and gate it through
        // `await_io_owning` the same way `accept`'s address buffer is
        // gated, rather than leaving it as a plain async-fn-frame local
        // that would be freed as soon as this future is dropped (e.g. under
        // a timeout), regardless of whether the op is still in flight.
        let remote_box = Box::new(remote);
        let remote_ptr = remote_box.as_ref() as *const SOCKADDR_UN as usize;
        let result = await_io_owning(&reg, remote_box, move |ovl| unsafe {
            let mut sent = 0u32;
            let ok = connect_ex(
                sock,
                remote_ptr as *const SOCKADDR,
                remote_len,
                core::ptr::null(),
                0,
                &mut sent,
                ovl,
            );
            classify_wsa_overlapped(ok.as_bool())
        })
        .await;
        result?;
        sock_check(unsafe { setsockopt(sock, SOL_SOCKET, SO_UPDATE_CONNECT_CONTEXT, None) })?;
        Ok(stream::from_reg(reg, stream::Peer::Unix))
    }

    /// Callback-based counterpart to [`connect`](Self::connect), for the COM
    /// connector: the result is delivered from the `ConnectEx` completion
    /// instead of being awaited, so nothing blocks a thread while the
    /// connect is outstanding.
    ///
    /// Same ownership argument as `arm_accept`: the remote `SOCKADDR_UN` is
    /// memory the provider may read until the operation really completes, so
    /// it is moved into the completion closure rather than left on a frame
    /// that could be dropped first.
    pub(crate) fn arm_connect(
        path: &str,
        on_result: ConnectSink,
    ) -> Result<OpHandle> {
        let sock = new_socket()?;
        // Closed on any early return from here on.
        let owned = unsafe { IoHandle::from_socket(sock) };
        let prepared = (move || -> Result<(ConnectExFn, Arc<IoReg>)> {
            // ConnectEx fails with WSAEINVAL on an unbound socket.
            let (local, local_len) = sockaddr_un_autobind();
            sock_check(unsafe { bind(sock, &local as *const _ as *const SOCKADDR, local_len) })?;
            let connect_ex = get_connectex(sock)?;
            let reg = IoReg::register(owned)?;
            Ok((connect_ex, reg))
        })();
        let (connect_ex, reg) = prepared?;

        // `reg`'s Drop closes the socket if this fails.
        let (remote, remote_len) = sockaddr_un(path)?;
        let remote_box = Box::new(remote);
        let remote_ptr = remote_box.as_ref() as *const SOCKADDR_UN as usize;

        let reg_for_stream = reg.clone();
        let op = unsafe {
            tpio::submit(
                &reg,
                move |ioresult: u32, _bytes: usize| {
                    // Hold the address until the real completion.
                    drop(remote_box);
                    let res = if ioresult == 0 {
                        sock_check(setsockopt(sock, SOL_SOCKET, SO_UPDATE_CONNECT_CONTEXT, None))
                            .map(|()| stream::from_reg(reg_for_stream, stream::Peer::Unix))
                    } else {
                        Err(Error::from_hresult(HRESULT::from_win32(ioresult)))
                    };
                    on_result.deliver(res);
                },
                move |ovl| {
                    let mut sent = 0u32;
                    let ok = connect_ex(
                        sock,
                        remote_ptr as *const SOCKADDR,
                        remote_len,
                        core::ptr::null(),
                        0,
                        &mut sent,
                        ovl,
                    );
                    classify_wsa_overlapped(ok.as_bool())
                },
            )
        };
        Ok(op)
    }

}

impl UnixConnector {
    /// Creates the connector. It holds no state; connecting is what it does.
    #[allow(clippy::new_without_default)]
    pub fn new() -> windows_core::ComObject<UnixConnector> {
        windows_core::ComObject::new(UnixConnector {
            _module: ModuleRef::new(),
        })
    }

    /// Typed Rust mirror of the vtable's `ConnectAsync`; the result arrives
    /// from the `ConnectEx` completion, so no thread waits on it.
    pub fn connect_async(
        &self,
        path: &windows_core::HSTRING,
    ) -> windows_future::IAsyncOperation<ITransportStream> {
        let path = path.to_string();
        connect_op(move |sink| Self::arm_connect(&path, sink))
    }
}

impl ITransport_Impl for UnixConnector_Impl {
    fn Transport(&self) -> Result<windows_core::HSTRING> {
        Ok(windows_core::HSTRING::from(TRANSPORT))
    }
}

impl IUnixConnector_Impl for UnixConnector_Impl {
    fn ConnectAsync(
        &self,
        path: &windows_core::HSTRING,
    ) -> Result<windows_future::IAsyncOperation<ITransportStream>> {
        Ok(self.connect_async(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::buffer::{buffer_ptr, new_buffer};
    use crate::common::block_on;
    use std::future::{Future, IntoFuture};
    use std::task::{Context, RawWaker, RawWakerVTable, Waker};
    use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
    use windows_core::Interface;

    /// A `Waker` that does nothing when woken — sufficient for tests that
    /// only need one `poll()` to run an async fn's body up to its first
    /// `.await` (an async fn's body runs none of its code until first
    /// polled) and don't care about being woken again afterward.
    fn noop_waker() -> Waker {
        fn clone(_: *const ()) -> RawWaker {
            raw()
        }
        fn no_op(_: *const ()) {}
        fn raw() -> RawWaker {
            static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, no_op, no_op, no_op);
            RawWaker::new(std::ptr::null(), &VTABLE)
        }
        unsafe { Waker::from_raw(raw()) }
    }

    /// Polls `fut` exactly once (enough to issue the overlapped op the async
    /// fn starts with, since it runs no code until first polled) and then
    /// drops it immediately, mid-flight.
    fn issue_then_drop<F: Future>(fut: F) {
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut fut = Box::pin(fut);
        let _ = fut.as_mut().poll(&mut cx);
        drop(fut);
    }

    fn assert_no_live_ops(msg: &str) {
        use std::sync::atomic::Ordering;
        for _ in 0..500 {
            if crate::comio::tpio::LIVE_OPS.load(Ordering::Relaxed) == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            crate::comio::tpio::LIVE_OPS.load(Ordering::Relaxed),
            0,
            "{msg}"
        );
    }

    #[test]
    fn af_unix_echo() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("comio_unix_{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let p = path.to_string_lossy().into_owned();
        let sp = p.clone();
        let server = std::thread::spawn(move || {
            let l = UnixStreamListener::bind(&sp, None).unwrap();
            let s = block_on(l.accept()).unwrap();
            let input: IInputStream = s.cast().unwrap();
            let output: IOutputStream = s.cast().unwrap();
            let rbuf = new_buffer(16).unwrap();
            let got: IBuffer = block_on(
                input
                    .ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), 16, InputStreamOptions(0))
                    .unwrap()
                    .into_future(),
            )
            .unwrap();
            block_on(output.WriteAsync(&got).unwrap().into_future()).unwrap();
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let c = block_on(UnixConnector::connect(&p)).unwrap();
        let c_out: IOutputStream = c.cast().unwrap();
        let c_in: IInputStream = c.cast().unwrap();
        let wbuf = new_buffer(4).unwrap();
        unsafe {
            buffer_ptr(&wbuf.clone().into())
                .unwrap()
                .copy_from_nonoverlapping(b"ping".as_ptr(), 4);
        }
        wbuf.SetLength(4).unwrap();
        block_on(
            c_out
                .WriteAsync(&wbuf.cast::<IBuffer>().unwrap())
                .unwrap()
                .into_future(),
        )
        .unwrap();
        let rbuf = new_buffer(4).unwrap();
        let echoed: IBuffer = block_on(
            c_in.ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0))
                .unwrap()
                .into_future(),
        )
        .unwrap();
        unsafe {
            assert_eq!(
                std::slice::from_raw_parts(buffer_ptr(&echoed).unwrap(), 4),
                b"ping"
            );
        }
        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    /// Exercises the `WSARecv` path (`stream::ReadAsync`'s socket branch)
    /// with a non-trivial, multi-chunk payload, rather than `af_unix_echo`'s
    /// single 4-byte round trip. This is realistic coverage for the
    /// `WSARecv`-`lpFlags` write-after-free fix in `IoAsyncOp::start_read`
    /// (`flags` is now a kernel-gated `Box<u32>` keep-alive instead of a
    /// stack local): every chunk here drives a real overlapped `WSARecv`
    /// whose provider writes back through that pointer at completion.
    #[test]
    fn af_unix_echo_large_payload() {
        const TOTAL: usize = 5000;
        const CHUNK: usize = 1024;

        let dir = std::env::temp_dir();
        let path = dir.join(format!("comio_unix_large_{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let p = path.to_string_lossy().into_owned();
        let sp = p.clone();

        let payload: Vec<u8> = (0..TOTAL).map(|i| (i % 256) as u8).collect();
        let payload_for_server = payload.clone();

        let server = std::thread::spawn(move || {
            let l = UnixStreamListener::bind(&sp, None).unwrap();
            let s = block_on(l.accept()).unwrap();
            let input: IInputStream = s.cast().unwrap();
            let output: IOutputStream = s.cast().unwrap();

            // Read exactly TOTAL bytes. A single overlapped recv is not
            // guaranteed to fill a several-KB request in one completion, so
            // loop `ReadAsync` (each iteration a fresh `WSARecv`) until the
            // whole payload has arrived.
            let mut received = Vec::with_capacity(TOTAL);
            while received.len() < TOTAL {
                let want = (TOTAL - received.len()).min(CHUNK) as u32;
                let rbuf = new_buffer(want).unwrap();
                let got: IBuffer = block_on(
                    input
                        .ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), want, InputStreamOptions(0))
                        .unwrap()
                        .into_future(),
                )
                .unwrap();
                let len = got.Length().unwrap() as usize;
                assert!(len > 0, "peer closed early");
                unsafe {
                    let ptr = buffer_ptr(&got).unwrap();
                    received.extend_from_slice(std::slice::from_raw_parts(ptr, len));
                }
            }
            assert_eq!(received, payload_for_server);

            // Echo it back, also in chunks, exercising the `WSASend` side
            // with the same multi-chunk shape.
            let mut sent = 0usize;
            while sent < TOTAL {
                let chunk_len = (TOTAL - sent).min(CHUNK);
                let wbuf = new_buffer(chunk_len as u32).unwrap();
                let wib: IBuffer = wbuf.clone().into();
                unsafe {
                    buffer_ptr(&wib).unwrap().copy_from_nonoverlapping(
                        received[sent..sent + chunk_len].as_ptr(),
                        chunk_len,
                    );
                }
                wbuf.SetLength(chunk_len as u32).unwrap();
                let wrote = block_on(output.WriteAsync(&wib).unwrap().into_future()).unwrap();
                sent += wrote as usize;
            }
        });

        std::thread::sleep(std::time::Duration::from_millis(100));
        let c = block_on(UnixConnector::connect(&p)).unwrap();
        let c_out: IOutputStream = c.cast().unwrap();
        let c_in: IInputStream = c.cast().unwrap();

        let mut sent = 0usize;
        while sent < TOTAL {
            let chunk_len = (TOTAL - sent).min(CHUNK);
            let wbuf = new_buffer(chunk_len as u32).unwrap();
            let wib: IBuffer = wbuf.clone().into();
            unsafe {
                buffer_ptr(&wib)
                    .unwrap()
                    .copy_from_nonoverlapping(payload[sent..sent + chunk_len].as_ptr(), chunk_len);
            }
            wbuf.SetLength(chunk_len as u32).unwrap();
            let wrote = block_on(c_out.WriteAsync(&wib).unwrap().into_future()).unwrap();
            sent += wrote as usize;
        }

        let mut echoed = Vec::with_capacity(TOTAL);
        while echoed.len() < TOTAL {
            let want = (TOTAL - echoed.len()).min(CHUNK) as u32;
            let rbuf = new_buffer(want).unwrap();
            let got: IBuffer = block_on(
                c_in.ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), want, InputStreamOptions(0))
                    .unwrap()
                    .into_future(),
            )
            .unwrap();
            let len = got.Length().unwrap() as usize;
            assert!(len > 0, "server closed early");
            unsafe {
                let ptr = buffer_ptr(&got).unwrap();
                echoed.extend_from_slice(std::slice::from_raw_parts(ptr, len));
            }
        }
        assert_eq!(echoed, payload);

        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    /// Regression test for the write-after-free this change fixes:
    /// `AcceptEx`'s output address buffer used to be a plain local in
    /// `accept()`'s async fn frame, freed as soon as the future dropped —
    /// but `await_io`'s (now `await_io_owning`'s) `Drop` only issues
    /// `CancelIoEx` and returns immediately, without waiting for the real
    /// `ERROR_OPERATION_ABORTED` completion, so the kernel could still write
    /// into that freed memory afterward. Repeatedly starts `accept()` with
    /// no client ever connecting, polls it exactly once (to actually issue
    /// the `AcceptEx`), and drops it mid-flight — the same shape as a
    /// `select!`/timeout abandoning the future. With the fix, the address
    /// buffer now lives on the kernel-completion-gated `Op` block (via
    /// `await_io_owning`) instead of the async-fn frame, so it survives
    /// until the real completion regardless of the future's `Drop`. This
    /// asserts the ops don't leak (`LIVE_OPS` settles back to 0), which
    /// mirrors `tp/io/net.rs::cancel_storm_accept_and_read` and
    /// `tpio.rs::cancel_in_flight_read_does_not_leak`'s leak-detection
    /// pattern; it does not (and cannot, without a race detector) directly
    /// observe the UAF itself, so the actual safety guarantee rests on the
    /// `await_io_owning` contract (payload moved into the kernel-gated
    /// `on_complete` closure) being upheld here, not on this test alone.
    #[test]
    fn accept_cancel_storm_does_not_leak() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("comio_unix_accept_cancel_{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let p = path.to_string_lossy().into_owned();
        let listener = UnixStreamListener::bind(&p, None).unwrap();

        for _ in 0..500 {
            issue_then_drop(listener.accept());
        }
        drop(listener);

        assert_no_live_ops("op blocks leaked after accept() cancel storm");
        let _ = std::fs::remove_file(&path);
    }

    /// As `accept_cancel_storm_does_not_leak`, for `connect()`'s
    /// `ConnectEx` input `SOCKADDR_UN` (lower certainty than the accept
    /// buffer per the task brief — it's a provider *input*, so may be read
    /// synchronously rather than deferred — but gated identically for
    /// consistency with the module's invariant, so exercised identically
    /// here). No `accept()` is ever called on `listener`, so connections
    /// simply queue in the listen backlog rather than completing, giving
    /// `connect()`'s op a real window to be cancelled mid-flight.
    #[test]
    fn connect_cancel_storm_does_not_leak() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("comio_unix_connect_cancel_{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let p = path.to_string_lossy().into_owned();
        let listener = UnixStreamListener::bind(&p, None).unwrap();

        for _ in 0..200 {
            issue_then_drop(UnixConnector::connect(&p));
        }
        drop(listener);

        assert_no_live_ops("op blocks leaked after connect() cancel storm");
        let _ = std::fs::remove_file(&path);
    }
}
