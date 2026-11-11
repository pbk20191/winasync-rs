use super::*;
use crate::common::block_on;
use std::io::{Read, Write};
use std::time::Duration;

/// A port for this test, and the prefix to serve on it.
fn prefix(tag: &str) -> (u16, String) {
    use std::sync::atomic::AtomicU16;
    static NEXT: AtomicU16 = AtomicU16::new(0);
    let port = 41000 + (std::process::id() % 2000) as u16 * 3 + NEXT.fetch_add(1, Ordering::Relaxed);
    (port, format!("http://localhost:{port}/{tag}/"))
}

/// A raw HTTP/1.1 exchange from a plain socket: what is sent is exactly
/// what the test says.
fn client(port: u16, request: Vec<u8>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(&request).unwrap();
        let mut response = Vec::new();
        let _ = s.read_to_end(&mut response);
        response
    })
}

fn split(response: &[u8]) -> (String, Vec<u8>) {
    let at = response.windows(4).position(|w| w == b"\r\n\r\n").expect("no end of headers");
    (String::from_utf8_lossy(&response[..at]).into_owned(), response[at + 4..].to_vec())
}

#[test]
fn a_get_is_parsed_and_answered() {
    let (port, url) = prefix("get");
    let server = HttpServer::bind(&[&url]).unwrap();
    let c = client(
        port,
        format!(
            "GET /get/hello%20there?x=1 HTTP/1.1\r\nHost: localhost:{port}\r\nX-Test: yes\r\nUser-Agent: probe\r\nConnection: close\r\n\r\n"
        )
        .into_bytes(),
    );
    let req = block_on(server.accept()).unwrap();
    assert_eq!(req.method(), "GET");
    assert_eq!(req.path, "/get/hello there");
    assert_eq!(req.query, "?x=1");
    assert_eq!(req.raw_url(), "/get/hello%20there?x=1");
    assert_eq!(req.version, (1, 1));
    assert_eq!(req.header("x-test"), Some("yes"));
    assert_eq!(req.header("User-Agent"), Some("probe"));
    assert!(req.remote_addr.unwrap().ip().is_loopback());
    let response = Response::new(200).header("Content-Type", "text/plain").header("X-Served", "1");
    block_on(req.respond(response, b"hi".to_vec())).0.unwrap();
    let (head, body) = split(&c.join().unwrap());
    assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
    assert!(head.contains("Content-Length: 2"), "{head}");
    assert!(head.contains("Content-Type: text/plain"), "{head}");
    assert!(head.contains("X-Served: 1"), "{head}");
    assert_eq!(body, b"hi");
}

