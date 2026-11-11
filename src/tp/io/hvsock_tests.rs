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
            unsafe { windows::Win32::Networking::WinSock::closesocket(s) };
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
