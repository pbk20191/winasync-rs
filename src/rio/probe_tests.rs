//! Probe: which address families take Registered I/O.
//!
//! `rq.rs` says `WSA_FLAG_REGISTERED_IO` is accepted only by the TCP/IP
//! families. This measures it, in three steps per family, and prints where
//! each stops and with which Winsock error:
//!
//! 1. `WSASocketW` with `WSA_FLAG_REGISTERED_IO`;
//! 2. `RIOCreateRequestQueue` on that socket;
//! 3. a connected pair, one `RIOReceive` on the RIO side, one plain `send`
//!    from the peer, and the completion dequeued by polling.
//!
//! Run by hand: `cargo test --features rio rio_families -- --ignored --nocapture`.

use std::mem::size_of;
use windows::Win32::Networking::WinSock::{
    accept, bind, closesocket, connect, listen, send, ADDRESS_FAMILY, AF_UNIX,
    SEND_RECV_FLAGS, SOCKADDR, SOCKADDR_UN, SOCKET, SOCK_STREAM, WSASocketW, WSA_FLAG_OVERLAPPED,
    WSA_FLAG_REGISTERED_IO,
};

use super::cq::RioCq;
use super::rq::RIO_INVALID_RQ;
use super::table::rio_table;
use super::RioBufferPool;
use crate::common::winsock::{wsa_last_err, WinsockRef};

/// The family's end of the probe: the RIO socket, and a plain peer connected
/// to it (the peer's socket is returned to be closed).
struct Family {
    name: &'static str,
    family: i32,
    protocol: i32,
    /// Connects `rio` to a fresh plain listener and returns the accepted
    /// peer, or an error naming the step.
    pair: fn(rio: SOCKET) -> Result<SOCKET, String>,
}

fn rio_socket(f: &Family) -> windows_core::Result<SOCKET> {
    unsafe {
        WSASocketW(
            f.family,
            SOCK_STREAM.0,
            f.protocol,
            None,
            0,
            WSA_FLAG_OVERLAPPED | WSA_FLAG_REGISTERED_IO,
        )
    }
}

fn plain_socket(family: i32, protocol: i32) -> windows_core::Result<SOCKET> {
    unsafe { WSASocketW(family, SOCK_STREAM.0, protocol, None, 0, WSA_FLAG_OVERLAPPED) }
}