#[test]
fn a_body_is_read_whole() {
    let (port, url) = prefix("post");
    let server = HttpServer::bind(&[&url]).unwrap();
    let payload: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let mut request = format!(
        "POST /post/ HTTP/1.1\r\nHost: localhost:{port}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    )
    .into_bytes();
    request.extend_from_slice(&payload);
    let c = client(port, request);
    let req = block_on(server.accept()).unwrap();
    assert_eq!(req.method(), "POST");
    let body = block_on(req.read_body_to_end(1 << 20)).unwrap();
    assert_eq!(body, payload);
    block_on(req.respond(Response::new(204), Vec::new())).0.unwrap();
    let (head, _) = split(&c.join().unwrap());
    assert!(head.starts_with("HTTP/1.1 204"), "{head}");
}

/// Larger than the first buffer: received again by ID, not lost.
#[test]
fn a_large_request_is_received_whole() {
    let (port, url) = prefix("big");
    let server = HttpServer::bind(&[&url]).unwrap();
    // Under http.sys's default per-header limit (16 KiB); a longer header
    // is refused with a 400 before the request reaches the queue.
    let big = "y".repeat(12_000);
    let c = client(
        port,
        format!("GET /big/ HTTP/1.1\r\nHost: localhost:{port}\r\nX-Big: {big}\r\nConnection: close\r\n\r\n")
            .into_bytes(),
    );
    let req = block_on(server.accept()).unwrap();
    assert_eq!(req.header("X-Big").map(str::len), Some(12_000));
    block_on(req.respond(Response::new(200), b"ok".to_vec())).0.unwrap();
    let (head, body) = split(&c.join().unwrap());
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert_eq!(body, b"ok");
}

#[test]
fn a_response_streams_in_parts() {
    let (port, url) = prefix("stream");
    let server = HttpServer::bind(&[&url]).unwrap();
    let c = client(
        port,
        format!("GET /stream/ HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n").into_bytes(),
    );
    let req = block_on(server.accept()).unwrap();
    block_on(req.start_response(Response::new(200).header("Content-Length", "10"))).unwrap();
    block_on(req.send_body(b"01234".to_vec(), false)).0.unwrap();
    block_on(req.send_body(b"56789".to_vec(), true)).0.unwrap();
    let (head, body) = split(&c.join().unwrap());
    assert!(head.contains("Content-Length: 10"), "{head}");
    assert_eq!(body, b"0123456789");
}

/// A request dropped unanswered is cancelled, which ends the
/// connection rather than leaving the client waiting.
#[test]
fn an_unanswered_request_is_cancelled() {
    let (port, url) = prefix("drop");
    let server = HttpServer::bind(&[&url]).unwrap();
    let c = client(
        port,
        format!("GET /drop/ HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n").into_bytes(),
    );
    let req = block_on(server.accept()).unwrap();
    let started = std::time::Instant::now();
    drop(req);
    let response = c.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(5), "the client waited for a timeout");
    eprintln!("client after cancel got {} bytes", response.len());
}

/// A waiting accept dropped by a timeout leaves the server usable.
#[test]
fn a_dropped_accept_is_cancelled() {
    let (port, url) = prefix("cancel");
    let server = HttpServer::bind(&[&url]).unwrap();
    // Polled once, so the receive is issued, then dropped while pending.
    let mut accept = Box::pin(server.accept());
    let waker = std::task::Waker::noop();
    let polled = accept.as_mut().poll(&mut std::task::Context::from_waker(waker));
    assert!(polled.is_pending());
    drop(accept);
    let c = client(
        port,
        format!("GET /cancel/ HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n").into_bytes(),
    );
    let req = block_on(server.accept()).unwrap();
    block_on(req.respond(Response::new(200), b"after".to_vec())).0.unwrap();
    assert_eq!(split(&c.join().unwrap()).1, b"after");
}

/// One GET for `path` answered with `body`, over a fresh connection.
fn round_trip(server: &HttpServer, port: u16, path: &str, body: &[u8]) -> (String, Vec<u8>) {
    let c = client(port, get(port, path));
    let req = block_on(server.accept()).unwrap();
    block_on(req.respond(Response::new(200), body.to_vec())).0.unwrap();
    split(&c.join().unwrap())
}

fn get(port: u16, path: &str) -> Vec<u8> {
    format!("GET {path} HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n").into_bytes()
}

/// The status line a client gets for `path` when no handler answers:
/// http.sys's own response.
fn unanswered(port: u16, path: &str) -> String {
    let response = client(port, get(port, path)).join().unwrap();
    String::from_utf8_lossy(&response).lines().next().unwrap_or("").to_owned()
}

/// Never run: only has to compile, which it does only if every future a
/// handler awaits can move to another thread.
#[allow(dead_code)]
fn futures_are_send(server: &HttpServer, req: &HttpRequest, file: std::fs::File) {
    fn send<T: Send>(_: T) {}
    send(server.accept());
    send(req.read_body(vec![0u8; 1]));
    send(req.read_body_to_end(1));
    send(req.respond(Response::new(200), Vec::new()));
    send(req.respond_file(Response::new(200), file, 0, None));
    send(req.start_response(Response::new(200)));
    send(req.send_body(Vec::new(), true));
    send(req.wait_for_disconnect());
}

