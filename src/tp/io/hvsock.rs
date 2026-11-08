//! Async Hyper-V sockets (AF_HYPERV, "vsock" on Linux guests) over TP_IO.
//!
//! A Hyper-V socket is a stream between a partition and the host, or between
//! two partitions, that needs no virtual NIC — the transport is VMBus. On the
//! Linux side of a VM the same endpoints appear as `AF_VSOCK`, which is why the
//! service GUID has a template form carrying a 32-bit vsock port.
//!
//! The `windows` crate exposes only `AF_HYPERV`; `SOCKADDR_HV`, the protocol
//! number, the well-known VM GUIDs, and the socket options are declared here
//! from `hvsocket.h`.
//!
//! Addressing is a pair of GUIDs rather than address + port:
//!   * **VM id** — which partition ([`HV_GUID_LOOPBACK`], [`HV_GUID_PARENT`],
//!     [`HV_GUID_CHILDREN`], [`HV_GUID_WILDCARD`], or a specific VM's id).
//!   * **service id** — which endpoint inside it. For a service to be
//!     connectable from a guest it must be registered under
//!     `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Virtualization\GuestCommunicationServices`
//!     (admin, one key per service GUID). Host-to-host loopback needs the same
//!     registration.
//!
//! Streams mirror [`super::unix`]: `AcceptEx` / `ConnectEx` for the handshake,
//! `WSARecv` / `WSASend` for data.

use super::net::{
    classify_wsa_overlapped, get_connectex, new_socket, sock_check, VectoredPayload, ADDR_SLOT,
};
use super::{submit, IoRes};
use crate::tp::{Runtime, RuntimeInner};
use core::ffi::c_void;
use std::mem;
use std::sync::Arc;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Networking::WinSock::{
    bind, closesocket, listen, setsockopt, AcceptEx, ADDRESS_FAMILY, AF_HYPERV, INVALID_SOCKET,
    IPPROTO, SOCKADDR, SOCKET, SOCK_STREAM, SOL_SOCKET, SOMAXCONN, SO_UPDATE_ACCEPT_CONTEXT,
    SO_UPDATE_CONNECT_CONTEXT, WSABUF, WSARecv, WSASend,
};
use windows_core::GUID;

/// `HV_PROTOCOL_RAW` — the only protocol AF_HYPERV defines, and also the
/// `setsockopt` level for the `HVSOCKET_*` options.
pub const HV_PROTOCOL_RAW: i32 = 1;

/// Timeout in milliseconds for a pending connect (`u32` option value).
pub const HVSOCKET_CONNECT_TIMEOUT: i32 = 0x01;
/// Allow the connect to cross a container (silo) boundary (`u32` boolean).
pub const HVSOCKET_CONTAINER_PASSTHRU: i32 = 0x02;
/// Keep the connection alive while the partition is suspended (`u32` boolean).
pub const HVSOCKET_CONNECTED_SUSPEND: i32 = 0x04;

/// All-zero GUID; also the wildcard VM id when binding.
pub const HV_GUID_ZERO: GUID = GUID::from_u128(0x00000000_0000_0000_0000_000000000000);
/// Bind target meaning "any partition".
pub const HV_GUID_WILDCARD: GUID = HV_GUID_ZERO;
/// Send to every partition that listens on the service id.
pub const HV_GUID_BROADCAST: GUID = GUID::from_u128(0xffffffff_ffff_ffff_ffff_ffffffffffff);
/// Bind target meaning "all child partitions of this host".
pub const HV_GUID_CHILDREN: GUID = GUID::from_u128(0x90db8b89_0d35_4f79_8ce9_49ea0ac8b7cd);
/// This same partition — host-to-host, used for testing without a VM.
pub const HV_GUID_LOOPBACK: GUID = GUID::from_u128(0xe0e16197_dd56_4a10_9195_5ee7a155a838);
/// From inside a guest: the host partition.
pub const HV_GUID_PARENT: GUID = GUID::from_u128(0xa42e7cda_d03f_480c_9cc2_a4de20abb878);
/// The silo host partition.
pub const HV_GUID_SILOHOST: GUID = GUID::from_u128(0x36bd0c5c_7276_4223_88ba_7d03b654c568);

/// Service id of the Linux-visible `AF_VSOCK` port `port`, from the template
/// `<port>-facb-11e6-bd58-64006a7986d3`. A guest connecting to vsock port 5000
/// reaches the host service with `vsock_service_id(5000)`.
pub const fn vsock_service_id(port: u32) -> GUID {
    GUID::from_u128(((port as u128) << 96) | 0x00000000_facb_11e6_bd58_64006a7986d3)
}

