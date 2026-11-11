use super::*;
use std::io::{Read, Write};
use windows::Storage::Streams::Buffer;
use windows_core::Interface;

/// A port of this process's own, above the `http` module's tests
/// (41000 + 3 * (pid % 2000), to ~47000) and below 49152, where the
/// ephemeral range begins: `HttpAddUrl` answers `ERROR_SHARING_VIOLATION`
/// for a port a transient client socket happens to hold, which the old
/// `47000 + 3 * (pid % 2000)` reached for two thirds of all pids (seen
/// once in six local runs, 2026-11-11).
fn port() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(0);
    47100 + (std::process::id() % 640) as u16 * 3 + NEXT.fetch_add(1, Ordering::Relaxed)
}

#[test]
fn a_request_round_trips_through_the_winrt_surface() {
    let port = port();
    let server: rc::HttpServer = HttpServer::new().into_interface();
    let iserver: rc::IHttpServer = server.cast().unwrap();
    iserver.BindAsync(&HSTRING::from(format!("http://localhost:{port}/winrt/"))).unwrap().join().unwrap();
    assert!(iserver.Information().unwrap().to_string().contains("/winrt/"));

    let body = b"hello winrt";
    let client = std::thread::spawn(move || {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(
            format!(
                "POST /winrt/x?a=1 HTTP/1.1\r\nHost: localhost:{port}\r\nX-Probe: yes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .unwrap();
        s.write_all(body).unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).unwrap();
        out
    });

    let req: rc::HttpRequest = iserver.AcceptAsync().unwrap().join().unwrap();
    let ireq: rc::IHttpRequest = req.cast().unwrap();
    assert_eq!(ireq.Method().unwrap(), "POST");
    assert_eq!(ireq.Path().unwrap(), "/winrt/x");
    assert_eq!(ireq.Query().unwrap(), "?a=1");
    assert_eq!(ireq.Header(&HSTRING::from("x-probe")).unwrap(), "yes");
    assert!(ireq.Headers().unwrap().to_string().contains("X-Probe: yes\r\n"));
    assert!(ireq.HasBody().unwrap());
    assert_eq!(ireq.AuthStatus().unwrap(), 0);

    let buf: IBuffer = Buffer::Create(64).unwrap().into();
    let n = ireq.ReadBodyAsync(&buf).unwrap().join().unwrap();
    assert_eq!(n as usize, body.len());
    assert_eq!(buf.Length().unwrap() as usize, body.len());
    let got = unsafe { std::slice::from_raw_parts(super::super::buffer::buffer_ptr(&buf).unwrap(), n as usize) };
    assert_eq!(got, body);

    let reply: IBuffer = Buffer::Create(8).unwrap().into();
    unsafe { std::ptr::copy_nonoverlapping(b"ok!".as_ptr(), super::super::buffer::buffer_ptr(&reply).unwrap(), 3) };
    reply.SetLength(3).unwrap();
    ireq.RespondAsync(200, &HSTRING::from("Content-Type: text/plain\r\nX-Served: 1"), &reply).unwrap().join().unwrap();
    // Answered: the request is gone.
    assert_eq!(ireq.Method().unwrap_err().code(), RO_E_CLOSED);

    let response = client.join().unwrap();
    let text = String::from_utf8_lossy(&response);
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    assert!(text.contains("X-Served: 1"), "{text}");
    assert!(text.ends_with("ok!"), "{text}");
}

#[test]
fn a_cancelled_accept_and_a_closed_server() {
    let port = port();
    let server: rc::HttpServer = HttpServer::new().into_interface();
    let iserver: rc::IHttpServer = server.cast().unwrap();
    assert!(iserver.AcceptAsync().is_err(), "accept before bind");
    iserver.BindAsync(&HSTRING::from(format!("http://localhost:{port}/winrt2/"))).unwrap().join().unwrap();
    let op = iserver.AcceptAsync().unwrap();
    op.Cancel().unwrap();
    assert_eq!(op.join().unwrap_err().code(), super::super::asyncop::E_CANCELLED);
    let closable: windows::Foundation::IClosable = server.cast().unwrap();
    closable.Close().unwrap();
    assert!(iserver.AcceptAsync().is_err(), "accept after close");
}