#[test]
fn a_file_is_sent_whole_and_in_part() {
    let (port, url) = prefix("file");
    let server = HttpServer::bind(&[&url]).unwrap();
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
    let path = std::env::temp_dir().join(format!("winasync-http-file-{}", std::process::id()));
    std::fs::write(&path, &data).unwrap();
    let file = std::fs::File::open(&path).unwrap();

    let c = client(port, get(port, "/file/"));
    let req = block_on(server.accept()).unwrap();
    let (r, file) = block_on(req.respond_file(Response::new(200), file, 0, None));
    r.unwrap();
    let (head, body) = split(&c.join().unwrap());
    assert!(head.contains("Content-Length: 300000"), "{head}");
    assert!(body == data, "the whole file differs");

    let c = client(port, get(port, "/file/"));
    let req = block_on(server.accept()).unwrap();
    let (r, file) = block_on(req.respond_file(Response::new(200), file, 1000, Some(20)));
    r.unwrap();
    let (head, body) = split(&c.join().unwrap());
    assert!(head.contains("Content-Length: 20"), "{head}");
    assert_eq!(body, &data[1000..1020]);

    // Streamed: memory, then the file from an offset to its end.
    let c = client(port, get(port, "/file/"));
    let req = block_on(server.accept()).unwrap();
    let len = 5 + (data.len() - 299_990);
    block_on(req.start_response(Response::new(200).header("Content-Length", len.to_string()))).unwrap();
    block_on(req.send_body(b"head:".to_vec(), false)).0.unwrap();
    let (r, file) = block_on(req.send_file(file, 299_990, None, true));
    r.unwrap();
    let (_, body) = split(&c.join().unwrap());
    assert_eq!(&body[..5], b"head:");
    assert_eq!(&body[5..], &data[299_990..]);
    drop(file);
    let _ = std::fs::remove_file(path);
}

/// An empty last part sends nothing, and still ends the response.
#[test]
fn an_empty_last_part_ends_the_response() {
    let (port, url) = prefix("end");
    let server = HttpServer::bind(&[&url]).unwrap();
    let c = client(port, get(port, "/end/"));
    let req = block_on(server.accept()).unwrap();
    block_on(req.start_response(Response::new(200).header("Content-Length", "5"))).unwrap();
    block_on(req.send_body(b"hello".to_vec(), false)).0.unwrap();
    block_on(req.send_body(Vec::new(), true)).0.unwrap();
    assert_eq!(split(&c.join().unwrap()).1, b"hello");
}

