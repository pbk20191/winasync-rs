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
//!   * **service id** — which endpoint inside it.
//!
//! # Registering a host service so guests can reach it
//!
//! Binding a service id is not enough to make it reachable **from a guest**.
//! Only service GUIDs registered on the host cross VMBus, so the failure looks
//! like this: `bind` and `listen` succeed, the host can reach itself over
//! [`HV_GUID_LOOPBACK`], and a guest's `connect` fails anyway. Host-to-host
//! loopback needs no registration, which is exactly why a test suite can pass
//! while guests cannot connect.
//!
//! The registration is one registry key per service, under
//! `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Virtualization\GuestCommunicationServices`:
//!
//! ```text
//! GuestCommunicationServices\
//!     <service-guid>\            key name = the service id, hyphenated, no braces
//!         ElementName  REG_SZ    a display name for the service
//! ```
//!
//! Writing under `HKLM` needs **administrator rights**, and the key persists
//! until something removes it — it is machine provisioning, not something a
//! socket library does at run time, so this crate deliberately ships no helper
//! for it. Do it from your installer, your configuration management, or by
//! hand:
//!
//! ```powershell
//! $svc = '5a83c7e1-9b04-4d26-8f17-63ca0e2b95d8'
//! $key = "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Virtualization\GuestCommunicationServices\$svc"
//! New-Item -Path $key -Force | Out-Null
//! Set-ItemProperty -Path $key -Name ElementName -Value 'my service'
//! ```
//!
//! For a **Linux guest** connecting over `AF_VSOCK`, register the GUID that
//! [`vsock_service_id`] builds for the port — a guest dialling vsock port 5000
//! reaches the service registered as
//! `00001388-facb-11e6-bd58-64006a7986d3`. The guest side needs no
//! registration of its own; this is host-side configuration only.

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
/// The exact inverse of [`Display`](HvSocketAddr#impl-Display), so an
/// endpoint can make the round trip through a string.
///
/// That matters because a string is the only shape this endpoint can take
/// across a WinRT projection today: PyWinRT's ABI call shape census has no
/// callback for a `(GUID, GUID) -> pointer` member, so the two-GUID form of
/// `BindEndpointAsync`/`ConnectAsync` projects but raises when called. The
/// string form also reads better from a script than two raw GUIDs would.
///
/// Both halves accept either the spelled-out name or a plain GUID, so a
/// caller is never *forced* through the sugar:
///
/// ```text
/// hv://loopback/vsock:5000
/// hv://wildcard/00000000-0000-0000-0000-000000000000
/// hv://9b6a1e00-1234-5678-9abc-def012345678/vsock:22
/// ```
impl core::str::FromStr for HvSocketAddr {
    type Err = HvAddrParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s.strip_prefix("hv://").ok_or(HvAddrParseError)?;
        // `split_once` rather than `split('/')`: a GUID contains no slash, so
        // the first one is always the separator, and a trailing slash in the
        // service half should fail rather than be silently trimmed.
        let (vm, service) = rest.split_once('/').ok_or(HvAddrParseError)?;

        let vm_id = match vm {
            "wildcard" => HV_GUID_WILDCARD,
            "broadcast" => HV_GUID_BROADCAST,
            "children" => HV_GUID_CHILDREN,
            "loopback" => HV_GUID_LOOPBACK,
            "parent" => HV_GUID_PARENT,
            "silohost" => HV_GUID_SILOHOST,
            other => GUID::try_from(other).map_err(|_| HvAddrParseError)?,
        };

        Ok(Self::new(vm_id, service_id_from_str(service)?))
    }
}

/// The service half of an endpoint on its own: `vsock:<port>` or a GUID.
///
/// Split out because a service id is also addressable by itself — binding a
/// service on *any* partition names only this half, which is the AF_HYPERV
/// counterpart of `StreamSocketListener::BindServiceNameAsync` taking a
/// service name rather than a full endpoint.
pub fn service_id_from_str(s: &str) -> Result<GUID, HvAddrParseError> {
    match s.strip_prefix("vsock:") {
        Some(port) => Ok(vsock_service_id(
            port.parse::<u32>().map_err(|_| HvAddrParseError)?,
        )),
        None => GUID::try_from(s).map_err(|_| HvAddrParseError),
    }
}

/// `hv://…` would not parse. Carries nothing: there is one way to spell an
/// endpoint and a caller that got it wrong needs the format, not a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HvAddrParseError;

impl core::fmt::Display for HvAddrParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("expected hv://<vm>/<service>, as `HvSocketAddr` renders it")
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every form `Display` can produce must parse back to the same address.
    ///
    /// Driven from the *rendered* string rather than from a table of literals,
    /// so the two impls cannot drift: a change to either side fails here.
    #[test]
    fn endpoint_strings_round_trip() {
        let cases = [
            HvSocketAddr::wildcard(vsock_service_id(5000)),
            HvSocketAddr::loopback(vsock_service_id(22)),
            HvSocketAddr::parent(vsock_service_id(0)),
            HvSocketAddr::new(HV_GUID_BROADCAST, HV_GUID_ZERO),
            HvSocketAddr::new(HV_GUID_CHILDREN, HV_GUID_ZERO),
            HvSocketAddr::new(HV_GUID_SILOHOST, HV_GUID_ZERO),
            // Neither half well-known: both fall through to plain GUIDs.
            HvSocketAddr::new(
                GUID::from_u128(0x9b6a1e00_1234_5678_9abc_def012345678),
                GUID::from_u128(0x0fedcba9_8765_4321_0fed_cba987654321),
            ),
        ];
        for addr in cases {
            let rendered = addr.to_string();
            let parsed: HvSocketAddr = rendered.parse().unwrap_or_else(|e| {
                panic!("{rendered} did not parse back: {e}");
            });
            assert_eq!(parsed, addr, "round trip changed {rendered}");
        }
    }

    /// The sugar is optional: a caller may always spell a GUID out, and it
    /// means the same endpoint as the name.
    #[test]
    fn plain_guids_mean_the_same_as_the_names() {
        let sugared: HvSocketAddr = "hv://loopback/vsock:5000".parse().unwrap();
        let spelled: HvSocketAddr =
            "hv://e0e16197-dd56-4a10-9195-5ee7a155a838/00001388-facb-11e6-bd58-64006a7986d3"
                .parse()
                .unwrap();
        assert_eq!(sugared, spelled);
    }

    #[test]
    fn malformed_endpoints_are_refused() {
        for bad in [
            "",
            "loopback/vsock:1",            // no scheme
            "hv://loopback",               // no service
            "hv://loopback/",              // empty service
            "hv:///vsock:1",               // empty vm
            "hv://nosuchvm/vsock:1",       // unknown name, not a GUID
            "hv://loopback/vsock:",        // no port
            "hv://loopback/vsock:notanum", // port is not a number
            "hv://loopback/vsock:-1",      // ports are unsigned
            "hv://loopback/not-a-guid",
        ] {
            assert!(
                bad.parse::<HvSocketAddr>().is_err(),
                "{bad:?} should not parse"
            );
        }
    }
}
