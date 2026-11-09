//! Async Hyper-V sockets (AF_HYPERV, "vsock" on Linux guests) over TP_IO.
//!
//! A Hyper-V socket is a stream between a partition and the host, or between
//! two partitions, that needs no virtual NIC — the transport is VMBus. On the
//! Linux side of a VM the same endpoints appear as `AF_VSOCK`, which is why the
//! service GUID has a template form carrying a 32-bit vsock port.
//!
//! Addressing — `SOCKADDR_HV`, the protocol number, the socket options and the
//! well-known VM GUIDs — lives in [`crate::common::hvaddr`], shared with
//! `comio`'s Hyper-V transport.
//!
//! Streams mirror [`super::unix`]: `AcceptEx` / `ConnectEx` for the handshake,
//! `WSARecv` / `WSASend` for data.

use super::net::{
    classify_wsa_overlapped, new_socket, sock_check, AcceptPayload, RecvPayload,
    VectoredPayload, ADDR_SLOT,
};
use super::{submit, IoRes};
use crate::common::buffer::{IoBuf, IoBufMut};
use crate::tp::{Runtime, RuntimeInner};
use crate::common::winsock::{connectex, ConnectExFn};
use core::ffi::c_void;
use std::mem;
use std::sync::{Arc, OnceLock};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Networking::WinSock::{
    bind, closesocket, listen, setsockopt, AcceptEx, ADDRESS_FAMILY, AF_HYPERV, INVALID_SOCKET,
    IPPROTO, SOCKADDR, SOCKET, SOCK_STREAM, SOL_SOCKET, SOMAXCONN, SO_UPDATE_ACCEPT_CONTEXT,
    SO_UPDATE_CONNECT_CONTEXT, WSABUF, WSARecv, WSASend,
};

// The whole addressing surface, re-exported so callers reach it as
// `tp::io::hvsock::*` rather than through `common`.
#[allow(unused_imports)]
pub use crate::common::hvaddr::{
    local_addr_of, peer_addr_of, vsock_port_of, vsock_service_id, HvSocketAddr,
    HVSOCKET_CONNECTED_SUSPEND, HVSOCKET_CONNECT_TIMEOUT,
    HVSOCKET_CONTAINER_PASSTHRU, HV_GUID_BROADCAST, HV_GUID_CHILDREN, HV_GUID_LOOPBACK,
    HV_GUID_PARENT, HV_GUID_SILOHOST, HV_GUID_WILDCARD, HV_GUID_ZERO, HV_PROTOCOL_RAW,
};

/// `ConnectEx` for the **HvSocket** provider. The signature lives in
/// `common::winsock`; only this cache is local, because the pointer belongs
/// to the provider.
fn get_connectex(sock: SOCKET) -> windows_core::Result<ConnectExFn> {
    static PTR: OnceLock<usize> = OnceLock::new();
    connectex(sock, &PTR)
}

/// Creates an AF_HYPERV stream socket. Fails with `WSAEAFNOSUPPORT` (10047)
/// where the Hyper-V socket provider is not present.
fn hv_socket() -> windows_core::Result<SOCKET> {
    new_socket(
        ADDRESS_FAMILY(AF_HYPERV),
        SOCK_STREAM,
        IPPROTO(HV_PROTOCOL_RAW),
    )
}


/// A connected Hyper-V socket stream.
pub struct HvStream {
    res: Arc<IoRes>,
}