#[test]
fn a_closed_connection_is_noticed() {
    let (port, url) = prefix("gone");
    let server = HttpServer::bind(&[&url]).unwrap();
    let c = std::thread::spawn(move || {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(format!("GET /gone/ HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n").as_bytes()).unwrap();
        // Kept alive: read the response, then hang up.
        let mut got = Vec::new();
        let mut buf = [0u8; 512];
        while !got.ends_with(b"ok") {
            let n = s.read(&mut buf).unwrap();
            assert!(n != 0, "closed before the response");
            got.extend_from_slice(&buf[..n]);
        }
    });
    let req = block_on(server.accept()).unwrap();
    // Issued now, before the client can be gone.
    let gone = req.wait_for_disconnect();
    block_on(req.respond(Response::new(200), b"ok".to_vec())).0.unwrap();
    c.join().unwrap();
    let started = std::time::Instant::now();
    block_on(gone).unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_shut_down_server_takes_no_more_requests() {
    let (_, url) = prefix("shut");
    let server = HttpServer::bind(&[&url]).unwrap();
    let mut waiting = Box::pin(server.accept());
    let waker = std::task::Waker::noop();
    assert!(waiting.as_mut().poll(&mut std::task::Context::from_waker(waker)).is_pending());
    server.shutdown().unwrap();
    let e = block_on(waiting).err().expect("the waiting accept survived");
    eprintln!("waiting accept after shutdown: {e}");
    assert!(block_on(server.accept()).is_err(), "a later accept succeeded");
}

#[test]
fn a_disabled_server_is_answered_by_http_sys() {
    let (port, url) = prefix("off");
    let server = HttpServer::bind(&[&url]).unwrap();
    server.set_503_verbosity(Verbosity503::Limited).unwrap();
    server.set_enabled(false).unwrap();
    let line = unanswered(port, "/off/");
    eprintln!("disabled server answered: {line}");
    assert!(line.contains(" 503 "), "{line}");
    server.set_enabled(true).unwrap();
    assert_eq!(round_trip(&server, port, "/off/", b"on").1, b"on");
}

#[test]
fn prefixes_come_and_go() {
    let (port, url) = prefix("one");
    let server = HttpServer::bind(&[&url]).unwrap();
    let two = format!("http://localhost:{port}/two/");
    server.add_url(&two).unwrap();
    assert_eq!(round_trip(&server, port, "/two/", b"2").1, b"2");
    server.remove_url(&two).unwrap();
    let line = unanswered(port, "/two/");
    eprintln!("removed prefix answered: {line}");
    assert!(line.contains(" 404 ") || line.contains(" 503 "), "{line}");
    assert_eq!(round_trip(&server, port, "/one/", b"1").1, b"1");
}

/// A kept-alive connection gone idle after a request is closed by
/// http.sys once the timeout set here runs out, not after its default
/// two minutes. (Measured: one that never sends a request is not -- it
/// has not reached any server yet, so no server's timeouts apply to it.)
#[test]
fn timeouts_apply() {
    let (port, url) = prefix("slow");
    let server = HttpServer::bind(&[&url]).unwrap();
    server
        .set_timeouts(&Timeouts {
            idle_connection: Some(Duration::from_secs(1)),
            ..Timeouts::default()
        })
        .unwrap();
    server.set_queue_length(10).unwrap();
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(90))).unwrap();
    s.write_all(format!("GET /slow/ HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n").as_bytes()).unwrap();
    let req = block_on(server.accept()).unwrap();
    block_on(req.respond(Response::new(200), b"ok".to_vec())).0.unwrap();
    let mut got = Vec::new();
    let mut buf = [0u8; 512];
    while !got.ends_with(b"ok") {
        let n = s.read(&mut buf).unwrap();
        assert!(n != 0, "closed before the response");
        got.extend_from_slice(&buf[..n]);
    }
    let started = std::time::Instant::now();
    let mut rest = Vec::new();
    let _ = s.read_to_end(&mut rest);
    let waited = started.elapsed();
    eprintln!("idle connection closed after {waited:?}");
    // Measured: 5 s here, 10 s on a GitHub runner, against a default of
    // 120 s. Only "well under the default" is asserted.
    assert!(waited < Duration::from_secs(60), "{waited:?}");
}

#[cfg(feature = "tp")]
#[test]
fn a_server_runs_on_a_tp_runtime() {
    let rt = crate::tp::Runtime::new();
    let (port, url) = prefix("tp");
    let server = HttpServer::bind_on(&rt, &[&url]).unwrap();
    // The server keeps the pool.
    drop(rt);
    assert_eq!(round_trip(&server, port, "/tp/", b"tp").1, b"tp");
}

#[cfg(feature = "iocp")]
#[test]
fn a_server_runs_on_a_callers_port() {
    use crate::iocp::{Completion, CompletionPort};
    use std::sync::atomic::AtomicUsize;
    let cp = Arc::new(CompletionPort::new(1).unwrap());
    // The crate's key cannot be posted under.
    let forged = Completion {
        key: crate::common::overlapped::port_key(),
        overlapped: 0x1234,
        ..Completion::default()
    };
    assert!(cp.post(forged).is_err());

    let stop = Arc::new(AtomicBool::new(false));
    let dispatched = Arc::new(AtomicUsize::new(0));
    let pump = {
        let (cp, stop, dispatched) = (cp.clone(), stop.clone(), dispatched.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                if let Some(c) = cp.dequeue(Some(Duration::from_millis(50))).unwrap() {
                    assert!(cp.dispatch(&c), "a completion that is not the crate's");
                    dispatched.fetch_add(1, Ordering::Relaxed);
                }
            }
        })
    };
    let (port, url) = prefix("iocp");
    let server = HttpServer::bind_to_port(&cp, &[&url]).unwrap();
    assert_eq!(round_trip(&server, port, "/iocp/", b"port").1, b"port");
    drop(server);
    stop.store(true, Ordering::Release);
    pump.join().unwrap();
    // The receive and the response, at least.
    assert!(dispatched.load(Ordering::Relaxed) >= 2);
    // Someone else's completion is left alone.
    assert!(!cp.dispatch(&Completion { key: 7, overlapped: 0x1234, ..Completion::default() }));
}

