//! AF_HYPERV ("Hyper-V socket", `vsock` to a Linux guest) addressing, shared
//! by the `tp` and `comio` transports.
//!
//! The `windows` crate exposes only the `AF_HYPERV` constant, so `SOCKADDR_HV`,
//! the protocol number, the socket options and the well-known VM GUIDs are
//! declared here from `hvsocket.h`. Pure data and layout — no IO, no runtime
//! dependency — so both overlapped runtimes build on one copy and the GUIDs
//! cannot drift apart.
//!
//! Addressing is a pair of GUIDs rather than address + port:
//!   * **VM id** — which partition ([`HV_GUID_LOOPBACK`], [`HV_GUID_PARENT`],
//!     [`HV_GUID_CHILDREN`], [`HV_GUID_WILDCARD`], or a specific VM's id).
//!   * **service id** — which endpoint inside it. For a guest to reach a host
//!     service the GUID must be registered under
//!     `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Virtualization\GuestCommunicationServices`
//!     (admin, one key per service GUID). Host-to-host loopback does not need
//!     the registration.

use windows::Win32::Networking::WinSock::{ADDRESS_FAMILY, AF_HYPERV};
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
}