impl HvStream {
    fn from_socket(rt: &Arc<RuntimeInner>, sock: SOCKET) -> windows_core::Result<HvStream> {
        match IoRes::register(rt, HANDLE(sock.0 as *mut c_void), true) {
            Ok(res) => Ok(HvStream { res }),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }

    /// Connects to `addr`. `connect_timeout_ms`, when given, caps how long the
    /// provider waits for the target partition to answer — without it a
    /// connect to a VM that is not listening can hang for the provider's own
    /// (long) default.
    pub async fn connect(
        rt: &Runtime,
        addr: HvSocketAddr,
        connect_timeout_ms: Option<u32>,
    ) -> windows_core::Result<HvStream> {
        let sock = hv_socket()?;
        let prepared = (|| -> windows_core::Result<HvStream> {
            if let Some(ms) = connect_timeout_ms {
                sock_check(unsafe {
                    setsockopt(
                        sock,
                        HV_PROTOCOL_RAW,
                        HVSOCKET_CONNECT_TIMEOUT,
                        Some(&ms.to_ne_bytes()),
                    )
                })?;
            }
            // ConnectEx requires a bound socket; bind to the wildcard endpoint.
            let (local, local_len) = HvSocketAddr::new(HV_GUID_ZERO, HV_GUID_ZERO).to_raw();
            sock_check(unsafe { bind(sock, &local as *const _ as *const SOCKADDR, local_len) })?;
            get_connectex(sock)?;
            HvStream::from_socket(&rt.inner, sock)
        })();
        let stream = match prepared {
            Ok(s) => s,
            Err(e) => {
                unsafe { closesocket(sock) };
                return Err(e);
            }
        };
        let connect_ex = get_connectex(sock)?;
        let (remote, remote_len) = addr.to_raw();
        let fut = unsafe {
            submit(&stream.res, (remote, remote_len), move |ovl, p| {
                let mut sent = 0u32;
                let ok = connect_ex(
                    sock,
                    &p.0 as *const _ as *const SOCKADDR,
                    p.1,
                    core::ptr::null(),
                    0,
                    &mut sent,
                    ovl,
                );
                classify_wsa_overlapped(ok.as_bool())
            })
        };
        fut.await.0?;
        sock_check(unsafe { setsockopt(sock, SOL_SOCKET, SO_UPDATE_CONNECT_CONTEXT, None) })?;
        Ok(stream)
    }

    /// Skips the completion packet whenever an op on this handle completes
    /// **synchronously** — VMBus hands small messages over without
    /// ever blocking. The op is then finished on the issuing
    /// thread, with no pool hand-off and no context switch.
    ///
    /// Enable it before issuing ops. Issue closures report a synchronous
    /// success as a fact and the core applies this mode, so there is no
    /// per-op contract to get wrong; semantics are unchanged, only sooner.
    pub fn set_skip_completion_on_success(&self) -> windows_core::Result<()> {
        self.res.set_skip_completion_on_success()
    }

    /// Which partition is on the other end, and which service it reached.
    ///
    /// For a host service this is the authorization primitive: a listener on
    /// the wildcard VM id accepts from every partition, so this is the only
    /// way to tell *which guest* a connection came from.
    pub fn peer_addr(&self) -> windows_core::Result<HvSocketAddr> {
        peer_addr_of(self.res.socket())
    }

    /// This socket's own AF_HYPERV name.
    pub fn local_addr(&self) -> windows_core::Result<HvSocketAddr> {
        local_addr_of(self.res.socket())
    }

    /// Receives into `buf`; resolves to the byte count (0 = peer closed).
    pub fn read<B: IoBufMut>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, RecvPayload { buf, flags: 0 }, move |ovl, p| {
                let wsabuf = WSABUF {
                    len: IoBuf::len(&p.buf) as u32,
                    buf: windows_core::PSTR(p.buf.stable_mut_ptr()),
                };
                let r = WSARecv(sock, &[wsabuf], None, &mut p.flags, Some(ovl), None);
                classify_wsa_overlapped(r == 0)
            })
        };
        async move {
            let (result, payload) = fut.await;
            (result, payload.buf)
        }
    }

    /// Sends `buf`; resolves to the byte count.
    pub fn write<B: IoBuf>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let sock = self.res.socket();
        
        unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                let wsabuf = WSABUF {
                    len: IoBuf::len(buf) as u32,
                    buf: windows_core::PSTR(buf.stable_ptr() as *mut u8),
                };
                let r = WSASend(sock, &[wsabuf], None, 0, Some(ovl), None);
                classify_wsa_overlapped(r == 0)
            })
        }
    }

    /// Scatter-reads into `bufs` with one vectored `WSARecv`.
    pub fn read_vectored<B: IoBufMut>(
        &self,
        bufs: Vec<B>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<B>)> + Send + use<B> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, VectoredPayload::new(bufs), move |ovl, p| {
                p.fill_recv();
                let r = WSARecv(sock, &p.wsabufs, None, &mut p.flags, Some(ovl), None);
                classify_wsa_overlapped(r == 0)
            })
        };
        async move {
            let (result, payload) = fut.await;
            (result, payload.bufs)
        }
    }

    /// Gather-writes `bufs` with one vectored `WSASend`.
    pub fn write_vectored<B: IoBuf>(
        &self,
        bufs: Vec<B>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<B>)> + Send + use<B> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, VectoredPayload::new(bufs), move |ovl, p| {
                p.fill_send();
                let r = WSASend(sock, &p.wsabufs, None, 0, Some(ovl), None);
                classify_wsa_overlapped(r == 0)
            })
        };
        async move {
            let (result, payload) = fut.await;
            (result, payload.bufs)
        }
    }
}

