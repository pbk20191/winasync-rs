use super::*;
use crate::common::block_on;
use std::future::IntoFuture;
use std::sync::mpsc;
use std::time::Duration;
use windows::Foundation::{IClosable, TimeSpan};
use windows::Storage::Streams::{IBuffer, IOutputStream};
use windows::Win32::Foundation::{
    E_INVALIDARG, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_PIPE_BUSY,
};
use windows_core::{AgileReference, HSTRING, Interface};

use super::super::bindings::Pbk20191::WinAsync::Comio::{
    IListenerSecurity, IMessageStream, INamedPipeConnector, INamedPipeListenerSettings,
};

fn pipe_name(tag: &str) -> String {
    format!(r"\\.\pipe\comio_set_{tag}_{}", std::process::id())
}

fn buffer_of(bytes: &[u8]) -> IBuffer {
    let buffer: IBuffer = crate::comio::buffer::new_buffer(bytes.len() as u32)
        .unwrap()
        .cast()
        .unwrap();
    let ptr = crate::comio::buffer::buffer_ptr(&buffer).unwrap();
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
    buffer.SetLength(bytes.len() as u32).unwrap();
    buffer
}

fn bytes_of(buffer: &IBuffer) -> Vec<u8> {
    let ptr = crate::comio::buffer::buffer_ptr(buffer).unwrap();
    let len = buffer.Length().unwrap() as usize;
    unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
}

fn send(stream: &ITransportStream, bytes: &[u8]) {
    let out: IOutputStream = stream.cast().unwrap();
    block_on(out.WriteAsync(&buffer_of(bytes)).unwrap().into_future()).unwrap();
}

/// Delivered server streams, handed to the test thread.
fn stream_handler() -> (ConnectionHandler, mpsc::Receiver<AgileReference<ITransportStream>>) {
    let (tx, rx) = mpsc::channel();
    let handler = ConnectionHandler::new(move |_s, args| {
        let stream = args.as_ref().unwrap().Stream()?;
        let _ = tx.send(AgileReference::new(&stream)?);
        Ok(())
    });
    (handler, rx)
}

/// Connects, retrying while the name is missing: with an instance limit
/// the name does not exist between the last close and the next accept.
fn connect_soon(name: &str) -> ITransportStream {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match block_on(crate::comio::NamedPipeConnector::connect(name)) {
            Err(e)
                if e.code() == ERROR_FILE_NOT_FOUND.to_hresult()
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            other => return other.unwrap(),
        }
    }
}

#[test]
fn settings_round_trip_and_are_fixed_at_bind() {
    let listener = NamedPipeConnectionListener::new();
    let settings: INamedPipeListenerSettings = listener.cast().unwrap();
    assert!(!settings.MessageMode().unwrap());
    assert!(!settings.AcceptRemoteClients().unwrap());
    assert_eq!(settings.MaxInstances().unwrap(), 0);
    assert_eq!(settings.InBufferSize().unwrap(), 4096);
    assert_eq!(settings.OutBufferSize().unwrap(), 4096);

    assert_eq!(settings.SetMaxInstances(255).unwrap_err().code(), E_INVALIDARG);
    settings.SetMaxInstances(3).unwrap();
    assert_eq!(settings.MaxInstances().unwrap(), 3);
    settings.SetMaxInstances(0).unwrap();
    assert_eq!(settings.MaxInstances().unwrap(), 0);
    settings.SetInBufferSize(512).unwrap();
    assert_eq!(settings.InBufferSize().unwrap(), 512);

    // Remote clients need a descriptor, and the bind is where that is
    // checked: the two are set independently.
    settings.SetAcceptRemoteClients(true).unwrap();
    let name = HSTRING::from(pipe_name("fixed"));
    assert_eq!(listener.bind(&name).unwrap_err().code(), E_INVALIDARG);
    let me = crate::common::security::process_user_sid(std::process::id()).unwrap();
    listener
        .cast::<IListenerSecurity>()
        .unwrap()
        .SetSecurityDescriptor(&HSTRING::from(format!("D:P(A;;GA;;;{me})")))
        .unwrap();
    block_on(listener.bind(&name).unwrap().into_future()).unwrap();

    assert_eq!(
        settings.SetMessageMode(true).unwrap_err().code(),
        E_ILLEGAL_METHOD_CALL,
        "a setting changed after the bind"
    );
    listener.cast::<IClosable>().unwrap().Close().unwrap();
}