/// What a plain request carries beyond V1: measured, not assumed.
#[test]
fn a_request_carries_its_v2_info() {
    let (port, url) = prefix("v2");
    let server = HttpServer::bind(&[&url]).unwrap();
    let c = client(port, get(port, "/v2/"));
    let req = block_on(server.accept()).unwrap();
    eprintln!(
        "v2: http2={} ip_routed={} auth={:?} ssl={:?} timing={} other={:?}",
        req.http2,
        req.ip_routed,
        req.auth.as_ref().map(|a| a.status),
        req.ssl_protocol,
        req.timing.len(),
        req.other_info.iter().map(|(t, b)| (*t, b.len())).collect::<Vec<_>>()
    );
    assert!(!req.http2 && !req.http3);
    assert!(req.ssl_protocol.is_none(), "TLS parameters on plain HTTP");
    assert!(req.auth.is_none(), "an auth record with authentication off");
    block_on(req.respond(Response::new(200), b"ok".to_vec())).0.unwrap();
    c.join().unwrap();
}

/// Accepts until a request arrives authenticated, answering the
/// challenge to the rest -- the server's half of the exchange. Gives up
/// after 15 s rather than hang on a client that stopped.
fn accept_authenticated(server: &HttpServer, schemes: AuthSchemes) -> Option<HttpRequest> {
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    let started = std::time::Instant::now();
    loop {
        let mut accept = Box::pin(server.accept());
        let req = loop {
            if let std::task::Poll::Ready(r) = accept.as_mut().poll(&mut cx) {
                break r.unwrap();
            }
            if started.elapsed() > Duration::from_secs(15) {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let auth = req.auth.as_ref().expect("no auth record with authentication on");
        eprintln!("auth round: {:?} {:?} package={:?} sec={:?}", auth.status, auth.scheme, auth.package, auth.sec_status);
        if auth.status == AuthStatus::Success {
            return Some(req);
        }
        block_on(req.respond(Response::challenge(schemes, ""), Vec::new())).0.unwrap();
    }
}

/// Basic: a request with no credentials is delivered, marked; the server
/// challenges; wrong credentials are delivered marked as failed.
#[test]
fn basic_credentials_are_checked_by_http_sys() {
    let (port, url) = prefix("basic");
    let server = HttpServer::bind(&[&url]).unwrap();
    server.set_authentication(AuthSchemes::BASIC, "winasync", "").unwrap();

    let c = client(port, get(port, "/basic/"));
    let req = block_on(server.accept()).unwrap();
    let auth = req.auth.as_ref().expect("no auth record");
    assert_eq!(auth.status, AuthStatus::NotAuthenticated);
    block_on(req.respond(Response::challenge(AuthSchemes::BASIC, "winasync"), Vec::new())).0.unwrap();
    let (head, _) = split(&c.join().unwrap());
    assert!(head.starts_with("HTTP/1.1 401"), "{head}");
    assert!(head.contains("WWW-Authenticate: Basic realm=\"winasync\""), "{head}");

    // bogus:nope
    let c = client(
        port,
        format!(
            "GET /basic/ HTTP/1.1\r\nHost: localhost:{port}\r\nAuthorization: Basic Ym9ndXM6bm9wZQ==\r\nConnection: close\r\n\r\n"
        )
        .into_bytes(),
    );
    let req = block_on(server.accept()).unwrap();
    let auth = req.auth.as_ref().expect("no auth record");
    eprintln!("basic, bad credentials: {:?} {:?} {:?}", auth.status, auth.scheme, auth.sec_status);
    assert_eq!(auth.status, AuthStatus::Failure);
    assert_eq!(auth.scheme, AuthScheme::Basic);
    assert!(auth.token.is_none());
    block_on(req.respond(Response::new(403), b"no".to_vec())).0.unwrap();
    let (head, _) = split(&c.join().unwrap());
    assert!(head.starts_with("HTTP/1.1 403"), "{head}");
}

/// Negotiate against the local machine. The client is WinHTTP through
/// PowerShell, which authenticates on loopback with the caller's logon;
/// the system curl cannot (no NTLM in its build, and Negotiate finds
/// no default credentials for it). Measured: two rounds -- the first
/// `NotAuthenticated`, then, after the challenge and http.sys's own NTLM
/// leg, `Success` with `Negotiate`/`NTLM` and the token.
#[test]
fn negotiate_delivers_the_callers_token() {
    let (port, url) = prefix("nego");
    let server = HttpServer::bind(&[&url]).unwrap();
    let schemes = AuthSchemes::NEGOTIATE | AuthSchemes::NTLM;
    server.set_authentication(schemes, "", "").unwrap();
    let script = format!("(Invoke-WebRequest -UseBasicParsing -UseDefaultCredentials 'http://localhost:{port}/nego/').StatusCode");
    let c = std::thread::spawn(move || {
        std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .unwrap()
    });
    let req = match accept_authenticated(&server, schemes) {
        Some(req) => req,
        None => {
            let out = c.join().unwrap();
            panic!("no authenticated request; client said {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        }
    };
    let auth = req.auth.as_ref().unwrap();
    eprintln!("negotiate: {:?} package={} attrs={:#x}", auth.scheme, auth.package, auth.context_attributes);
    assert_eq!(auth.scheme, AuthScheme::Negotiate);
    assert!(auth.token.is_some(), "no access token");
    assert!(!auth.package.is_empty());
    block_on(req.respond(Response::new(200), b"hello".to_vec())).0.unwrap();
    let out = c.join().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "200", "{}", String::from_utf8_lossy(&out.stderr));
}

/// Polls `accept` for up to `wait`; `None` if no request came.
fn accept_within(server: &HttpServer, wait: Duration) -> Option<HttpRequest> {
    let mut accept = Box::pin(server.accept());
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    let started = std::time::Instant::now();
    loop {
        if let std::task::Poll::Ready(r) = accept.as_mut().poll(&mut cx) {
            return Some(r.unwrap());
        }
        if started.elapsed() > wait {
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_named_queue_is_shared_with_whoever_opens_it() {
    let (port, url) = prefix("named");
    let name = format!("winasync-test-{}-{port}", std::process::id());
    let owner = HttpServer::bind_named(&name, &[&url]).unwrap();
    assert!(HttpServer::bind_named(&name, &[&url]).is_err(), "the name was given out twice");
    let worker = HttpServer::open(&name).unwrap();
    assert!(HttpServer::open("winasync-no-such-queue").is_err());
    // The worker takes a request to the owner's URL and answers it.
    let c = client(port, get(port, "/named/"));
    let req = block_on(worker.accept()).unwrap();
    block_on(req.respond(Response::new(200), b"worker".to_vec())).0.unwrap();
    assert_eq!(split(&c.join().unwrap()).1, b"worker");
    // And the owner still can.
    assert_eq!(round_trip(&owner, port, "/named/", b"owner").1, b"owner");
}

/// A cached response is served by http.sys until it is flushed.
#[test]
fn a_cached_response_never_reaches_accept_again() {
    let (port, url) = prefix("cache");
    let server = HttpServer::bind(&[&url]).unwrap();
    let c = client(port, get(port, "/cache/x"));
    let req = block_on(server.accept()).unwrap();
    let response = Response::new(200).header("Content-Type", "text/plain").cache_until_flushed();
    block_on(req.respond(response, b"cached".to_vec())).0.unwrap();
    assert_eq!(split(&c.join().unwrap()).1, b"cached");

    let c = client(port, get(port, "/cache/x"));
    let again = accept_within(&server, Duration::from_secs(1));
    let (head, body) = split(&c.join().unwrap());
    eprintln!("second GET: delivered={} head={head}", again.is_some());
    assert!(again.is_none(), "the cached URL reached accept");
    assert_eq!(body, b"cached");

    server.flush_cache(&format!("http://localhost:{port}/cache/x"), false).unwrap();
    assert_eq!(round_trip(&server, port, "/cache/x", b"fresh").1, b"fresh");
}

#[test]
fn a_fragment_is_sent_from_the_cache() {
    let (port, url) = prefix("frag");
    let server = HttpServer::bind(&[&url]).unwrap();
    let name = format!("http://localhost:{port}/frag/piece");
    server.add_fragment(&name, b"from the cache", None).unwrap();

    let c = client(port, get(port, "/frag/"));
    let req = block_on(server.accept()).unwrap();
    block_on(req.respond_fragment(Response::new(200), &name)).unwrap();
    let (head, body) = split(&c.join().unwrap());
    eprintln!("fragment response head:\n{head}");
    assert_eq!(body, b"from the cache");

    let c = client(port, get(port, "/frag/"));
    let req = block_on(server.accept()).unwrap();
    block_on(req.start_response(Response::new(200).header("Content-Length", "20"))).unwrap();
    block_on(req.send_body(b"head: ".to_vec(), false)).0.unwrap();
    // Measured on a GitHub runner: the fragment was gone after the one
    // response that used it (ERROR_PATH_NOT_FOUND); locally it stayed. A
    // cache may drop an entry whenever it likes, so the caller re-adds.
    if let Err(e) = block_on(req.send_fragment(&name, true)) {
        eprintln!("fragment gone after one use ({e:?}); adding it again");
        server.add_fragment(&name, b"from the cache", None).unwrap();
        block_on(req.send_fragment(&name, true)).unwrap();
    }
    assert_eq!(split(&c.join().unwrap()).1, b"head: from the cache");

    server.flush_cache(&name, false).unwrap();
    let c = client(port, get(port, "/frag/"));
    let req = block_on(server.accept()).unwrap();
    let r = block_on(req.respond_fragment(Response::new(200), &name));
    eprintln!("flushed fragment: {r:?}");
    assert!(r.is_err(), "a flushed fragment was sent");
    drop(req);
    let _ = c.join();
}

/// Digest with credentials http.sys cannot verify (curl `--digest`):
/// the realm-only challenge comes back completed, and the answer is
/// delivered marked as failed. Measured: a bare 401 gets no challenge
/// appended, for Digest as for the others.
#[test]
fn digest_is_verified_by_http_sys() {
    let curl = std::path::PathBuf::from(std::env::var("SystemRoot").unwrap()).join("System32").join("curl.exe");
    if !curl.exists() {
        eprintln!("no system curl; skipped");
        return;
    }
    let (port, url) = prefix("digest");
    let server = HttpServer::bind(&[&url]).unwrap();
    server.set_authentication(AuthSchemes::DIGEST, "winasync", "localhost").unwrap();
    let target = format!("http://localhost:{port}/digest/");
    let c = std::thread::spawn(move || {
        std::process::Command::new(curl)
            .args(["--digest", "-u", "bogus:nope", "-v", "-s", "-o", "NUL", "-w", "%{http_code}", &target])
            .output()
            .unwrap()
    });
    let mut rounds = Vec::new();
    let outcome = loop {
        let Some(req) = accept_within(&server, Duration::from_secs(10)) else {
            break None;
        };
        let auth = req.auth.as_ref().expect("no auth record");
        rounds.push((auth.status, auth.scheme, auth.sec_status));
        if auth.status != AuthStatus::NotAuthenticated {
            assert!(auth.token.is_none());
            block_on(req.respond(Response::new(403), b"no".to_vec())).0.unwrap();
            break Some((auth.status, auth.scheme));
        }
        block_on(req.respond(Response::challenge(AuthSchemes::DIGEST, "winasync"), Vec::new())).0.unwrap();
    };
    let out = c.join().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    eprintln!("digest rounds {rounds:?}; curl {}", String::from_utf8_lossy(&out.stdout));
    assert_eq!(outcome, Some((AuthStatus::Failure, AuthScheme::Digest)), "{err}");
    // The challenge the client saw was http.sys's completed one.
    assert!(err.contains("nonce=") && err.contains("qop=\"auth\""), "{err}");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "403");
}

/// Whether this process is elevated: the SSL binding needs it.
fn is_elevated() -> bool {
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token = HANDLE::default();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }
    let mut info = TOKEN_ELEVATION::default();
    let mut len = 0u32;
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut info as *mut TOKEN_ELEVATION).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    };
    unsafe {
        let _ = CloseHandle(token);
    }
    ok.is_ok() && info.TokenIsElevated != 0
}

/// HTTPS end to end when elevated: a self-signed certificate, bound to
/// the port, serving `https://localhost`. Unelevated, measures what the
/// binding call says.
#[test]
fn https_serves_with_a_bound_certificate() {
    let (port, _) = prefix("tls");
    let url = format!("https://localhost:{port}/tls/");
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse().unwrap();
    let app_id = windows_core::GUID::from_u128(0x6f2d2f3a_7b1e_4c3c_9d1a_5e8b1c0a2f11);
    if !is_elevated() {
        let fake = [0u8; 20];
        let e = ssl::bind_certificate(addr, &fake, app_id, "MY").unwrap_err();
        eprintln!("unelevated bind_certificate: {e:?}");
        let q = ssl::query_certificate(addr);
        eprintln!("unelevated query_certificate: {q:?}");
        let reg = HttpServer::bind(&[&url]);
        eprintln!("unelevated https:// registration: {:?}", reg.as_ref().err());
        return;
    }
    let ps = |script: &str| {
        let out = std::process::Command::new("powershell.exe").args(["-NoProfile", "-Command", script]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    let thumb = ps("(New-SelfSignedCertificate -DnsName localhost -CertStoreLocation Cert:\\LocalMachine\\My -FriendlyName winasync-test).Thumbprint");
    let hash: Vec<u8> = (0..thumb.len()).step_by(2).map(|i| u8::from_str_radix(&thumb[i..i + 2], 16).unwrap()).collect();
    assert_eq!(hash.len(), 20);
    let cleanup = |thumb: &str| {
        let _ = ssl::unbind_certificate(addr);
        ps(&format!("Remove-Item Cert:\\LocalMachine\\My\\{thumb}"));
    };
    if let Err(e) = ssl::bind_certificate(addr, &hash, app_id, "MY") {
        cleanup(&thumb);
        panic!("bind_certificate: {e:?}");
    }
    let bound = ssl::query_certificate(addr).unwrap();
    eprintln!("bound: {bound:?}");
    assert_eq!(bound.as_ref().map(|b| b.hash.as_slice()), Some(hash.as_slice()));

    let server = match HttpServer::bind(&[&url]) {
        Ok(s) => s,
        Err(e) => {
            cleanup(&thumb);
            panic!("https bind: {e:?}");
        }
    };
    let curl = std::path::PathBuf::from(std::env::var("SystemRoot").unwrap()).join("System32").join("curl.exe");
    let target = url.clone();
    let c = std::thread::spawn(move || {
        std::process::Command::new(curl).args(["-k", "-s", &target]).output().unwrap()
    });
    let req = accept_within(&server, Duration::from_secs(15));
    let Some(req) = req else {
        let out = c.join().unwrap();
        cleanup(&thumb);
        panic!("no https request; curl: {}", String::from_utf8_lossy(&out.stderr));
    };
    eprintln!("https request: ssl_protocol={:?}", req.ssl_protocol);
    let tls = req.ssl_protocol;
    block_on(req.respond(Response::new(200), b"secure".to_vec())).0.unwrap();
    let out = c.join().unwrap();
    drop(server);
    cleanup(&thumb);
    assert_eq!(out.stdout, b"secure");
    assert!(tls.is_some(), "no TLS parameters on an https request");
}