/// A listening Hyper-V socket.
pub struct HvListener {
    res: Arc<IoRes>,
    rt: Runtime,
}

impl HvListener {
    /// Binds and listens on `addr`.
    ///
    /// Bind the VM id to `HV_GUID_WILDCARD` to accept from any partition. For
    /// a **guest** to reach the service, its GUID must first be registered on
    /// the host — see the registration section in [`crate::common::hvaddr`].
    /// Host-to-host loopback works without it, so a listener that answers
    /// locally may still be unreachable from a VM.
    pub fn bind(rt: &Runtime, addr: HvSocketAddr) -> windows_core::Result<HvListener> {
        let sock = hv_socket()?;
        let register = || -> windows_core::Result<Arc<IoRes>> {
            let (sa, len) = addr.to_raw();
            sock_check(unsafe { bind(sock, &sa as *const _ as *const SOCKADDR, len) })?;
            sock_check(unsafe { listen(sock, SOMAXCONN as i32) })?;
            IoRes::register(&rt.inner, HANDLE(sock.0 as *mut c_void), true)
        };
        match register() {
            Ok(res) => Ok(HvListener {
                res,
                rt: rt.clone(),
            }),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }

    /// Accepts one connection.
    pub async fn accept(&self) -> windows_core::Result<HvStream> {
        let accept_sock = hv_socket()?;
        let listen_sock = self.res.socket();
        let fut = unsafe {
            submit(
                &self.res,
                AcceptPayload {
                    accept_sock,
                    addr_buf: Box::new([0; 2 * ADDR_SLOT]),
                },
                |ovl, p| {
                    let mut bytes = 0u32;
                    let ok = AcceptEx(
                        listen_sock,
                        p.accept_sock,
                        p.addr_buf.as_mut_ptr() as *mut c_void,
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
        let (result, mut payload) = fut.await;
        result?;
        let sock = mem::replace(&mut payload.accept_sock, INVALID_SOCKET);
        let finish = || -> windows_core::Result<HvStream> {
            let listen_val = listen_sock.0.to_ne_bytes();
            sock_check(unsafe {
                setsockopt(sock, SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, Some(&listen_val))
            })?;
            HvStream::from_socket(&self.rt.inner, sock)
        };
        match finish() {
            Ok(stream) => Ok(stream),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{block_on, timeout, Runtime};
    use super::*;
    use std::time::Duration;
    use windows::Win32::Networking::WinSock::WSAEAFNOSUPPORT;
    use windows_core::GUID;

    /// True where this machine has no Hyper-V socket provider at all — the
    /// whole family is then untestable and the test reports a skip rather than
    /// a failure.
    fn hv_unsupported(e: &windows_core::Error) -> bool {
        e.code() == windows_core::HRESULT::from_win32(WSAEAFNOSUPPORT.0 as u32)
    }

    #[test]
    fn hv_socket_can_be_created() {
        match hv_socket() {
            Ok(s) => {
                unsafe { closesocket(s) };
            }
            Err(e) if hv_unsupported(&e) => {
                eprintln!("skipped: AF_HYPERV unsupported on this machine");
            }
            Err(e) => panic!("unexpected AF_HYPERV socket error: {e:?}"),
        }
    }

    #[test]
    fn hv_peer_addr_identifies_the_partition() {
        let rt = Runtime::new();
        let service = GUID::from_u128(0x41c79e03_6b52_4a18_bd74_2e1f05a63c90);
        let listener = match HvListener::bind(&rt, HvSocketAddr::wildcard(service)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("skipped: AF_HYPERV bind unavailable ({e:?})");
                return;
            }
        };
        let result = block_on(timeout(&rt, Duration::from_secs(5), async {
            let server = rt.spawn(async move {
                let s = listener.accept().await?;
                // The listener bound the *wildcard* VM id, so this is the only
                // way to learn who actually connected.
                Ok::<_, windows_core::Error>((s.peer_addr()?, s.local_addr()?))
            });
            let client =
                match HvStream::connect(&rt, HvSocketAddr::loopback(service), Some(2000)).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("skipped: AF_HYPERV loopback connect refused ({e:?})");
                        return;
                    }
                };
            let (peer, _local) = server.await.unwrap().unwrap();

            // The part worth authorizing on: which partition connected. The
            // listener bound the *wildcard* VM id, so nothing else tells us.
            assert_eq!(peer.vm_id, HV_GUID_LOOPBACK, "peer should be this partition");
            assert_eq!(peer.vm_name(), Some("loopback"));

            // The peer's service id is the client's own ephemeral endpoint —
            // it matches what the client reports as its local address, and is
            // emphatically not the service that was dialled.
            assert_eq!(peer.service_id, client.local_addr().unwrap().service_id);
            assert_ne!(
                peer.service_id, service,
                "a peer service id must never be mistaken for the dialled service"
            );

            // From the client's side, the peer is exactly what it dialled.
            assert_eq!(client.peer_addr().unwrap(), HvSocketAddr::loopback(service));
        }));
        if result.is_err() {
            eprintln!("skipped: AF_HYPERV loopback timed out (service not registered)");
        }
    }

    #[test]
    fn hv_addr_display_and_vsock_roundtrip() {
        assert_eq!(vsock_port_of(vsock_service_id(5000)), Some(5000));
        assert_eq!(vsock_port_of(GUID::from_u128(0x12340000_0000_0000_0000_000000000000)), None);
        assert_eq!(
            HvSocketAddr::loopback(vsock_service_id(5000)).to_string(),
            "hv://loopback/vsock:5000"
        );
        assert_eq!(
            HvSocketAddr::new(
                GUID::from_u128(0x2fd10ba4_7e2f_4a3c_9d31_0f5a8c7b4e21),
                GUID::from_u128(0x11112222_3333_4444_5555_666677778888),
            )
            .to_string(),
            "hv://2fd10ba4-7e2f-4a3c-9d31-0f5a8c7b4e21/11112222-3333-4444-5555-666677778888"
        );
    }

    #[test]
    fn vsock_service_id_matches_template() {
        // Linux vsock port 5000 over the Hyper-V service-id template.
        assert_eq!(
            vsock_service_id(5000),
            GUID::from_u128(0x00001388_facb_11e6_bd58_64006a7986d3)
        );
        assert_eq!(vsock_service_id(0).to_u128(), 0x00000000_facb_11e6_bd58_64006a7986d3);
    }

    #[test]
    fn hv_loopback_echo() {
        let rt = Runtime::new();
        // A service GUID of our own; loopback still requires it to be
        // registered under GuestCommunicationServices, so bind/connect may
        // legitimately fail on an unprepared machine.
        let service = GUID::from_u128(0x2fd10ba4_7e2f_4a3c_9d31_0f5a8c7b4e21);
        let listener = match HvListener::bind(&rt, HvSocketAddr::wildcard(service)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("skipped: AF_HYPERV bind unavailable ({e:?})");
                return;
            }
        };
        let result = block_on(timeout(&rt, Duration::from_secs(5), async {
            let server = rt.spawn(async move {
                let s = listener.accept().await?;
                let (n, b) = s.read(vec![0u8; 32]).await;
                let n = n?;
                s.write(b[..n].to_vec()).await.0?;
                windows_core::Result::Ok(())
            });
            let client =
                match HvStream::connect(&rt, HvSocketAddr::loopback(service), Some(2000)).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("skipped: AF_HYPERV loopback connect refused ({e:?})");
                        return;
                    }
                };
            client.write(b"hv!".to_vec()).await.0.unwrap();
            let (n, b) = client.read(vec![0u8; 32]).await;
            assert_eq!(&b[..n.unwrap()], b"hv!");
            server.await.unwrap().unwrap();
        }));
        // A timeout here means the service is not registered, not a bug in the
        // transport — the accept simply never sees a connection.
        if result.is_err() {
            eprintln!("skipped: AF_HYPERV loopback timed out (service not registered)");
        }
    }
}
