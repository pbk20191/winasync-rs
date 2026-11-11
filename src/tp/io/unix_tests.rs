use super::super::super::{block_on, timeout, Runtime};
use super::*;
use std::time::Duration;

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
