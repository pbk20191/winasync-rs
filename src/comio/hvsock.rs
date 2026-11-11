//! Hyper-V socket (AF_HYPERV) transport: `HvStreamListener` (`AcceptEx`) and
//! `HyperVConnector` (`ConnectEx` fetched via `WSAIoctl`), producing
//! `TransportStream`s (via `stream::from_reg`) registered as sockets
//! (`is_socket = true`), so `ReadAsync`/`WriteAsync` drive `WSARecv`/`WSASend`.
//!
//! This is a **pillar-3 gap filler**, not a WinRT duplicate: `Windows.Networking.Sockets`
//! has no Hyper-V socket type, and `HostName`/`HostNameType`
//! (`DomainName | Ipv4 | Ipv6 | Bluetooth`) cannot express a VM-GUID +
//! service-GUID endpoint at all, so `StreamSocket`/`StreamSocketListener`
//! could not reach this transport even in principle.
//!
//! Structurally this mirrors [`super::unix`] exactly — same `AcceptEx` /
//! `ConnectEx` shape, same `await_io_owning` discipline for the kernel-owned
//! address memory, same `arm_accept` callback form for `listener.rs`'s
//! executor-free accept chain. Only the address family and the sockaddr
//! differ; see [`crate::common::hvaddr`] for the addressing, shared with
//! `tp`'s Hyper-V transport.

use crate::common::winsock::ExtFnCache;
use crate::comio::unload::ModuleRef;
use crate::common::winsock::{
    classify_wsa_overlapped, connectex, sock_check, ConnectExFn,
};
use super::bindings;
use super::bindings::Pbk20191::WinAsync::Comio::{IHyperVConnector_Impl, ITransport_Impl};
use super::connector::{connect_op, ConnectSink};
use super::listener::{AcceptSink, Accepted};
use super::bindings::Pbk20191::WinAsync::Comio::ITransportStream;
use super::stream::{self, AcceptSockGuard};
use super::tpio::{self, IoHandle, IoReg, OpHandle};
use super::waitop::await_io_owning;
use crate::common::hvaddr::{
    peer_addr_of, HvSocketAddr, SockaddrHv, HV_GUID_ZERO, HV_PROTOCOL_RAW,
};
use core::ffi::c_void;
use std::mem;
use std::sync::Arc;
use windows::Win32::Networking::WinSock::{
    bind, closesocket, listen, setsockopt, AcceptEx, AF_HYPERV, INVALID_SOCKET,
    SOCKADDR, SOCKET, SOCK_STREAM, SOL_SOCKET,
    SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT,
};
use windows_core::{Error, HSTRING, Result, HRESULT};

/// Creates an AF_HYPERV stream socket. Fails with `WSAEAFNOSUPPORT` (10047)
/// where the Hyper-V socket provider is not present.
fn new_socket() -> Result<SOCKET> {
    // See `unix::new_socket` for why this is not `wsa_init()` + `WSASocketW`.
    crate::common::winsock::socket(AF_HYPERV as i32, SOCK_STREAM.0, HV_PROTOCOL_RAW, 0)
}

/// `ConnectEx` for the **HvSocket** provider. The signature lives in
/// `common::winsock`; only this cache is local, because the pointer belongs
/// to the provider.
fn get_connectex(sock: SOCKET) -> windows_core::Result<ConnectExFn> {
    static PTR: ExtFnCache = ExtFnCache::new();
    connectex(sock, &PTR)
}

/// AcceptEx address slot: sockaddr size + 16 bytes, per the API contract.
const ADDR_SLOT: usize = size_of::<SockaddrHv>() + 16;


/// A listening Hyper-V socket, registered with the thread pool's TP_IO so
/// `AcceptEx` can be driven through `await_io_owning`.
pub struct HvStreamListener {
    reg: Arc<IoReg>,
}

impl HvStreamListener {
    /// Binds a new AF_HYPERV socket to `addr` and starts listening.
    ///
    /// Bind the VM id to `HV_GUID_WILDCARD` to accept from any partition. For
    /// a **guest** to reach the service, its GUID must first be registered on
    /// the host — see the registration section in [`crate::common::hvaddr`].
    /// Host-to-host loopback works without it, so a listener that answers
    /// locally may still be unreachable from a VM.
    pub fn bind(addr: HvSocketAddr) -> Result<Self> {
        let sock = new_socket()?;
        // Closed on any early return from here on.
        let owned = unsafe { IoHandle::from_socket(sock) };
        let register = move || -> Result<Arc<IoReg>> {
            let (sa, addrlen) = addr.to_raw();
            sock_check(unsafe { bind(sock, &sa as *const _ as *const SOCKADDR, addrlen) })?;
            sock_check(unsafe { listen(sock, crate::common::winsock::LISTEN_BACKLOG) })?;
            IoReg::register(owned)
        };
        Ok(HvStreamListener { reg: register()? })
    }

