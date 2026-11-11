use super::super::super::{block_on, timeout, Runtime};
use super::*;
use std::time::Duration;

/// With the completion port skipped on both ends, an echo still round
/// trips: the mode applies to AF_UNIX as to any kernel-provider socket.
#[test]
fn unix_stream_echo_skipping_the_port() {
    let rt = Runtime::new();
    let path = std::env::temp_dir()
        .join(format!("tp_unix_skip_{}.sock", std::process::id()))
        .to_string_lossy()
        .into_owned();
    let _ = std::fs::remove_file(&path);
    let result = block_on(timeout(&rt, Duration::from_secs(10), async {
        let listener = UnixListener::bind(&rt, &path, None).unwrap();
        let server = rt.spawn(async move {
            let s = listener.accept().await.unwrap();
            s.set_skip_completion_on_success().unwrap();
            for _ in 0..100 {
                let (n, b) = s.read(vec![0u8; 16]).await;
                let n = n.unwrap();
                s.write(b[..n].to_vec()).await.0.unwrap();
            }
        });
        let client = UnixStream::connect(&rt, &path).await.unwrap();
        client.set_skip_completion_on_success().unwrap();
        for i in 0..100u8 {
            client.write(vec![i; 8]).await.0.unwrap();
            let (n, b) = client.read(vec![0u8; 16]).await;
            assert_eq!(&b[..n.unwrap()], &[i; 8]);
        }
        server.await.unwrap();
    }));
    let _ = std::fs::remove_file(&path);
    result.expect("unix echo timed out");
}

#[test]
fn unix_stream_echo() {
    let rt = Runtime::new();
    let path = std::env::temp_dir()
        .join(format!("tp_unix_{}.sock", std::process::id()))
        .to_string_lossy()
        .into_owned();
    let _ = std::fs::remove_file(&path);
    let result = block_on(timeout(&rt, Duration::from_secs(10), async {
        let listener = UnixListener::bind(&rt, &path, None).unwrap();
        let p2 = path.clone();
        let server = rt.spawn(async move {
            let s = listener.accept().await.unwrap();
            let (n, b) = s.read(vec![0u8; 16]).await;
            let n = n.unwrap();
            s.write(b[..n].to_vec()).await.0.unwrap();
            let _ = p2;
        });
        let client = UnixStream::connect(&rt, &path).await.unwrap();
        client.write(b"unix!".to_vec()).await.0.unwrap();
        let (n, b) = client.read(vec![0u8; 16]).await;
        assert_eq!(&b[..n.unwrap()], b"unix!");
        server.await.unwrap();
    }));
    let _ = std::fs::remove_file(&path);
    result.expect("unix echo timed out");
}

/// A buffer type of the caller's own, sent through `TransmitPackets<Own>`.
struct Own(Vec<u8>);

