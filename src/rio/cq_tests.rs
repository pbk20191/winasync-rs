use super::super::buffer::RioBufferPool;
use super::super::rq::RqConfig;
use super::super::socket::RioUdpSocket;
use super::*;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use windows::Win32::System::IO::{
    CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED,
};

fn v4() -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
}

/// RIO driven by a completion port this crate does not own and knows
/// nothing about — the interop path for an existing IOCP runtime.
///
/// No thread pool, no TP_WAIT: the loop below is all a foreign runtime
/// would need to add.
#[test]
fn rio_driven_by_a_foreign_iocp() {
    // A bare completion port, as any other library would have.
    let iocp = unsafe {
        CreateIoCompletionPort(windows::Win32::Foundation::INVALID_HANDLE_VALUE, None, 0, 1)
    }
    .expect("iocp");

    // The tag RIO stamps on its notifications. Boxed so it has a stable,
    // unique address for the queue's whole life.
    let tag: Box<OVERLAPPED> = Box::new(unsafe { core::mem::zeroed() });
    let tag_ptr = Box::into_raw(tag);
    const RIO_KEY: usize = 0x5249_4F00; // 'RIO' + 0 — any value the caller picks

    let cq = unsafe {
        RioCq::with_iocp(iocp, RIO_KEY, tag_ptr as *mut core::ffi::c_void, 64)
    }
    .expect("iocp-driven cq");

    let pool = RioBufferPool::new(512, 4).expect("pool");
    // sum(max_outstanding) must not exceed the queue size, and RIO
    // enforces it: two sockets at 8+8 fit a 64-entry queue with room to
    // spare, whereas RqConfig::default() (64+64 each) would be refused
    // with WSAENOBUFS at bind.
    let rq = RqConfig {
        max_outstanding_recv: 8,
        max_outstanding_send: 8,
    };
    let server = RioUdpSocket::bind(&cq, v4(), rq).expect("server");
    let client = RioUdpSocket::bind(&cq, v4(), rq).expect("client");
    client.connect(server.local_addr().unwrap()).unwrap();

    let rx = server.recv(pool.acquire().unwrap()).expect("post recv");
    let mut out = pool.acquire().unwrap();
    out[..4].copy_from_slice(b"iocp");
    out.set_len(4);
    let tx = client.send(out).expect("post send");

    // The foreign runtime's loop: wait on its own port, recognise RIO's
    // packet by the tag, drain, re-arm.
    let mut drained = 0usize;
    for _ in 0..8 {
        let mut bytes = 0u32;
        let mut key = 0usize;
        let mut ovl: *mut OVERLAPPED = core::ptr::null_mut();
        let ok = unsafe {
            GetQueuedCompletionStatus(iocp, &mut bytes, &mut key, &mut ovl, 2000)
        };
        if ok.is_err() {
            break;
        }
        assert_eq!(key, RIO_KEY, "RIO stamped the key we gave it");
        assert_eq!(ovl, tag_ptr, "and the OVERLAPPED tag we gave it");
        assert_eq!(bytes, 0, "a notification carries no byte count");
        drained += cq.drain().expect("queue not corrupt");
        cq.notify().expect("re-arm");
        if drained >= 2 {
            break;
        }
    }
    assert_eq!(drained, 2, "both the send and the receive completed");

    let (sent, _) = block_on_ready(tx);
    assert_eq!(sent.unwrap(), 4);
    let (got, slice) = block_on_ready(rx);
    let n = got.unwrap();
    assert_eq!(&slice[..n], b"iocp");

    drop(server);
    drop(client);
    drop(cq);
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(iocp);
        drop(Box::from_raw(tag_ptr));
    }
}

/// Polls a future that is already complete. The completions were
/// dispatched by the drain above, so no executor is involved.
fn block_on_ready<F: Future>(fut: F) -> F::Output {
    crate::common::block_on(fut)
}
