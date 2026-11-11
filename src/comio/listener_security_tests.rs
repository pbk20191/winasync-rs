use super::*;
use crate::common::block_on;
use crate::common::security::process_user_sid;
use std::future::IntoFuture;
use std::sync::mpsc;
use windows::Foundation::IClosable;
use windows_core::{HSTRING, Interface};

use super::super::bindings::Pbk20191::WinAsync::Comio::IListenerSecurity;

fn pipe_name(tag: &str) -> String {
    format!(r"\\.\pipe\comio_sec_{tag}_{}", std::process::id())
}

fn sock_path(tag: &str) -> String {
    let p = std::env::temp_dir()
        .join(format!("comio_sec_{tag}_{}.sock", std::process::id()))
        .to_string_lossy()
        .into_owned();
    let _ = std::fs::remove_file(&p);
    p
}

/// A handler reporting `(RemoteProcessId, RemoteUserSid)` per connection,
/// keeping each delivered stream alive until the test ends.
fn identity_handler() -> (ConnectionHandler, mpsc::Receiver<(u32, String)>) {
    let (tx, rx) = mpsc::channel();
    let handler = ConnectionHandler::new(move |_s, args| {
        let args = args.as_ref().unwrap();
        std::mem::forget(args.Stream()?);
        let _ = tx.send((args.RemoteProcessId()?, args.RemoteUserSid()?.to_string()));
        Ok(())
    });
    (handler, rx)
}

fn me() -> (u32, String) {
    let pid = std::process::id();
    (pid, process_user_sid(pid).unwrap())
}

#[test]
fn pipe_args_name_the_client() {
    let name = pipe_name("peer");
    let listener = NamedPipeConnectionListener::new();
    let (handler, rx) = identity_handler();
    listener.connection_received(&handler).unwrap();
    block_on(listener.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();
    let _client = block_on(crate::comio::NamedPipeConnector::connect(&name)).unwrap();
    let got = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!(got, me());
    listener.cast::<IClosable>().unwrap().Close().unwrap();
}

#[test]
fn unix_args_name_the_client() {
    let path = sock_path("peer");
    let listener = UnixConnectionListener::new();
    let (handler, rx) = identity_handler();
    listener.connection_received(&handler).unwrap();
    block_on(listener.bind(&HSTRING::from(&path)).unwrap().into_future()).unwrap();
    let _client = block_on(crate::comio::UnixConnector::connect(&path)).unwrap();
    let got = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!(got, me());
    listener.cast::<IClosable>().unwrap().Close().unwrap();
    let _ = std::fs::remove_file(&path);
}

/// `ITransportPeer` from both ends of both local transports: the server's
/// stream names the client, and the client's names the server -- which is
/// how a client checks it reached the server it meant.
#[test]
fn streams_name_their_peer_from_either_end() {
    use super::super::bindings::Pbk20191::WinAsync::Comio::ITransportPeer;
    let peer_of = |s: &ITransportStream| {
        let p: ITransportPeer = s.cast().unwrap();
        (p.RemoteProcessId().unwrap(), p.RemoteUserSid().unwrap().to_string())
    };
    // The server ends are delivered on a pool thread; ask there.
    let (tx, rx) = mpsc::channel();
    let handler = ConnectionHandler::new(move |_s, args| {
        let stream = args.as_ref().unwrap().Stream()?;
        let p: super::super::bindings::Pbk20191::WinAsync::Comio::ITransportPeer = stream.cast()?;
        let _ = tx.send((p.RemoteProcessId()?, p.RemoteUserSid()?.to_string()));
        std::mem::forget(stream);
        Ok(())
    });

    let name = pipe_name("streampeer");
    let pipe = NamedPipeConnectionListener::new();
    pipe.connection_received(&handler).unwrap();
    block_on(pipe.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();
    let client = block_on(crate::comio::NamedPipeConnector::connect(&name)).unwrap();
    assert_eq!(peer_of(&client), me(), "pipe client end");
    assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(), me(), "pipe server end");
    pipe.cast::<IClosable>().unwrap().Close().unwrap();

    let path = sock_path("streampeer");
    let unix = UnixConnectionListener::new();
    unix.connection_received(&handler).unwrap();
    block_on(unix.bind(&HSTRING::from(&path)).unwrap().into_future()).unwrap();
    let client = block_on(crate::comio::UnixConnector::connect(&path)).unwrap();
    assert_eq!(peer_of(&client), me(), "unix client end");
    assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(), me(), "unix server end");
    unix.cast::<IClosable>().unwrap().Close().unwrap();
    let _ = std::fs::remove_file(&path);
}

