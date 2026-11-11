use crate::common::security::SecurityDescriptor;
use super::super::super::{block_on, timeout, Runtime};
use super::*;

#[test]
fn named_pipe_echo() {
    let rt = Runtime::new();
    let name = format!(r"\\.\pipe\tp_pipe_echo_{}", std::process::id());
    let rt2 = rt.clone();
    let sname = name.clone();
    let server = std::thread::spawn(move || {
        let listener = NamedPipeListener::bind(&rt2, &sname, &NamedPipeOptions::new()).unwrap();
        let s = block_on(listener.accept()).unwrap();
        let (n, b) = block_on(s.read(vec![0u8; 16]));
        let n = n.unwrap();
        block_on(s.write(b[..n].to_vec())).0.unwrap();
        n
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    let client = NamedPipeConnector::connect(&rt, &name).unwrap();
    block_on(client.write(b"hello".to_vec())).0.unwrap();
    let (n, b) = block_on(client.read(vec![0u8; 16]));
    assert_eq!(&b[..n.unwrap()], b"hello");
    assert_eq!(server.join().unwrap(), 5);
}

fn unique(tag: &str) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    format!(
        r"\\.\pipe\tp_pipe_{tag}_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// Accepts one client of `name` on a thread, returning the server end.
fn accept_one(rt: &Runtime, listener: NamedPipeListener) -> std::thread::JoinHandle<windows_core::Result<NamedPipeStream>> {
    let _ = rt;
    std::thread::spawn(move || block_on(listener.accept()))
}

/// Both ends name each other, and the server reads the client's account
/// from its token -- this process, on both counts.
#[test]
fn each_end_identifies_the_other() {
    let rt = Runtime::new();
    let name = unique("peer");
    let server = accept_one(&rt, NamedPipeListener::bind(
        &rt,
        &name,
        &NamedPipeOptions::new(),
    ).unwrap());
    std::thread::sleep(std::time::Duration::from_millis(100));
    let client = NamedPipeConnector::connect(&rt, &name).unwrap();
    let server = server.join().unwrap().unwrap();

    let me = std::process::id();
    let my_sid = security::process_user_sid(me).unwrap();
    assert_eq!(server.peer_process_id().unwrap(), me);
    assert_eq!(server.peer_user_sid().unwrap(), my_sid);
    assert_eq!(client.peer_process_id().unwrap(), me);
    assert_eq!(client.peer_user_sid().unwrap(), my_sid);
}

/// A pipe someone else created first is not joined: the first instance
/// asks to be first, and is refused.
#[test]
fn a_squatted_name_is_refused() {
    use windows::Win32::Storage::FileSystem::{FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    let rt = Runtime::new();
    let name = unique("squat");
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
    let err = block_on(NamedPipeListener::bind(
        &rt,
        &name,
        &NamedPipeOptions::new(),
    ).unwrap().accept()).err().unwrap();
    assert_eq!(err.code(), windows::Win32::Foundation::ERROR_ACCESS_DENIED.to_hresult());
    unsafe { windows::Win32::Foundation::CloseHandle(squatter).unwrap() };
}

/// Once a client has connected and gone, the name is still ours: the
/// next instance was created before the connected one was handed on.
#[test]
fn the_name_is_held_between_accepts() {
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    let rt = Runtime::new();
    let name = unique("held");
    let listener = Arc::new(NamedPipeListener::bind(
        &rt,
        &name,
        &NamedPipeOptions::new(),
    ).unwrap());
    let l2 = listener.clone();
    let server = std::thread::spawn(move || block_on(l2.accept()));
    std::thread::sleep(std::time::Duration::from_millis(100));
    drop(NamedPipeConnector::connect(&rt, &name).unwrap());
    drop(server.join().unwrap().unwrap());

    let squatter = unsafe {
        CreateNamedPipeW(
            &HSTRING::from(&name),
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_FIRST_PIPE_INSTANCE.0),
            PIPE_TYPE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            0,
            0,
            0,
            None,
        )
    };
    assert!(squatter.is_invalid(), "the name was free between accepts");
    drop(listener);
}

/// An accept dropped before anyone connects -- here by a timeout -- still
/// leaves the name held: its instance is replaced on the way out.
#[test]
fn a_dropped_accept_keeps_the_name() {
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    let rt = Runtime::new();
    let name = unique("dropped");
    let listener = NamedPipeListener::bind(&rt, &name, &NamedPipeOptions::new()).unwrap();
    let timed_out = block_on(crate::tp::timeout(
        &rt,
        std::time::Duration::from_millis(100),
        listener.accept(),
    ))
    .is_err();
    assert!(timed_out);
    let squatter = unsafe {
        CreateNamedPipeW(
            &HSTRING::from(&name),
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_FIRST_PIPE_INSTANCE.0),
            PIPE_TYPE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            0,
            0,
            0,
            None,
        )
    };
    assert!(squatter.is_invalid(), "the name was free after a dropped accept");
    // And the reserved instance still serves the next accept.
    let server = accept_one(&rt, listener);
    let _client = NamedPipeConnector::connect(&rt, &name).unwrap();
    server.join().unwrap().unwrap();
}

/// A descriptor that does not grant this account keeps it out.
#[test]
fn a_descriptor_keeps_out_who_it_does_not_grant() {
    let rt = Runtime::new();
    let name = unique("deny");
    let system_only = SecurityDescriptor::from_sddl("D:P(A;;GA;;;SY)").unwrap();
    let listener = NamedPipeListener::bind(
        &rt,
        &name,
        &NamedPipeOptions::new().security(&system_only),
    ).unwrap();
    // Nobody is let in, so the accept only ends by timing out -- which
    // drops it and cancels its operation, rather than leaving one live
    // for the leak checks elsewhere in this process to trip over.
    let rt2 = rt.clone();
    let server = std::thread::spawn(move || {
        block_on(crate::tp::timeout(
            &rt2,
            std::time::Duration::from_millis(500),
            listener.accept(),
        ))
        .is_err()
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    let err = NamedPipeConnector::connect(&rt, &name).err().unwrap();
    assert_eq!(err.code(), windows::Win32::Foundation::ERROR_ACCESS_DENIED.to_hresult());
    assert!(server.join().unwrap(), "the accept completed for a client it should refuse");
}

/// And one that does grant it lets it in.
#[test]
fn a_descriptor_admits_who_it_grants() {
    let rt = Runtime::new();
    let name = unique("allow");
    let owner_only = SecurityDescriptor::from_sddl("D:P(A;;GA;;;OW)").unwrap();
    let server = accept_one(&rt, NamedPipeListener::bind(
        &rt,
        &name,
        &NamedPipeOptions::new().security(&owner_only),
    ).unwrap());
    std::thread::sleep(std::time::Duration::from_millis(100));
    let _client = NamedPipeConnector::connect(&rt, &name).unwrap();
    server.join().unwrap().unwrap();
}

/// Opens `name` read-only as the anonymous account, the way any local
/// process could under `CreateNamedPipeW`'s own default descriptor.
pub(crate) fn open_read_only_as_anonymous(name: &str) -> windows_core::Result<()> {
    use windows::Win32::Security::{ImpersonateAnonymousToken, RevertToSelf};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_SHARE_NONE, OPEN_EXISTING,
    };
    use windows::Win32::System::Threading::GetCurrentThread;
    let name = name.to_owned();
    std::thread::spawn(move || unsafe {
        ImpersonateAnonymousToken(GetCurrentThread()).unwrap();
        let opened = CreateFileW(
            &HSTRING::from(&name),
            FILE_GENERIC_READ.0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            None,
        );
        RevertToSelf().unwrap();
        opened.map(|h| {
            let _ = windows::Win32::Foundation::CloseHandle(h);
        })
    })
    .join()
    .unwrap()
}

/// The default descriptor is the one the docs promise. The system's own
/// default would let this read-only, anonymous open through.
#[test]
fn the_default_descriptor_keeps_out_anonymous_readers() {
    let rt = Runtime::new();
    let name = unique("anon");
    let listener = NamedPipeListener::bind(&rt, &name, &NamedPipeOptions::new()).unwrap();
    let rt2 = rt.clone();
    let server = std::thread::spawn(move || {
        block_on(crate::tp::timeout(
            &rt2,
            std::time::Duration::from_millis(500),
            listener.accept(),
        ))
        .is_err()
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    let err = open_read_only_as_anonymous(&name).unwrap_err();
    assert_eq!(err.code(), windows::Win32::Foundation::ERROR_ACCESS_DENIED.to_hresult());
    assert!(server.join().unwrap(), "the accept completed for an anonymous reader");
}

/// A message-mode pair: the server end, and a client that found out the
/// mode by itself.
fn message_pair(rt: &Runtime, options: NamedPipeOptions) -> (NamedPipeStream, NamedPipeStream) {
    let name = unique("msg");
    let listener =
        NamedPipeListener::bind(rt, &name, &options.message_mode(true)).unwrap();
    let server = accept_one(rt, listener);
    std::thread::sleep(std::time::Duration::from_millis(100));
    let client = NamedPipeConnector::connect(rt, &name).unwrap();
    (server.join().unwrap().unwrap(), client)
}

/// Messages arrive whole and one at a time, whatever their size --
/// including empty, and including larger than the first read.
#[test]
fn message_mode_keeps_boundaries() {
    let rt = Runtime::new();
    let (server, client) = message_pair(&rt, NamedPipeOptions::new());
    assert!(server.is_message_mode());
    assert!(client.is_message_mode(), "the connector did not notice message mode");

    let big: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
    let sent = [b"hello".to_vec(), Vec::new(), big.clone(), b"!".to_vec()];
    let writer = {
        let sent = sent.clone();
        let rt = rt.clone();
        std::thread::spawn(move || {
            let _ = &rt;
            for m in sent {
                block_on(client.write(m)).0.unwrap();
            }
            client
        })
    };
    for m in &sent {
        assert_eq!(&block_on(server.read_message_vec(1 << 20)).unwrap(), m);
    }
    drop(writer.join().unwrap());
}

/// A plain `read` still works on a message pipe; it just cannot say
/// where a message ends.
#[test]
fn a_byte_read_returns_a_message_in_parts() {
    let rt = Runtime::new();
    let (server, client) = message_pair(&rt, NamedPipeOptions::new());
    block_on(client.write(b"hello".to_vec())).0.unwrap();
    let (n, b) = block_on(server.read(vec![0u8; 3]));
    assert_eq!(&b[..n.unwrap()], b"hel");
    let (n, b) = block_on(server.read(vec![0u8; 3]));
    assert_eq!(&b[..n.unwrap()], b"lo");
}

/// Too long for the caller: read to its end, discarded, and the message
/// after it is untouched.
#[test]
fn an_oversized_message_is_discarded_whole() {
    let rt = Runtime::new();
    let (server, client) = message_pair(&rt, NamedPipeOptions::new());
    // On a thread: a message larger than the pipe's buffer is not written
    // until it is read.
    let writer = std::thread::spawn(move || {
        block_on(client.write(vec![1u8; 100_000])).0.unwrap();
        block_on(client.write(b"next".to_vec())).0.unwrap();
        client
    });
    let err = block_on(server.read_message_vec(10)).unwrap_err();
    assert_eq!(err.code(), ERROR_MORE_DATA.to_hresult());
    assert_eq!(block_on(server.read_message_vec(10)).unwrap(), b"next");
    drop(writer.join().unwrap());
}

#[test]
fn a_byte_pipe_has_no_messages() {
    let rt = Runtime::new();
    let name = unique("bytes");
    let server = accept_one(&rt, NamedPipeListener::bind(
        &rt,
        &name,
        &NamedPipeOptions::new(),
    ).unwrap());
    std::thread::sleep(std::time::Duration::from_millis(100));
    let client = NamedPipeConnector::connect(&rt, &name).unwrap();
    let server = server.join().unwrap().unwrap();
    assert!(!client.is_message_mode());
    assert_eq!(
        block_on(server.read_message_vec(16)).unwrap_err().code(),
        E_ILLEGAL_METHOD_CALL
    );
}

#[test]
fn options_no_pipe_can_have_are_refused() {
    let rt = Runtime::new();
    let invalid = windows::Win32::Foundation::E_INVALIDARG;
    for options in [
        NamedPipeOptions::new().max_instances(0),
        NamedPipeOptions::new().max_instances(256),
        NamedPipeOptions::new().accept_remote_clients(true),
    ] {
        let err = NamedPipeListener::bind(&rt, &unique("bad"), &options)
            .err()
            .unwrap();
        assert_eq!(err.code(), invalid, "{options:?}");
    }
}

/// `\\localhost\pipe\...` goes through the SMB redirector even on the
/// same machine, so it is what a remote client looks like.
fn remote_name(local: &str) -> String {
    local.replacen(r"\\.\", r"\\localhost\", 1)
}

#[test]
fn remote_clients_are_refused_by_default() {
    let rt = Runtime::new();
    let name = unique("remote_no");
    let listener = NamedPipeListener::bind(&rt, &name, &NamedPipeOptions::new()).unwrap();
    let rt2 = rt.clone();
    let server = std::thread::spawn(move || {
        block_on(crate::tp::timeout(
            &rt2,
            std::time::Duration::from_millis(500),
            listener.accept(),
        ))
        .is_err()
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    let err = NamedPipeConnector::connect_with_timeout(&rt, &remote_name(&name), Duration::ZERO)
        .err()
        .unwrap();
    eprintln!("remote connect to a local-only pipe -> {err:?}");
    assert!(server.join().unwrap(), "the accept completed for a remote client");
}

#[test]
fn remote_clients_are_admitted_when_asked() {
    let rt = Runtime::new();
    let name = unique("remote_yes");
    let me = security::process_user_sid(std::process::id()).unwrap();
    let only_me = SecurityDescriptor::from_sddl(&format!("D:P(A;;GA;;;{me})")).unwrap();
    let options = NamedPipeOptions::new()
        .security(&only_me)
        .accept_remote_clients(true);
    let listener = NamedPipeListener::bind(&rt, &name, &options).unwrap();
    let server = accept_one(&rt, listener);
    std::thread::sleep(std::time::Duration::from_millis(100));
    let client = NamedPipeConnector::connect(&rt, &remote_name(&name)).unwrap();
    let server = server.join().unwrap().unwrap();
    block_on(client.write(b"over smb".to_vec())).0.unwrap();
    let (n, b) = block_on(server.read(vec![0u8; 16]));
    assert_eq!(&b[..n.unwrap()], b"over smb");
    eprintln!(
        "remote peer: pid {:?} (this process is {}), sid {:?}",
        server.peer_process_id(),
        std::process::id(),
        server.peer_user_sid()
    );
    assert_eq!(server.peer_user_sid().unwrap(), me);
}

/// At the limit an accept waits for an instance to close, and a client
/// is told the pipe is busy.
#[test]
fn the_instance_limit_holds_and_frees() {
    let rt = Runtime::new();
    let name = unique("limit");
    let options = NamedPipeOptions::new().max_instances(1);
    let listener =
        Arc::new(NamedPipeListener::bind(&rt, &name, &options).unwrap());

    let l = listener.clone();
    let first = std::thread::spawn(move || block_on(l.accept()));
    std::thread::sleep(std::time::Duration::from_millis(100));
    let client1 = NamedPipeConnector::connect(&rt, &name).unwrap();
    let server1 = first.join().unwrap().unwrap();

    // The one instance is taken, so the next accept waits for it.
    let l = listener.clone();
    let second = std::thread::spawn(move || block_on(l.accept()));
    std::thread::sleep(std::time::Duration::from_millis(100));
    let err = NamedPipeConnector::connect_with_timeout(&rt, &name, Duration::ZERO)
        .err()
        .unwrap();
    assert_eq!(err.code(), windows::Win32::Foundation::ERROR_PIPE_BUSY.to_hresult());
    assert!(!second.is_finished(), "an accept went past the instance limit");

    // Closing the first frees the slot; the waiting accept takes it.
    // Until it has, no instance exists and the name is not found -- the
    // gap a limit allows -- so the client retries briefly.
    drop(server1);
    drop(client1);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let _client2 = loop {
        match NamedPipeConnector::connect_with_timeout(&rt, &name, Duration::from_secs(1)) {
            Err(e)
                if e.code() == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND.to_hresult()
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            other => break other.unwrap(),
        }
    };
    second.join().unwrap().unwrap();
}

/// With a limit there may be no instance in reserve, so the name can be
/// free between accepts. Someone who takes it then is not joined: with
/// no instance of ours left, the next one asks to be first.
#[test]
fn a_name_lost_at_the_limit_is_not_joined() {
    use windows::Win32::Storage::FileSystem::{FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    let rt = Runtime::new();
    let name = unique("limit_squat");
    let options = NamedPipeOptions::new().max_instances(1);
    let listener =
        Arc::new(NamedPipeListener::bind(&rt, &name, &options).unwrap());
    let l = listener.clone();
    let first = std::thread::spawn(move || block_on(l.accept()));
    std::thread::sleep(std::time::Duration::from_millis(100));
    drop(NamedPipeConnector::connect(&rt, &name).unwrap());
    drop(first.join().unwrap().unwrap());

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
    assert!(!squatter.is_invalid(), "the name was still held");
    let err = block_on(listener.accept()).err().unwrap();
    assert_eq!(err.code(), windows::Win32::Foundation::ERROR_ACCESS_DENIED.to_hresult());
    unsafe { windows::Win32::Foundation::CloseHandle(squatter).unwrap() };
}

/// A connect gives up when its time is spent, not before and not never.
#[test]
fn a_connect_waits_its_timeout_for_a_busy_pipe() {
    let rt = Runtime::new();
    let name = unique("busy");
    let options = NamedPipeOptions::new().max_instances(1);
    let listener = NamedPipeListener::bind(&rt, &name, &options).unwrap();
    let server = accept_one(&rt, listener);
    std::thread::sleep(std::time::Duration::from_millis(100));
    let _client = NamedPipeConnector::connect(&rt, &name).unwrap();
    let _server = server.join().unwrap().unwrap();

    let started = std::time::Instant::now();
    let err = NamedPipeConnector::connect_with_timeout(&rt, &name, Duration::from_millis(300))
        .err()
        .unwrap();
    let waited = started.elapsed();
    assert_eq!(err.code(), windows::Win32::Foundation::ERROR_PIPE_BUSY.to_hresult());
    assert!(
        waited >= Duration::from_millis(250) && waited < Duration::from_secs(3),
        "waited {waited:?} for a 300 ms timeout"
    );
}

/// A server that answers each message with `reply(message)` until the
/// client goes away.
fn answering(
    server: NamedPipeStream,
    reply: impl Fn(&[u8]) -> Vec<u8> + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while let Ok(m) = block_on(server.read_message_vec(1 << 20)) {
            if block_on(server.write(reply(&m))).0.is_err() {
                break;
            }
        }
    })
}

/// One call writes the request and reads the reply, whatever its size.
#[test]
fn transact_round_trips() {
    let rt = Runtime::new();
    let (server, client) = message_pair(&rt, NamedPipeOptions::new());
    let server = answering(server, |m| {
        if m == b"big" {
            (0..150_000u32).map(|i| i as u8).collect()
        } else {
            m.to_ascii_uppercase()
        }
    });
    assert_eq!(block_on(client.transact_vec(b"ping".to_vec(), 64)).unwrap(), b"PING");
    let big = block_on(client.transact_vec(b"big".to_vec(), 1 << 20)).unwrap();
    assert_eq!(big, (0..150_000u32).map(|i| i as u8).collect::<Vec<_>>());
    assert_eq!(block_on(client.transact_vec(b"again".to_vec(), 64)).unwrap(), b"AGAIN");
    drop(client);
    server.join().unwrap();
}

/// A reply too long for the caller is discarded whole, and the pipe is
/// ready for the next transaction.
#[test]
fn an_oversized_reply_is_discarded_whole() {
    let rt = Runtime::new();
    let (server, client) = message_pair(&rt, NamedPipeOptions::new());
    let server = answering(server, |m| {
        if m == b"big" { vec![9u8; 100_000] } else { m.to_vec() }
    });
    let err = block_on(client.transact_vec(b"big".to_vec(), 10)).unwrap_err();
    assert_eq!(err.code(), ERROR_MORE_DATA.to_hresult());
    assert_eq!(block_on(client.transact_vec(b"small".to_vec(), 10)).unwrap(), b"small");
    drop(client);
    server.join().unwrap();
}

#[test]
fn a_byte_pipe_has_no_transactions() {
    let rt = Runtime::new();
    let name = unique("bytes_tx");
    let server = accept_one(&rt, NamedPipeListener::bind(
        &rt,
        &name,
        &NamedPipeOptions::new(),
    ).unwrap());
    std::thread::sleep(std::time::Duration::from_millis(100));
    let client = NamedPipeConnector::connect(&rt, &name).unwrap();
    let _server = server.join().unwrap().unwrap();
    assert_eq!(
        block_on(client.transact_vec(b"x".to_vec(), 16)).unwrap_err().code(),
        E_ILLEGAL_METHOD_CALL
    );
}

/// A connected byte-mode pair, server end first.
fn byte_pair(rt: &Runtime) -> (NamedPipeStream, NamedPipeStream) {
    let name = unique("own");
    let server = accept_one(rt, NamedPipeListener::bind(
        rt,
        &name,
        &NamedPipeOptions::new(),
    ).unwrap());
    std::thread::sleep(std::time::Duration::from_millis(100));
    let client = NamedPipeConnector::connect(rt, &name).unwrap();
    (server.join().unwrap().unwrap(), client)
}

/// A read lands in spare capacity: no zeroed buffer, and the length ends
/// where the data does.
#[test]
fn a_read_fills_spare_capacity() {
    let rt = Runtime::new();
    let (server, client) = byte_pair(&rt);
    block_on(client.write(b"hello".to_vec())).0.unwrap();
    let (n, got) = block_on(server.read(Vec::with_capacity(64)));
    assert_eq!(n.unwrap(), 5);
    assert_eq!(got, b"hello", "the length did not follow the data");

    // A zero-filled buffer keeps its length, as callers have relied on.
    block_on(client.write(b"abc".to_vec())).0.unwrap();
    let (n, got) = block_on(server.read(vec![0u8; 16]));
    assert_eq!(n.unwrap(), 3);
    assert_eq!(got.len(), 16);
    assert_eq!(&got[..3], b"abc");
}

/// Reading into, and writing out of, part of one buffer.
#[test]
fn slices_read_and_write_part_of_a_buffer() {
    use crate::common::buffer::IoBuf;
    let rt = Runtime::new();
    let (server, client) = byte_pair(&rt);

    // Send bytes 2..5 of a larger frame.
    let (n, frame) = block_on(client.write(b"xxPAYyy".to_vec().slice(2..5)));
    assert_eq!(n.unwrap(), 3);
    assert_eq!(frame.into_inner(), b"xxPAYyy");

    // Append the read after what the buffer already holds.
    let mut buf = Vec::with_capacity(32);
    buf.extend_from_slice(b"head:");
    let (n, rest) = block_on(server.read(buf.slice(5..)));
    assert_eq!(n.unwrap(), 3);
    assert_eq!(rest.into_inner(), b"head:PAY");
}

/// One shared allocation out on two connections, and memory that is
/// never freed.
#[test]
fn shared_and_static_buffers_can_be_sent() {
    let rt = Runtime::new();
    let (server_a, client_a) = byte_pair(&rt);
    let (server_b, client_b) = byte_pair(&rt);
    let shared: Arc<[u8]> = Arc::from(&b"shared"[..]);
    block_on(client_a.write(shared.clone())).0.unwrap();
    block_on(client_b.write(shared.clone())).0.unwrap();
    static BANNER: &[u8] = b"!";
    block_on(client_a.write(BANNER)).0.unwrap();
    let (n, got) = block_on(server_a.read(Vec::with_capacity(16)));
    assert_eq!(&got[..n.unwrap()], b"shared!");
    let (_, got) = block_on(server_b.read(Vec::with_capacity(16)));
    assert_eq!(got, b"shared");
}

/// Memory the crate does not own, kept alive by what does.
#[test]
fn a_raw_buffer_wraps_foreign_memory() {
    use crate::common::buffer::{RawBuf, Region};
    let rt = Runtime::new();
    let (server, client) = byte_pair(&rt);
    let region = Region::new(4096).unwrap();
    let raw = unsafe { RawBuf::from_raw_parts(region.base(), 0, 4096, region) };
    block_on(client.write(b"mapped".to_vec())).0.unwrap();
    let (n, raw) = block_on(server.read(raw));
    assert_eq!(n.unwrap(), 6);
    assert_eq!(raw.as_slice(), b"mapped");
}

#[cfg(feature = "bytes")]
#[test]
fn bytes_buffers_go_both_ways() {
    let rt = Runtime::new();
    let (server, client) = byte_pair(&rt);
    block_on(client.write(bytes::Bytes::from_static(b"from bytes"))).0.unwrap();
    let (n, got) = block_on(server.read(bytes::BytesMut::with_capacity(64)));
    assert_eq!(n.unwrap(), 10);
    assert_eq!(&got[..], b"from bytes");
}

/// A message into a fixed buffer of the caller's: it fits, or it is
/// discarded whole and the next one is untouched.
#[test]
fn messages_into_the_callers_buffer() {
    use crate::common::buffer::SlabPool;
    let rt = Runtime::new();
    let (server, client) = message_pair(&rt, NamedPipeOptions::new());
    let writer = std::thread::spawn(move || {
        for m in [&b"fits"[..], &[3u8; 100], b"next"] {
            block_on(client.write(m.to_vec())).0.unwrap();
        }
        client
    });
    let pool = SlabPool::new(16, 1).unwrap();
    let (n, buf) = block_on(server.read_message(pool.acquire().unwrap()));
    assert_eq!(&buf[..n.unwrap()], b"fits");
    let (err, buf) = block_on(server.read_message(buf));
    assert_eq!(err.unwrap_err().code(), ERROR_MORE_DATA.to_hresult());
    let (n, _) = block_on(server.read_message(buf));
    assert_eq!(n.unwrap(), 4);
    drop(writer.join().unwrap());
}

#[test]
fn transact_into_the_callers_buffers() {
    let rt = Runtime::new();
    let (server, client) = message_pair(&rt, NamedPipeOptions::new());
    let server = answering(server, |m| m.to_ascii_uppercase());
    let request: Arc<[u8]> = Arc::from(&b"shout"[..]);
    let (n, _request, reply) =
        block_on(client.transact(request, Vec::with_capacity(32)));
    assert_eq!(n.unwrap(), 5);
    assert_eq!(reply, b"SHOUT");
    drop(client);
    server.join().unwrap();
}

/// Borrowed buffers both ways: a stack array in, a slice out.
#[test]
fn unchecked_ops_take_borrowed_buffers() {
    let rt = Runtime::new();
    let (server, client) = byte_pair(&rt);
    let request = *b"borrowed";
    let mut reply = [0u8; 16];
    // SAFETY: each future is awaited to completion right here.
    let (n, _) = block_on(unsafe { client.write_unchecked(&request[..]) });
    assert_eq!(n.unwrap(), 8);
    let (n, _) = block_on(unsafe { server.read_unchecked(&mut reply[..]) });
    assert_eq!(&reply[..n.unwrap()], b"borrowed");
}

/// The point of the waiting drop: once a cancelled read's future is gone,
/// the kernel no longer writes into the buffer -- data that arrives later
/// is left in the pipe for the next read, and the buffer keeps what the
/// caller put there.
#[test]
fn a_dropped_unchecked_read_lets_go_of_the_buffer() {
    let rt = Runtime::new();
    let (server, client) = byte_pair(&rt);
    let mut buf = [0u8; 16];
    // SAFETY: the future is dropped by the timeout, never forgotten.
    let timed_out = block_on(timeout(
        &rt,
        std::time::Duration::from_millis(100),
        unsafe { server.read_unchecked(&mut buf[..]) },
    ))
    .is_err();
    assert!(timed_out);
    buf.fill(0xAA);
    block_on(client.write(b"late".to_vec())).0.unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(buf.iter().all(|&b| b == 0xAA), "the kernel wrote after the drop: {buf:?}");
    let (n, got) = block_on(server.read(Vec::with_capacity(16)));
    assert_eq!(&got[..n.unwrap()], b"late", "the late data was lost");
}

/// Dropping on the pool's only thread does not deadlock: the drop waits
/// on the operation's status, not on a callback that thread would have
/// to run.
#[test]
fn a_dropped_unchecked_read_does_not_need_a_pool_thread() {
    let rt = crate::tp::PoolBuilder::new().thread_limits(1, 1).build_runtime();
    let (server, _client) = byte_pair(&rt);
    let (done, finished) = std::sync::mpsc::channel();
    let rt2 = rt.clone();
    let task = rt.spawn(async move {
        let mut buf = [0u8; 16];
        // SAFETY: dropped by the timeout, inside this task.
        let r = timeout(&rt2, std::time::Duration::from_millis(50), unsafe {
            server.read_unchecked(&mut buf[..])
        })
        .await;
        r.is_err()
    });
    std::thread::spawn(move || {
        let _ = done.send(block_on(task));
    });
    let timed_out = finished
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("deadlocked dropping an unchecked read on a one-thread pool");
    assert!(timed_out.unwrap());
}

/// Cancellation racing completion, many times over: whatever wins, the
/// drop returns, and nothing arrives twice or out of order.
///
/// Bytes *can* be lost, and that is not a defect of this API: when the
/// read completes in the same instant its future is dropped, the kernel
/// has already taken the bytes out of the pipe, and the future that would
/// have returned them is gone. Every completion-based read has that
/// property (an owned-buffer read dropped the same way loses its buffer
/// and the data in it). CI caught a 299-of-300 run of an earlier version
/// of this test that asserted otherwise.
#[test]
fn unchecked_drops_race_completions_cleanly() {
    let rt = Runtime::new();
    let (server, client) = byte_pair(&rt);
    let writer = std::thread::spawn(move || {
        for i in 0..300u32 {
            block_on(client.write(vec![(i % 251) as u8])).0.unwrap();
            if i % 7 == 0 {
                std::thread::yield_now();
            }
        }
        client
    });
    let mut seen: Vec<u8> = Vec::new();
    for i in 0..2000u32 {
        let mut buf = [0u8; 4];
        let wait = std::time::Duration::from_micros(u64::from(i % 5) * 200);
        // SAFETY: dropped by the timeout or completed, never forgotten.
        if let Ok((n, _)) = block_on(timeout(&rt, wait, unsafe { server.read_unchecked(&mut buf[..]) })) {
            seen.extend_from_slice(&buf[..n.unwrap()]);
        }
        if seen.len() >= 300 {
            break;
        }
    }
    let client = writer.join().unwrap();
    drop(client);
    // Whatever was left unread arrives now.
    loop {
        let (n, rest) = block_on(server.read(Vec::with_capacity(512)));
        match n {
            Ok(0) | Err(_) => break,
            Ok(n) => seen.extend_from_slice(&rest[..n]),
        }
    }
    // The values were written in order, 0..251 then again from 0: every
    // byte that arrived must continue that order without repeating one.
    assert!(!seen.is_empty() && seen.len() <= 300, "{} bytes", seen.len());
    let mut expected = 0usize;
    for &b in &seen {
        while (expected % 251) as u8 != b {
            expected += 1;
            assert!(expected < 300, "a byte arrived twice or out of order");
        }
        expected += 1;
    }
}