// SAFETY: a `Vec`'s heap stays put when the `Vec` moves.
unsafe impl crate::common::buffer::IoBuf for Own {
    fn stable_ptr(&self) -> *const u8 {
        self.0.as_ptr()
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}

/// `TransmitPackets` and `TransmitFile` on AF_UNIX (measured to work):
/// kinds mixed with `then`, a file, a caller's own buffer type, and a
/// run-time list of mixed kinds through `RawBuf::of`.
#[test]
fn unix_transmit_packets_and_file() {
    use crate::tp::{List, TransmitFlags, TransmitSequence};
    use crate::common::buffer::RawBuf;
    let rt = Runtime::new();
    let path = temp_path("tx");
    let file_path = std::env::temp_dir().join(format!("tp_unix_tx_{}.txt", std::process::id()));
    std::fs::write(&file_path, b"file!").unwrap();
    let result = block_on(timeout(&rt, Duration::from_secs(10), async {
        let listener = UnixListener::bind(&rt, &path, None).unwrap();
        let server = rt.spawn(async move {
            let s = listener.accept().await.unwrap();
            let mut got = Vec::new();
            while got.len() < 17 {
                let (n, b) = s.read(vec![0u8; 16]).await;
                let n = n.unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&b[..n]);
            }
            got
        });
        let client = UnixStream::connect(&rt, &path).await.unwrap();
        // Kinds mixed by `then`: a Vec, then a shared slice.
        let shared: std::sync::Arc<[u8]> = std::sync::Arc::from(&b"ts!"[..]);
        let mixed = b"pk".to_vec().then(shared);
        let n = client.transmit_packets(mixed, TransmitFlags::default()).await.unwrap();
        assert_eq!(n, 5);
        let file = crate::tp::File::open(&rt, &file_path.to_string_lossy()).unwrap();
        let n = client.transmit_file(&file, 0, 0, TransmitFlags::default()).await.unwrap();
        assert_eq!(n, 5);
        // A caller's own buffer type is a sequence by itself ...
        let n = client.transmit_packets(Own(b"ow".to_vec()), TransmitFlags::default()).await.unwrap();
        assert_eq!(n, 2);
        // ... and kinds mixed at run time go through `RawBuf::of`.
        let dynamic = List(vec![RawBuf::of(b"n".to_vec()), RawBuf::of(Own(b"!".to_vec()))]);
        let n = client.transmit_packets(dynamic, TransmitFlags::default()).await.unwrap();
        assert_eq!(n, 2);
        // Borrowed elements, on the caller's event: the unchecked path.
        let tail = String::from("end");
        let borrowed = (&tail.as_bytes()[..1]).then(&tail.as_bytes()[1..]);
        let n = unsafe { client.transmit_packets_unchecked(borrowed, TransmitFlags::default(), crate::tp::io_event().unwrap()) }
            .await
            .unwrap();
        assert_eq!(n, 3);
        assert_eq!(server.await.unwrap(), b"pkts!file!own!end");
    }));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&file_path);
    result.expect("timed out");
}

fn temp_path(tag: &str) -> String {
    let path = std::env::temp_dir()
        .join(format!("tp_unix_{tag}_{}.sock", std::process::id()))
        .to_string_lossy()
        .into_owned();
    let _ = std::fs::remove_file(&path);
    path
}

/// Both ends report the other's process, and through it the account.
#[test]
fn each_end_identifies_the_other() {
    let rt = Runtime::new();
    let path = temp_path("peer");
    let result = block_on(timeout(&rt, Duration::from_secs(10), async {
        let listener = UnixListener::bind(&rt, &path, None).unwrap();
        let server = rt.spawn(async move { listener.accept().await.unwrap() });
        let client = UnixStream::connect(&rt, &path).await.unwrap();
        let server = server.await.unwrap();
        let me = std::process::id();
        let my_sid = security::process_user_sid(me).unwrap();
        assert_eq!(server.peer_process_id().unwrap(), me);
        assert_eq!(server.peer_user_sid().unwrap(), my_sid);
        assert_eq!(client.peer_process_id().unwrap(), me);
    }));
    let _ = std::fs::remove_file(&path);
    result.expect("timed out");
}

/// The socket file's DACL is what `connect` is checked against, so a
/// descriptor that does not grant this account keeps it out.
#[test]
fn a_descriptor_keeps_out_who_it_does_not_grant() {
    let rt = Runtime::new();
    let path = temp_path("deny");
    let system_only = SecurityDescriptor::from_sddl("D:P(A;;GA;;;SY)").unwrap();
    let listener = UnixListener::bind(&rt, &path, Some(&system_only)).unwrap();
    let result =
        block_on(timeout(&rt, Duration::from_secs(10), UnixStream::connect(&rt, &path)))
            .expect("timed out");
    drop(listener);
    let _ = std::fs::remove_file(&path);
    if !security::afunix_dacl_is_reliable_here() {
        eprintln!("AF_UNIX connect through a SYSTEM-only DACL -> {:?}", result.as_ref().map(|_| ()));
        return;
    }
    let err = result.err().expect("connected through a DACL that does not grant this account");
    // WSAEACCES
    assert_eq!(err.code(), windows_core::HRESULT::from_win32(10013), "{err}");
}