/// The property round-trips, rejects what is not SDDL, and is fixed once
/// the listener is bound.
#[test]
fn the_descriptor_is_validated_and_fixed_at_bind() {
    let listener = NamedPipeConnectionListener::new();
    let security: IListenerSecurity = listener.cast().unwrap();
    assert_eq!(security.SecurityDescriptor().unwrap(), "");
    security.SetSecurityDescriptor(&HSTRING::from("D:P(A;;GA;;;OW)")).unwrap();
    assert_eq!(security.SecurityDescriptor().unwrap(), "D:P(A;;GA;;;OW)");
    assert_eq!(
        security.SetSecurityDescriptor(&HSTRING::from("not sddl")).unwrap_err().code(),
        windows::Win32::Foundation::E_INVALIDARG
    );
    block_on(listener.bind(&HSTRING::from(pipe_name("fixed"))).unwrap().into_future()).unwrap();
    assert_eq!(
        security.SetSecurityDescriptor(&HSTRING::new()).unwrap_err().code(),
        E_ILLEGAL_METHOD_CALL
    );
}

/// SYSTEM only: this account is kept out, on both local transports.
#[test]
fn a_descriptor_keeps_out_who_it_does_not_grant() {
    const SYSTEM_ONLY: &str = "D:P(A;;GA;;;SY)";
    let (handler, _rx) = identity_handler();

    let name = pipe_name("deny");
    let pipe = NamedPipeConnectionListener::new();
    pipe.set_security_descriptor(&HSTRING::from(SYSTEM_ONLY)).unwrap();
    pipe.connection_received(&handler).unwrap();
    block_on(pipe.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();
    let err = block_on(crate::comio::NamedPipeConnector::connect(&name)).unwrap_err();
    assert_eq!(err.code(), windows::Win32::Foundation::ERROR_ACCESS_DENIED.to_hresult());
    pipe.cast::<IClosable>().unwrap().Close().unwrap();

    let path = sock_path("deny");
    let unix = UnixConnectionListener::new();
    unix.set_security_descriptor(&HSTRING::from(SYSTEM_ONLY)).unwrap();
    unix.connection_received(&handler).unwrap();
    block_on(unix.bind(&HSTRING::from(&path)).unwrap().into_future()).unwrap();
    let result = block_on(crate::comio::UnixConnector::connect(&path));
    unix.cast::<IClosable>().unwrap().Close().unwrap();
    let _ = std::fs::remove_file(&path);
    if !crate::common::security::afunix_dacl_is_reliable_here() {
        eprintln!(
            "AF_UNIX connect through a SYSTEM-only DACL -> {:?}",
            result.as_ref().map(|_| ())
        );
        return;
    }
    let err = result.unwrap_err();
    // WSAEACCES
    assert_eq!(err.code(), HRESULT::from_win32(10013), "{err}");
}

/// A name another server already holds is reported by the bind that
/// starts accepting, not joined.
#[test]
fn a_squatted_name_fails_the_bind() {
    use windows::Win32::Storage::FileSystem::{FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    let name = pipe_name("squat");
    let squatter = unsafe {
        CreateNamedPipeW(
            &HSTRING::from(&name),
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0),
            PIPE_TYPE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            0,
            0,
            0,
            None,
        )
    };
    assert!(!squatter.is_invalid());
    let listener = NamedPipeConnectionListener::new();
    let (handler, _rx) = identity_handler();
    listener.connection_received(&handler).unwrap();
    let err = listener.bind(&HSTRING::from(&name)).unwrap_err();
    assert_eq!(err.code(), windows::Win32::Foundation::ERROR_ACCESS_DENIED.to_hresult());
    unsafe { windows::Win32::Foundation::CloseHandle(squatter).unwrap() };
}

/// `Close` gives the name up: the reserved instance goes with it.
#[test]
fn close_releases_the_name() {
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    let claim = |name: &str| unsafe {
        CreateNamedPipeW(
            &HSTRING::from(name),
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_FIRST_PIPE_INSTANCE.0),
            PIPE_TYPE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            0,
            0,
            0,
            None,
        )
    };
    let name = pipe_name("release");
    let listener = NamedPipeConnectionListener::new();
    // Drops each delivered stream at once, so after the one connection
    // below only the reserved instance holds the name.
    let (tx, rx) = mpsc::channel();
    let handler = ConnectionHandler::new(move |_s, _args| {
        let _ = tx.send(());
        Ok(())
    });
    listener.connection_received(&handler).unwrap();
    block_on(listener.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();
    drop(block_on(crate::comio::NamedPipeConnector::connect(&name)).unwrap());
    rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();

    // Bound and open: the name is not free.
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(claim(&name).is_invalid(), "the name was free while the listener was open");

    listener.cast::<IClosable>().unwrap().Close().unwrap();
    // The cancelled accept's completion runs on the pool; give it a beat.
    let mut taken = claim(&name);
    for _ in 0..50 {
        if !taken.is_invalid() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        taken = claim(&name);
    }
    assert!(!taken.is_invalid(), "Close kept the name");
    unsafe { windows::Win32::Foundation::CloseHandle(taken).unwrap() };
}