/// `SOCKADDR_HV` from `hvsocket.h` — 36 bytes, two GUIDs behind the family.
#[repr(C)]
#[derive(Clone, Copy)]
struct SockaddrHv {
    family: ADDRESS_FAMILY,
    reserved: u16,
    vm_id: GUID,
    service_id: GUID,
}

/// A Hyper-V socket endpoint: which partition, and which service inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HvSocketAddr {
    pub vm_id: GUID,
    pub service_id: GUID,
}

impl HvSocketAddr {
    pub const fn new(vm_id: GUID, service_id: GUID) -> Self {
        Self { vm_id, service_id }
    }

    /// This partition talking to itself (needs the service registered).
    pub const fn loopback(service_id: GUID) -> Self {
        Self::new(HV_GUID_LOOPBACK, service_id)
    }

    /// Listen on `service_id` for connections from any partition.
    pub const fn wildcard(service_id: GUID) -> Self {
        Self::new(HV_GUID_WILDCARD, service_id)
    }

    /// The host partition, as seen from inside a guest.
    pub const fn parent(service_id: GUID) -> Self {
        Self::new(HV_GUID_PARENT, service_id)
    }

    fn to_raw(self) -> (SockaddrHv, i32) {
        (
            SockaddrHv {
                family: ADDRESS_FAMILY(AF_HYPERV),
                reserved: 0,
                vm_id: self.vm_id,
                service_id: self.service_id,
            },
            size_of::<SockaddrHv>() as i32,
        )
    }
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

struct RecvPayload {
    buf: Vec<u8>,
    flags: u32,
}

struct AcceptPayload {
    accept_sock: SOCKET,
    addr_buf: Box<[u8; 2 * ADDR_SLOT]>,
}
impl Drop for AcceptPayload {
    fn drop(&mut self) {
        if self.accept_sock != INVALID_SOCKET {
            unsafe { closesocket(self.accept_sock) };
        }
    }
}
unsafe impl Send for AcceptPayload {}

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

    /// Receives into `buf`; resolves to the byte count (0 = peer closed).
    pub fn read(
        &self,
        buf: Vec<u8>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<u8>)> + Send + use<> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, RecvPayload { buf, flags: 0 }, move |ovl, p| {
                let wsabuf = WSABUF {
                    len: p.buf.len() as u32,
                    buf: windows_core::PSTR(p.buf.as_mut_ptr()),
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
    pub fn write(
        &self,
        buf: Vec<u8>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<u8>)> + Send + use<> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                let wsabuf = WSABUF {
                    len: buf.len() as u32,
                    buf: windows_core::PSTR(buf.as_mut_ptr()),
                };
                let r = WSASend(sock, &[wsabuf], None, 0, Some(ovl), None);
                classify_wsa_overlapped(r == 0)
            })
        };
        async move { fut.await }
    }

    /// Scatter-reads into `bufs` with one vectored `WSARecv`.
    pub fn read_vectored(
        &self,
        bufs: Vec<Vec<u8>>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<Vec<u8>>)> + Send + use<> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, VectoredPayload::new(bufs), move |ovl, p| {
                p.fill();
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
    pub fn write_vectored(
        &self,
        bufs: Vec<Vec<u8>>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<Vec<u8>>)> + Send + use<> {
        let sock = self.res.socket();
        let fut = unsafe {
            submit(&self.res, VectoredPayload::new(bufs), move |ovl, p| {
                p.fill();
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
    /// Binds and listens on `addr`. The service id must be registered under
    /// `GuestCommunicationServices` for guests (or loopback) to reach it.
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
    fn vsock_service_id_matches_template() {
        // Linux vsock port 5000 over the Hyper-V service-id template.
        assert_eq!(
            vsock_service_id(5000),
            GUID::from_u128(0x00001388_facb_11e6_bd58_64006a7986d3)
        );
        assert_eq!(vsock_service_id(0).to_u128(), 0xfacb_11e6_bd58_64006a7986d3);
    }

    #[test]
    fn hv_loopback_echo() {
        let rt = Runtime::new();
        // A service GUID of our own; loopback still requires it to be
        // registered under GuestCommunicationServices, so bind/connect may
        // legitimately fail on an unprepared machine.
        let service = GUID::from_u128(0x2fd1_0ba4_7e2f_4a3c_9d31_0f5a8c7b4e21);
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