    /// Accepts one connection, returning it as a duplex WinRT stream.
    pub async fn accept(&self) -> Result<ITransportStream> {
        let accept_sock = new_socket()?;
        let listen_sock = SOCKET(self.reg.handle.0 as usize);
        let mut guard = AcceptSockGuard(accept_sock);

        // The address buffer is kernel-owned memory for the op's duration (see
        // `super::unix::accept`'s long-form note): it must outlive not just
        // this `.await` but the op's real completion, hence `await_io_owning`
        // and the `usize`-smuggled pointer.
        let mut addr_buf: Box<[u8; 2 * ADDR_SLOT]> = Box::new([0u8; 2 * ADDR_SLOT]);
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

        let sock = mem::replace(&mut guard.0, INVALID_SOCKET);
        // Closed on any early return from here on.
        let owned = unsafe { IoHandle::from_socket(sock) };
        let finish = move || -> Result<ITransportStream> {
            let listen_val = listen_sock.0.to_ne_bytes();
            sock_check(unsafe {
                setsockopt(sock, SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, Some(&listen_val))
            })?;
            let reg = IoReg::register(owned)?;
            Ok(stream::from_reg(reg, stream::Peer::Opaque))
        };
        finish()
    }

    /// Callback-based counterpart to [`accept`](Self::accept), used by
    /// `listener.rs`'s self-re-arming, executor-free accept chain. Identical in
    /// structure to `super::unix::UnixStreamListener::arm_accept`, including
    /// the hand-rolled move of the address buffer and accept socket into the
    /// completion closure so both live until the real kernel completion.
    pub(crate) fn arm_accept(
        &self,
        on_result: AcceptSink,
    ) -> Result<OpHandle> {
        let accept_sock = new_socket()?;
        let listen_sock = SOCKET(self.reg.handle.0 as usize);

        let mut addr_buf: Box<[u8; 2 * ADDR_SLOT]> = Box::new([0u8; 2 * ADDR_SLOT]);
        let addr_ptr = addr_buf.as_mut_ptr() as usize;

        let op = unsafe {
            tpio::submit(
                &self.reg,
                move |ioresult: u32, _bytes: usize| {
                    // Keep the address buffer alive until this real completion.
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
                            // Read the peer *before* the socket is handed to
                            // the stream: this is the one chance to learn
                            // which partition connected, and the listener is
                            // typically bound to the wildcard VM id, so
                            // nothing else carries that. Valid only after
                            // SO_UPDATE_ACCEPT_CONTEXT above.
                            let remote = peer_addr_of(accept_sock).ok();
                            let reg = IoReg::register(owned)?;
                            Ok(Accepted {
                                stream: stream::from_reg(reg, stream::Peer::Opaque),
                                remote: remote.map(|a| HSTRING::from(a.to_string())),
                                // Neither a process nor an account crosses
                                // VMBus; the partition is the identity.
                                peer: Default::default(),
                            })
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
                        0,
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

/// Client side of a Hyper-V socket connection.
#[winasync_implement::implement(
    bindings::Pbk20191::WinAsync::Comio::HyperVConnector,
    bindings::Pbk20191::WinAsync::Comio::IHyperVConnector
)]
pub struct HyperVConnector {
    _module: ModuleRef,
}

/// The one spelling of this transport's name, as the vtable's `Transport`
/// reports it.
pub(crate) const TRANSPORT: &str = "hyperv";

impl HyperVConnector {
    /// Connects to `addr`, returning the connection as a duplex WinRT stream.
    pub async fn connect(addr: HvSocketAddr) -> Result<ITransportStream> {
        let sock = new_socket()?;
        // Closed on any early return from here on.
        let owned = unsafe { IoHandle::from_socket(sock) };
        let prepared = (move || -> Result<(ConnectExFn, Arc<IoReg>)> {
            // ConnectEx fails with WSAEINVAL on an unbound socket; the
            // AF_HYPERV equivalent of an autobind is the all-zero endpoint.
            let (local, local_len) = HvSocketAddr::new(HV_GUID_ZERO, HV_GUID_ZERO).to_raw();
            sock_check(unsafe { bind(sock, &local as *const _ as *const SOCKADDR, local_len) })?;
            let connect_ex = get_connectex(sock)?;
            let reg = IoReg::register(owned)?;
            Ok((connect_ex, reg))
        })();
        let (connect_ex, reg) = prepared?;

        let (remote, remote_len) = addr.to_raw();
        // `remote` is memory ConnectEx reads for the op's duration — boxed and
        // gated through `await_io_owning` exactly as in `super::unix::connect`,
        // so a dropped future (e.g. under a timeout) cannot free it while the
        // op is still live.
        let remote_box = Box::new(remote);
        let remote_ptr = remote_box.as_ref() as *const SockaddrHv as usize;
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
        Ok(stream::from_reg(reg, stream::Peer::Opaque))
    }

    /// Callback-based counterpart to [`connect`](Self::connect), for the COM
    /// connector: the result is delivered from the `ConnectEx` completion
    /// instead of being awaited, so nothing blocks a thread while the
    /// connect is outstanding.
    ///
    /// Same ownership argument as `arm_accept`: the remote `SockaddrHv` is
    /// memory the provider may read until the operation really completes, so
    /// it is moved into the completion closure rather than left on a frame
    /// that could be dropped first.
    pub(crate) fn arm_connect(
        addr: HvSocketAddr,
        on_result: ConnectSink,
    ) -> Result<OpHandle> {
        let sock = new_socket()?;
        // Closed on any early return from here on.
        let owned = unsafe { IoHandle::from_socket(sock) };
        let prepared = (move || -> Result<(ConnectExFn, Arc<IoReg>)> {
            // ConnectEx fails with WSAEINVAL on an unbound socket.
            let (local, local_len) = HvSocketAddr::new(HV_GUID_ZERO, HV_GUID_ZERO).to_raw();
            sock_check(unsafe { bind(sock, &local as *const _ as *const SOCKADDR, local_len) })?;
            let connect_ex = get_connectex(sock)?;
            let reg = IoReg::register(owned)?;
            Ok((connect_ex, reg))
        })();
        let (connect_ex, reg) = prepared?;

        // `reg`'s Drop closes the socket if this fails.
        let (remote, remote_len) = addr.to_raw();
        let remote_box = Box::new(remote);
        let remote_ptr = remote_box.as_ref() as *const SockaddrHv as usize;

        let reg_for_stream = reg.clone();
        let op = unsafe {
            tpio::submit(
                &reg,
                move |ioresult: u32, _bytes: usize| {
                    // Hold the address until the real completion.
                    drop(remote_box);
                    let res = if ioresult == 0 {
                        sock_check(setsockopt(sock, SOL_SOCKET, SO_UPDATE_CONNECT_CONTEXT, None))
                            .map(|()| stream::from_reg(reg_for_stream, stream::Peer::Opaque))
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

impl HyperVConnector {
    /// Creates the connector. It holds no state; connecting is what it does.
    #[allow(clippy::new_without_default)]
    pub fn new() -> windows_core::ComObject<HyperVConnector> {
        windows_core::ComObject::new(HyperVConnector {
            _module: ModuleRef::new(),
        })
    }

    /// Typed Rust mirror of the vtable's `ConnectAsync`; the result arrives
    /// from the `ConnectEx` completion, so no thread waits on it.
    ///
    /// A connect to a partition that is not listening can sit for the
    /// provider's own default. The Rust `connect` takes a
    /// `connect_timeout_ms` for that; this entry point does not expose one.
    pub fn connect_async(
        &self,
        vm_id: &windows_core::GUID,
        service_id: &windows_core::GUID,
    ) -> windows_future::IAsyncOperation<ITransportStream> {
        let addr = HvSocketAddr::new(*vm_id, *service_id);
        connect_op(move |sink| Self::arm_connect(addr, sink))
    }
}

impl ITransport_Impl for HyperVConnector_Impl {
    fn Transport(&self) -> Result<HSTRING> {
        Ok(HSTRING::from(TRANSPORT))
    }
}

#[allow(non_snake_case)]
impl IHyperVConnector_Impl for HyperVConnector_Impl {
    /// The string form the metadata declares; see the RDL for why. Rust
    /// callers use [`HyperVConnector::connect_async`], which keeps the GUIDs.
    fn ConnectAsync(
        &self,
        endpoint: &HSTRING,
    ) -> Result<windows_future::IAsyncOperation<ITransportStream>> {
        let addr: HvSocketAddr = endpoint
            .to_string()
            .parse()
            .map_err(|e: crate::common::hvaddr::HvAddrParseError| {
                windows_core::Error::new(windows::Win32::Foundation::E_INVALIDARG, e.to_string())
            })?;
        Ok(self.connect_async(&addr.vm_id, &addr.service_id))
    }
}

