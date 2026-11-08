//! AF_HYPERV ("Hyper-V socket", `vsock` to a Linux guest) addressing, shared
//! by the `tp` and `comio` transports.
//!
//! The `windows` crate exposes only the `AF_HYPERV` constant, so `SOCKADDR_HV`,
//! the protocol number, the socket options and the well-known VM GUIDs are
//! declared here from `hvsocket.h`. Addressing only — layout, naming, and the
//! two `getsockname`/`getpeername` lookups — with no runtime dependency, so
//! both overlapped runtimes build on one copy and the GUIDs cannot drift
//! apart.
//!
//! Addressing is a pair of GUIDs rather than address + port:
//!   * **VM id** — which partition ([`HV_GUID_LOOPBACK`], [`HV_GUID_PARENT`],
//!     [`HV_GUID_CHILDREN`], [`HV_GUID_WILDCARD`], or a specific VM's id).
//!   * **service id** — which endpoint inside it. For a guest to reach a host
//!     service the GUID must be registered under
//!     `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Virtualization\GuestCommunicationServices`
//!     (admin, one key per service GUID). Host-to-host loopback does not need
//!     the registration.

use super::winsock::sock_check;
use windows::Win32::Networking::WinSock::{
    getpeername, getsockname, ADDRESS_FAMILY, AF_HYPERV, SOCKADDR, SOCKET,
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

/// The tail the `AF_VSOCK` service-id template leaves fixed; the top 32 bits
/// carry the port.
const VSOCK_TEMPLATE_TAIL: u128 = 0x00000000_facb_11e6_bd58_64006a7986d3;

/// Service id of the Linux-visible `AF_VSOCK` port `port`, from the template
/// `<port>-facb-11e6-bd58-64006a7986d3`. A guest connecting to vsock port 5000
/// reaches the host service with `vsock_service_id(5000)`.
pub const fn vsock_service_id(port: u32) -> GUID {
    GUID::from_u128(((port as u128) << 96) | VSOCK_TEMPLATE_TAIL)
}

/// The inverse of [`vsock_service_id`]: the vsock port a service id encodes,
/// or `None` for a service GUID that is not from the template. Lets a host
/// service report *which* vsock port a guest dialled.
pub fn vsock_port_of(service_id: GUID) -> Option<u32> {
    let v = service_id.to_u128();
    if v & ((1u128 << 96) - 1) == VSOCK_TEMPLATE_TAIL {
        Some((v >> 96) as u32)
    } else {
        None
    }
}

/// Canonical hyphenated lowercase form, spelled out from the fields rather
/// than borrowed from `GUID`'s `Debug`, so the wire-ish string this produces
/// cannot change under us if that impl does.
fn guid_str(g: &GUID) -> String {
    let d = g.data4;
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        g.data1, g.data2, g.data3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
    )
}

/// `SOCKADDR_HV` from `hvsocket.h` — 36 bytes, two GUIDs behind the family.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SockaddrHv {
    pub family: ADDRESS_FAMILY,
    pub reserved: u16,
    pub vm_id: GUID,
    pub service_id: GUID,
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

    /// This partition talking to itself (no service registration needed).
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

    /// The wire form plus its length, for `bind` / `ConnectEx`.
    pub fn to_raw(self) -> (SockaddrHv, i32) {
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

    /// A name for the well-known VM GUIDs, else `None`. Turns an opaque GUID
    /// into something readable in a log line or an endpoint string.
    pub fn vm_name(&self) -> Option<&'static str> {
        Some(match self.vm_id {
            HV_GUID_ZERO => "wildcard",
            HV_GUID_BROADCAST => "broadcast",
            HV_GUID_CHILDREN => "children",
            HV_GUID_LOOPBACK => "loopback",
            HV_GUID_PARENT => "parent",
            HV_GUID_SILOHOST => "silohost",
            _ => return None,
        })
    }
}

/// `hv://<vm>/<service>`, with the well-known VM GUIDs and vsock-template
/// service ids spelled out: `hv://loopback/vsock:5000`.
impl core::fmt::Display for HvSocketAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("hv://")?;
        match self.vm_name() {
            Some(n) => f.write_str(n)?,
            None => f.write_str(&guid_str(&self.vm_id))?,
        }
        f.write_str("/")?;
        match vsock_port_of(self.service_id) {
            Some(p) => write!(f, "vsock:{p}"),
            None => f.write_str(&guid_str(&self.service_id)),
        }
    }
}

/// Reads a `SOCKADDR_HV` the kernel filled in. The family is not re-checked:
/// the caller only ever hands this an AF_HYPERV socket's own name.
fn addr_from_raw(raw: &SockaddrHv) -> HvSocketAddr {
    HvSocketAddr::new(raw.vm_id, raw.service_id)
}

/// The peer of a connected AF_HYPERV socket — **which partition** is on the
/// other end.
///
/// `vm_id` is the authorization primitive for a host service: a listener bound
/// to the wildcard VM id accepts from every partition, so "which guest is
/// this?" has to come from here. Valid only after `SO_UPDATE_ACCEPT_CONTEXT`
/// on an accepted socket.
///
/// **The `service_id` is not the service the client dialled.** Like a TCP
/// source port, it is the client's own ephemeral endpoint — the provider
/// generates a fresh GUID per connection for a socket that autobound, and this
/// returns exactly what [`local_addr_of`] reports on the client's side.
/// Authorize on `vm_id` alone; the service is something the server already
/// knows, because it is the one that bound it.
///
/// Note that [`local_addr_of`] on an *accepted* socket does not report the
/// bound service either — the accepted channel gets its own generated name
/// (with the VM id reading as `children`). Only the client's `peer_addr_of`
/// reproduces the dialled endpoint.
pub fn peer_addr_of(sock: SOCKET) -> windows_core::Result<HvSocketAddr> {
    let mut raw = unsafe { core::mem::zeroed::<SockaddrHv>() };
    let mut len = size_of::<SockaddrHv>() as i32;
    sock_check(unsafe { getpeername(sock, &mut raw as *mut _ as *mut SOCKADDR, &mut len) })?;
    Ok(addr_from_raw(&raw))
}

/// The local name of an AF_HYPERV socket.
pub fn local_addr_of(sock: SOCKET) -> windows_core::Result<HvSocketAddr> {
    let mut raw = unsafe { core::mem::zeroed::<SockaddrHv>() };
    let mut len = size_of::<SockaddrHv>() as i32;
    sock_check(unsafe { getsockname(sock, &mut raw as *mut _ as *mut SOCKADDR, &mut len) })?;
    Ok(addr_from_raw(&raw))
}