fn unix_pair(rio: SOCKET) -> Result<SOCKET, String> {
    let path = std::env::temp_dir().join(format!("winasync-rio-probe-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let bytes = path.to_string_lossy().into_owned().into_bytes();
    let mut sa = SOCKADDR_UN {
        sun_family: ADDRESS_FAMILY(AF_UNIX),
        sun_path: [0i8; 108],
    };
    for (i, &b) in bytes.iter().enumerate() {
        sa.sun_path[i] = b as i8;
    }
    let len = (size_of::<ADDRESS_FAMILY>() + bytes.len() + 1) as i32;
    let l = plain_socket(AF_UNIX as i32, 0).map_err(|e| format!("listener socket: {e:?}"))?;
    let r = unsafe { bind(l, &sa as *const _ as *const SOCKADDR, len) };
    if r != 0 {
        return Err(format!("bind: {:?}", wsa_last_err()));
    }
    if unsafe { listen(l, 1) } != 0 {
        return Err(format!("listen: {:?}", wsa_last_err()));
    }
    if unsafe { connect(rio, &sa as *const _ as *const SOCKADDR, len) } != 0 {
        return Err(format!("connect (RIO side): {:?}", wsa_last_err()));
    }
    let peer = unsafe { accept(l, None, None) };
    unsafe { closesocket(l) };
    let _ = std::fs::remove_file(&path);
    peer.map_err(|e| format!("accept: {e:?}"))
}

#[cfg(feature = "tp")]
fn hv_pair(rio: SOCKET) -> Result<SOCKET, String> {
    use crate::common::hvaddr::{HvSocketAddr, HV_PROTOCOL_RAW};
    use windows::Win32::Networking::WinSock::AF_HYPERV;
    let service = windows_core::GUID::from_u128(0x7d1b2a6e_3c4f_4b7a_9e2d_5f6a7b8c9d0e);
    let (wild, len) = HvSocketAddr::wildcard(service).to_raw();
    let l = plain_socket(AF_HYPERV as i32, HV_PROTOCOL_RAW)
        .map_err(|e| format!("listener socket: {e:?}"))?;
    if unsafe { bind(l, &wild as *const _ as *const SOCKADDR, len) } != 0 {
        return Err(format!("bind: {:?}", wsa_last_err()));
    }
    if unsafe { listen(l, 1) } != 0 {
        return Err(format!("listen: {:?}", wsa_last_err()));
    }
    let (lo, len) = HvSocketAddr::loopback(service).to_raw();
    if unsafe { connect(rio, &lo as *const _ as *const SOCKADDR, len) } != 0 {
        return Err(format!("connect (RIO side): {:?}", wsa_last_err()));
    }
    let peer = unsafe { accept(l, None, None) };
    unsafe { closesocket(l) };
    peer.map_err(|e| format!("accept: {e:?}"))
}

fn probe(f: &Family) -> String {
    // The raw sockets below are covered by this one reference for the
    // probe's whole duration.
    let _winsock = WinsockRef::new();
    let table = match rio_table() {
        Ok(t) => t,
        Err(e) => return format!("{}: no RIO table: {e:?}", f.name),
    };
    let sock = match rio_socket(f) {
        Ok(s) => s,
        Err(e) => {
            // Not the flag combination: REGISTERED_IO alone answers the same.
            let alone = unsafe { WSASocketW(f.family, SOCK_STREAM.0, f.protocol, None, 0, WSA_FLAG_REGISTERED_IO) };
            let alone = match alone {
                Ok(s) => {
                    unsafe { closesocket(s) };
                    "but REGISTERED_IO alone succeeds".to_string()
                }
                Err(e2) => format!("REGISTERED_IO alone: {:?}", e2.code()),
            };
            return format!("{}: step 1 WSASocketW(OVERLAPPED|REGISTERED_IO) failed: {:?} ({alone})", f.name, e.code());
        }
    };
    let cq = match RioCq::polling(16) {
        Ok(c) => c,
        Err(e) => return format!("{}: polling cq: {e:?}", f.name),
    };
    let rq = unsafe {
        (table.raw().RIOCreateRequestQueue.unwrap())(sock, 1, 1, 1, 1, cq.raw(), cq.raw(), core::ptr::null())
    };
    if rq == RIO_INVALID_RQ {
        let e = wsa_last_err();
        unsafe { closesocket(sock) };
        return format!("{}: step 1 ok; step 2 RIOCreateRequestQueue failed: {e:?}", f.name);
    }
    let peer = match (f.pair)(sock) {
        Ok(p) => p,
        Err(e) => {
            unsafe { closesocket(sock) };
            return format!("{}: steps 1-2 ok; step 3 pairing failed at {e}", f.name);
        }
    };
    let pool = RioBufferPool::new(64, 1).expect("pool");
    let slice = pool.acquire().unwrap();
    let buf = slice.rio_buf();
    let posted = unsafe { (table.raw().RIOReceive.unwrap())(rq, &buf, 1, 0, core::ptr::null()) };
    if !posted.as_bool() {
        let e = wsa_last_err();
        unsafe { closesocket(peer) };
        unsafe { closesocket(sock) };
        return format!("{}: steps 1-2 ok; step 3 RIOReceive failed: {e:?}", f.name);
    }
    let n = unsafe { send(peer, b"probe", SEND_RECV_FLAGS(0)) };
    if n < 0 {
        return format!("{}: steps 1-2 ok; step 3 peer send failed: {:?}", f.name, wsa_last_err());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut got = 0;
    while std::time::Instant::now() < deadline {
        match cq.drain() {
            Some(0) => std::thread::sleep(std::time::Duration::from_millis(1)),
            Some(k) => {
                got = k;
                break;
            }
            None => return format!("{}: step 3 RIO_CORRUPT_CQ", f.name),
        }
    }
    unsafe { closesocket(peer) };
    unsafe { closesocket(sock) };
    // The completion (if any) carried a null context: nothing to dispatch,
    // so the slice is still ours and the pool drops clean.
    drop(slice);
    if got == 0 {
        return format!("{}: steps 1-2 ok; step 3 RIOReceive posted but no completion in 2 s", f.name);
    }
    format!("{}: steps 1-3 ok — Registered I/O works ({} completion, {:?})", f.name, got, pool.slice_size())
}

#[test]
#[ignore = "probe, prints its findings; run with --ignored --nocapture"]
fn rio_families() {
    let families = [
        Family {
            name: "AF_UNIX",
            family: AF_UNIX as i32,
            protocol: 0,
            pair: unix_pair,
        },
        #[cfg(feature = "tp")]
        Family {
            name: "AF_HYPERV",
            family: windows::Win32::Networking::WinSock::AF_HYPERV as i32,
            protocol: crate::common::hvaddr::HV_PROTOCOL_RAW,
            pair: hv_pair,
        },
    ];
    for f in &families {
        eprintln!("{}", probe(f));
    }
}