/// Messages through the WinRT surface: whole, in order, empty and large
/// included; an oversized one discarded without disturbing the next.
#[test]
fn messages_cross_whole() {
    let name = pipe_name("messages");
    let listener = NamedPipeConnectionListener::new();
    listener
        .cast::<INamedPipeListenerSettings>()
        .unwrap()
        .SetMessageMode(true)
        .unwrap();
    // The server end reads on the pool thread it was delivered on.
    let (tx, rx) = mpsc::channel();
    let handler = ConnectionHandler::new(move |_s, args| {
        let stream: IMessageStream = args.as_ref().unwrap().Stream()?.cast()?;
        let _ = tx.send(Ok(vec![stream.IsMessageMode()? as u8]));
        for max in [1 << 20, 1 << 20, 1 << 20, 1 << 20, 10, 10] {
            let got = block_on(stream.ReadMessageAsync(max)?.into_future())
                .map(|b| bytes_of(&b))
                .map_err(|e| e.code());
            let _ = tx.send(got);
        }
        Ok(())
    });
    listener.connection_received(&handler).unwrap();
    block_on(listener.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();

    let client = block_on(crate::comio::NamedPipeConnector::connect(&name)).unwrap();
    assert!(client.cast::<IMessageStream>().unwrap().IsMessageMode().unwrap());
    let big: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
    for m in [&b"hello"[..], &[], &big, b"!", &[7u8; 100_000], b"next"] {
        send(&client, m);
    }

    let next = || rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(next(), Ok(vec![1]), "the server end is not in message mode");
    assert_eq!(next(), Ok(b"hello".to_vec()));
    assert_eq!(next(), Ok(Vec::new()));
    assert_eq!(next(), Ok(big));
    assert_eq!(next(), Ok(b"!".to_vec()));
    assert_eq!(next(), Err(HRESULT::from_win32(ERROR_MORE_DATA.0)));
    assert_eq!(next(), Ok(b"next".to_vec()));
    listener.cast::<IClosable>().unwrap().Close().unwrap();
}

/// `TransactAsync`: request out, whole reply back, in one call -- and a
/// reply too long for the caller discarded without spoiling the next.
#[test]
fn transactions_round_trip() {
    let name = pipe_name("transact");
    let listener = NamedPipeConnectionListener::new();
    listener
        .cast::<INamedPipeListenerSettings>()
        .unwrap()
        .SetMessageMode(true)
        .unwrap();
    // Answers each message on the pool thread it was delivered on.
    let handler = ConnectionHandler::new(move |_s, args| {
        let stream = args.as_ref().unwrap().Stream()?;
        let messages: IMessageStream = stream.cast()?;
        while let Ok(m) = block_on(messages.ReadMessageAsync(1 << 20)?.into_future()) {
            let m = bytes_of(&m);
            let reply = if m == b"big" { vec![5u8; 100_000] } else { m.to_ascii_uppercase() };
            send(&stream, &reply);
        }
        Ok(())
    });
    listener.connection_received(&handler).unwrap();
    block_on(listener.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();

    let client: IMessageStream = connect_soon(&name).cast().unwrap();
    let transact = |request: &[u8], max: u32| {
        block_on(client.TransactAsync(&buffer_of(request), max).unwrap().into_future())
            .map(|b| bytes_of(&b))
            .map_err(|e| e.code())
    };
    assert_eq!(transact(b"ping", 64), Ok(b"PING".to_vec()));
    assert_eq!(transact(b"big", 1 << 20), Ok(vec![5u8; 100_000]));
    assert_eq!(transact(b"big", 10), Err(HRESULT::from_win32(ERROR_MORE_DATA.0)));
    assert_eq!(transact(b"after", 64), Ok(b"AFTER".to_vec()));
    drop(client);
    listener.cast::<IClosable>().unwrap().Close().unwrap();
}

#[test]
fn a_byte_stream_has_no_messages() {
    let (stream, _other) = crate::comio::stream::tests::pipe_pair();
    let messages: IMessageStream = stream.cast().unwrap();
    assert!(!messages.IsMessageMode().unwrap());
    assert_eq!(
        messages.ReadMessageAsync(16).unwrap_err().code(),
        E_ILLEGAL_METHOD_CALL
    );
    assert_eq!(
        messages.TransactAsync(&buffer_of(b"x"), 16).unwrap_err().code(),
        E_ILLEGAL_METHOD_CALL
    );
}

/// At the limit the accept chain parks instead of stopping, and resumes
/// when an instance closes.
#[test]
fn the_chain_waits_out_the_instance_limit() {
    let name = pipe_name("limit");
    let listener = NamedPipeConnectionListener::new();
    listener
        .cast::<INamedPipeListenerSettings>()
        .unwrap()
        .SetMaxInstances(1)
        .unwrap();
    let (handler, rx) = stream_handler();
    listener.connection_received(&handler).unwrap();
    block_on(listener.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();

    let client1 = connect_soon(&name);
    let server1 = rx.recv_timeout(Duration::from_secs(5)).unwrap();

    let err = block_on(crate::comio::NamedPipeConnector::connect_with_timeout(
        &name,
        Duration::ZERO,
    ))
    .unwrap_err();
    assert_eq!(err.code(), ERROR_PIPE_BUSY.to_hresult());

    drop(server1);
    drop(client1);
    let _client2 = connect_soon(&name);
    rx.recv_timeout(Duration::from_secs(5))
        .expect("the chain did not resume after an instance closed");
    listener.cast::<IClosable>().unwrap().Close().unwrap();
}

/// `ConnectWithTimeoutAsync` through the vtable: a busy pipe is busy
/// after the time given, and a negative time is not a time.
#[test]
fn connect_with_timeout_through_the_vtable() {
    let name = pipe_name("timeout");
    let listener = NamedPipeConnectionListener::new();
    listener
        .cast::<INamedPipeListenerSettings>()
        .unwrap()
        .SetMaxInstances(1)
        .unwrap();
    let (handler, rx) = stream_handler();
    listener.connection_received(&handler).unwrap();
    block_on(listener.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();
    let _client1 = connect_soon(&name);
    let _server1 = rx.recv_timeout(Duration::from_secs(5)).unwrap();

    let connector: INamedPipeConnector =
        crate::comio::NamedPipeConnector::new().cast().unwrap();
    let name = HSTRING::from(&name);
    assert_eq!(
        connector
            .ConnectWithTimeoutAsync(&name, TimeSpan { Duration: -1 })
            .unwrap_err()
            .code(),
        E_INVALIDARG
    );
    let started = std::time::Instant::now();
    let err = connector
        .ConnectWithTimeoutAsync(&name, TimeSpan { Duration: 300 * 10_000 })
        .unwrap()
        .join()
        .unwrap_err();
    let waited = started.elapsed();
    assert_eq!(err.code(), ERROR_PIPE_BUSY.to_hresult());
    assert!(
        waited >= Duration::from_millis(250) && waited < Duration::from_secs(3),
        "waited {waited:?} for a 300 ms timeout"
    );
    listener.cast::<IClosable>().unwrap().Close().unwrap();
}

/// The listener's default descriptor is the explicit one, so a reader
/// the system's default would admit is turned away.
#[test]
fn the_default_descriptor_keeps_out_anonymous_readers() {
    use windows::Win32::Security::{ImpersonateAnonymousToken, RevertToSelf};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_SHARE_NONE, OPEN_EXISTING,
    };
    use windows::Win32::System::Threading::GetCurrentThread;
    let name = pipe_name("anon");
    let listener = NamedPipeConnectionListener::new();
    let (handler, rx) = stream_handler();
    listener.connection_received(&handler).unwrap();
    block_on(listener.bind(&HSTRING::from(&name)).unwrap().into_future()).unwrap();

    let opened = std::thread::spawn(move || unsafe {
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
    .unwrap();
    assert_eq!(opened.unwrap_err().code(), ERROR_ACCESS_DENIED.to_hresult());
    assert!(
        rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "a connection was delivered for an anonymous reader"
    );
    listener.cast::<IClosable>().unwrap().Close().unwrap();
}
