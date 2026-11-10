//! An HTTP server on http.sys, the kernel's HTTP stack, over TP_IO.
//!
//! [`HttpServer::bind`] creates a server session, a URL group and a request
//! queue, binds them, and registers the queue's handle with the runtime's
//! pool; every request, body read and response is then one overlapped call
//! on that handle. http.sys does the parsing, the connection management and
//! the port sharing -- several processes can serve different paths under the
//! same port -- and this module turns its records into owned Rust values.
//!
//! # What may be registered
//!
//! Measured as a non-elevated user: `http://localhost:PORT/...` and
//! `http://127.0.0.1:PORT/...` register; the wildcards `http://+:PORT/` and
//! `http://*:PORT/` fail with `ERROR_ACCESS_DENIED` unless the URL has been
//! reserved for the account (`netsh http add urlacl`) or the process is
//! elevated.
//!
//! # Limits http.sys applies first
//!
//! Requests over http.sys's own limits never reach the queue: the client
//! gets a 400 from http.sys and `accept` keeps waiting. The one that showed
//! in testing is the per-header limit (`MaxFieldLength`, 16 KiB by default):
//! a 20,000-byte header was refused that way. They are machine-wide
//! settings under `HKLM\System\CurrentControlSet\Services\HTTP\Parameters`.
//!
//! # Receiving a request too large for the buffer
//!
//! Measured: an overlapped receive into a buffer too small for the request
//! returns `ERROR_IO_PENDING`, and its completion carries `ERROR_MORE_DATA`
//! with the size needed as the byte count; the partial buffer already holds
//! the request's ID, and receiving again *with that ID* into a buffer of that
//! size returns the same request. (Receiving again with no ID, as some
//! samples do, would take the next request instead and lose this one.)

use super::{submit, IoRes, Issue};
use crate::common::buffer::{mark_read, IoBuf, IoBufMut};
use crate::tp::Runtime;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_HANDLE_EOF, ERROR_IO_PENDING, ERROR_MORE_DATA, HANDLE, NO_ERROR, WIN32_ERROR,
};
use windows::Win32::Networking::HttpServer::{
    HttpAddUrlToUrlGroup, HttpCancelHttpRequest, HttpCloseServerSession, HttpCloseUrlGroup,
    HttpCreateRequestQueue, HttpCreateServerSession, HttpCreateUrlGroup, HttpDataChunkFromMemory,
    HttpInitialize, HttpReceiveHttpRequest, HttpReceiveRequestEntityBody, HttpSendHttpResponse,
    HttpSendResponseEntityBody, HttpServerBindingProperty, HttpSetUrlGroupProperty, HttpTerminate,
    HTTPAPI_VERSION, HTTP_BINDING_INFO, HTTP_DATA_CHUNK, HTTP_DATA_CHUNK_0, HTTP_DATA_CHUNK_0_0,
    HTTP_INITIALIZE_SERVER, HTTP_KNOWN_HEADER, HTTP_PROPERTY_FLAGS, HTTP_RECEIVE_HTTP_REQUEST_FLAGS,
    HTTP_REQUEST_FLAG_MORE_ENTITY_BODY_EXISTS, HTTP_REQUEST_V1, HTTP_RESPONSE_V2,
    HTTP_SEND_RESPONSE_FLAG_MORE_DATA, HTTP_UNKNOWN_HEADER, HTTP_VERSION,
};
use windows::Win32::System::IO::OVERLAPPED;
use windows_core::{Error, HSTRING, Result};

const API: HTTPAPI_VERSION = HTTPAPI_VERSION {
    HttpApiMajorVersion: 2,
    HttpApiMinorVersion: 0,
};

/// What the first receive offers a request; the measured `ERROR_MORE_DATA`
/// path grows it when a request needs more.
const FIRST_REQUEST_BUFFER: usize = 4096;

/// A Win32 status from an http.sys call as a `Result`.
fn check(code: u32) -> Result<()> {
    if code == NO_ERROR.0 {
        Ok(())
    } else {
        Err(Error::from_hresult(WIN32_ERROR(code).to_hresult()))
    }
}

/// The same status as an [`Issue`], for an overlapped call. A request or a
/// body chunk larger than the buffer completes with `ERROR_MORE_DATA` --
/// a warning, delivered by the packet like a success.
fn issue(code: u32) -> Issue {
    match code {
        0 => Issue::SyncSuccess,
        c if c == ERROR_IO_PENDING.0 || c == ERROR_MORE_DATA.0 => Issue::Pending,
        c => Issue::Failed(WIN32_ERROR(c).to_hresult()),
    }
}

/// The session and URL group a server owns; closed after the queue.
struct Session {
    session: u64,
    group: u64,
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            if self.group != 0 {
                let _ = HttpCloseUrlGroup(self.group);
            }
            if self.session != 0 {
                let _ = HttpCloseServerSession(self.session);
            }
            let _ = HttpTerminate(HTTP_INITIALIZE_SERVER, None);
        }
    }
}

struct Server {
    // Field order is drop order: the queue closes before its URL group and
    // session.
    queue: Arc<IoRes>,
    _session: Session,
}

/// An http.sys server: a request queue bound to some URLs.
#[derive(Clone)]
pub struct HttpServer {
    inner: Arc<Server>,
}

impl HttpServer {
    /// Serves `urls` -- fully qualified prefixes such as
    /// `http://localhost:8080/app/` -- from a new request queue. See the
    /// module docs for which prefixes need a reservation.
    pub fn bind(rt: &Runtime, urls: &[&str]) -> Result<HttpServer> {
        check(unsafe { HttpInitialize(API, HTTP_INITIALIZE_SERVER, None) })?;
        // From here on `Session::drop` undoes whatever succeeded.
        let mut session = Session { session: 0, group: 0 };
        check(unsafe { HttpCreateServerSession(API, &mut session.session, None) })?;
        check(unsafe { HttpCreateUrlGroup(session.session, &mut session.group, None) })?;
        for url in urls {
            check(unsafe { HttpAddUrlToUrlGroup(session.group, &HSTRING::from(*url), 0, None) })?;
        }
        let mut queue = HANDLE::default();
        check(unsafe { HttpCreateRequestQueue(API, windows_core::PCWSTR::null(), None, None, &mut queue) })?;
        let queue = match IoRes::register(&rt.inner, queue, false) {
            Ok(res) => res,
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(queue);
                }
                return Err(e);
            }
        };
        let binding = HTTP_BINDING_INFO {
            Flags: HTTP_PROPERTY_FLAGS { _bitfield: 1 },
            RequestQueueHandle: queue.handle,
        };
        check(unsafe {
            HttpSetUrlGroupProperty(
                session.group,
                HttpServerBindingProperty,
                (&binding as *const HTTP_BINDING_INFO).cast(),
                size_of::<HTTP_BINDING_INFO>() as u32,
            )
        })?;
        Ok(HttpServer {
            inner: Arc::new(Server {
                queue,
                _session: session,
            }),
        })
    }

    /// Waits for the next request. Dropping the future cancels the wait.
    pub async fn accept(&self) -> Result<HttpRequest> {
        let mut id = 0u64;
        let mut size = FIRST_REQUEST_BUFFER;
        loop {
            // u64 elements: the record holds pointers and 64-bit IDs.
            let buf = vec![0u64; size.div_ceil(8)];
            let handle = self.inner.queue.handle;
            let fut = unsafe {
                submit(&self.inner.queue, buf, move |ovl, buf| {
                    issue(HttpReceiveHttpRequest(
                        handle,
                        id,
                        HTTP_RECEIVE_HTTP_REQUEST_FLAGS(0),
                        buf.as_mut_ptr().cast(),
                        (buf.len() * 8) as u32,
                        None,
                        Some(ovl),
                    ))
                })
            }
            .message();
            let (result, buf) = fut.await;
            let (bytes, more) = result?;
            let record = unsafe { &*(buf.as_ptr() as *const HTTP_REQUEST_V1) };
            if more {
                // Too small: the byte count is the size needed, and the
                // partial record already names the request.
                id = record.RequestId;
                size = bytes.max(size * 2);
                continue;
            }
            return Ok(unsafe { HttpRequest::parse(&self.inner, record) });
        }
    }
}

/// A request's method, as http.sys names it.
fn verb_name(verb: i32) -> Option<&'static str> {
    Some(match verb {
        3 => "OPTIONS",
        4 => "GET",
        5 => "HEAD",
        6 => "POST",
        7 => "PUT",
        8 => "DELETE",
        9 => "TRACE",
        10 => "CONNECT",
        11 => "TRACK",
        12 => "MOVE",
        13 => "COPY",
        14 => "PROPFIND",
        15 => "PROPPATCH",
        16 => "MKCOL",
        17 => "LOCK",
        18 => "UNLOCK",
        19 => "SEARCH",
        _ => return None,
    })
}

/// `HTTP_HEADER_ID` names for requests: the general and entity headers
/// (0-19) then the request ones.
const REQUEST_HEADERS: [&str; 41] = [
    "Cache-Control", "Connection", "Date", "Keep-Alive", "Pragma", "Trailer",
    "Transfer-Encoding", "Upgrade", "Via", "Warning", "Allow", "Content-Length",
    "Content-Type", "Content-Encoding", "Content-Language", "Content-Location",
    "Content-MD5", "Content-Range", "Expires", "Last-Modified", "Accept",
    "Accept-Charset", "Accept-Encoding", "Accept-Language", "Authorization", "Cookie",
    "Expect", "From", "Host", "If-Match", "If-Modified-Since", "If-None-Match",
    "If-Range", "If-Unmodified-Since", "Max-Forwards", "Proxy-Authorization", "Referer",
    "Range", "TE", "Translate", "User-Agent",
];

/// And for responses: the same first 20, then the response ones.
const RESPONSE_HEADERS: [&str; 30] = [
    "Cache-Control", "Connection", "Date", "Keep-Alive", "Pragma", "Trailer",
    "Transfer-Encoding", "Upgrade", "Via", "Warning", "Allow", "Content-Length",
    "Content-Type", "Content-Encoding", "Content-Language", "Content-Location",
    "Content-MD5", "Content-Range", "Expires", "Last-Modified", "Accept-Ranges", "Age",
    "ETag", "Location", "Proxy-Authenticate", "Retry-After", "Server", "Set-Cookie",
    "Vary", "WWW-Authenticate",
];

/// Bytes at `ptr` as text (http.sys hands header text over as bytes).
unsafe fn ascii(ptr: *const u8, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(ptr, len) }).into_owned()
}

/// `len` *bytes* of UTF-16 at `ptr`.
unsafe fn wide(ptr: *const u16, len_bytes: usize) -> String {
    if ptr.is_null() || len_bytes == 0 {
        return String::new();
    }
    String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(ptr, len_bytes / 2) })
}

unsafe fn address(sa: *const windows::Win32::Networking::WinSock::SOCKADDR) -> Option<SocketAddr> {
    use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6, SOCKADDR_IN, SOCKADDR_IN6, SOCKADDR_STORAGE};
    if sa.is_null() {
        return None;
    }
    let family = unsafe { (*sa).sa_family };
    let len = if family == AF_INET {
        size_of::<SOCKADDR_IN>()
    } else if family == AF_INET6 {
        size_of::<SOCKADDR_IN6>()
    } else {
        return None;
    };
    let mut storage = SOCKADDR_STORAGE::default();
    unsafe {
        core::ptr::copy_nonoverlapping(sa.cast::<u8>(), (&mut storage as *mut SOCKADDR_STORAGE).cast::<u8>(), len)
    };
    crate::common::sockaddr::read_sockaddr(&storage).ok()
}

/// One request, read out of http.sys's record into owned values.
///
/// Respond with [`respond`](Self::respond), or with
/// [`start_response`](Self::start_response) and
/// [`send_body`](Self::send_body). A request dropped without a response is
/// cancelled (`HttpCancelHttpRequest`), which ends its connection, rather
/// than left for http.sys to time out.
pub struct HttpRequest {
    server: Arc<Server>,
    id: u64,
    /// The method, upper case.
    pub method: String,
    /// The request's URL as received, before http.sys decoded it.
    pub raw_url: String,
    /// The decoded URL, scheme and host included.
    pub url: String,
    /// The decoded path, without the query.
    pub path: String,
    /// The query, `?` included; empty if there is none.
    pub query: String,
    pub version: (u16, u16),
    /// In arrival order: the headers http.sys knows by ID first, then the
    /// rest.
    pub headers: Vec<(String, String)>,
    pub remote_addr: Option<SocketAddr>,
    pub local_addr: Option<SocketAddr>,
    pub connection_id: u64,
    has_body: bool,
    responded: AtomicBool,
}

impl HttpRequest {
    /// # Safety
    /// `record` is a complete `HTTP_REQUEST_V1` whose pointers point into
    /// the buffer it sits in, which is alive for the call.
    unsafe fn parse(server: &Arc<Server>, record: &HTTP_REQUEST_V1) -> HttpRequest {
        unsafe {
            let method = match verb_name(record.Verb.0) {
                Some(name) => name.to_owned(),
                None => ascii(record.pUnknownVerb.0, record.UnknownVerbLength as usize),
            };
            let cooked = &record.CookedUrl;
            let mut headers = Vec::new();
            for (i, known) in record.Headers.KnownHeaders.iter().enumerate() {
                if known.RawValueLength != 0 || !known.pRawValue.is_null() {
                    let value = ascii(known.pRawValue.0, known.RawValueLength as usize);
                    if known.RawValueLength != 0 {
                        headers.push((REQUEST_HEADERS[i].to_owned(), value));
                    }
                }
            }
            let unknown = record.Headers.pUnknownHeaders;
            for i in 0..record.Headers.UnknownHeaderCount as usize {
                let h = &*unknown.add(i);
                headers.push((
                    ascii(h.pName.0, h.NameLength as usize),
                    ascii(h.pRawValue.0, h.RawValueLength as usize),
                ));
            }
            HttpRequest {
                server: server.clone(),
                id: record.RequestId,
                method,
                raw_url: ascii(record.pRawUrl.0, record.RawUrlLength as usize),
                url: wide(cooked.pFullUrl.0, cooked.FullUrlLength as usize),
                path: wide(cooked.pAbsPath.0, cooked.AbsPathLength as usize),
                query: wide(cooked.pQueryString.0, cooked.QueryStringLength as usize),
                version: (record.Version.MajorVersion, record.Version.MinorVersion),
                headers,
                remote_addr: address(record.Address.pRemoteAddress),
                local_addr: address(record.Address.pLocalAddress),
                connection_id: record.ConnectionId,
                has_body: record.Flags & HTTP_REQUEST_FLAG_MORE_ENTITY_BODY_EXISTS != 0,
                responded: AtomicBool::new(false),
            }
        }
    }

    /// The first header named `name`, ignoring case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Reads the next part of the body into `buf`; resolves to the bytes
    /// read, 0 at the end, and hands the buffer back.
    pub fn read_body<B: IoBufMut + 'static>(
        &self,
        buf: B,
    ) -> impl Future<Output = (Result<usize>, B)> + Send + use<B> {
        let handle = self.server.queue.handle;
        let id = self.id;
        let fut = if self.has_body {
            Ok(unsafe {
                submit(&self.server.queue, buf, move |ovl, buf| {
                    issue(HttpReceiveRequestEntityBody(
                        handle,
                        id,
                        0,
                        buf.stable_mut_ptr().cast(),
                        IoBuf::bytes_total(buf) as u32,
                        None,
                        Some(ovl),
                    ))
                })
            })
        } else {
            Err(buf)
        };
        async move {
            let (result, buf) = match fut {
                Ok(fut) => fut.await,
                Err(buf) => return (Ok(0), buf),
            };
            let result = match result {
                Err(e) if e.code() == ERROR_HANDLE_EOF.to_hresult() => Ok(0),
                other => other,
            };
            let n = result.as_ref().ok().copied();
            (result, mark_read(n, buf))
        }
    }

    /// The whole body, up to `max` bytes; fails with `ERROR_MORE_DATA` if it
    /// is longer.
    pub async fn read_body_to_end(&self, max: usize) -> Result<Vec<u8>> {
        let mut body = Vec::new();
        if !self.has_body {
            return Ok(body);
        }
        loop {
            let room = (max - body.len()).clamp(1, 64 * 1024);
            let start = body.len();
            body.reserve_exact(room);
            let (n, rest) = self.read_body(body.slice(start..start + room)).await;
            body = rest.into_inner();
            if n? == 0 {
                return Ok(body);
            }
            if body.len() >= max {
                // Full: anything more is too much.
                let (n, _) = self.read_body(vec![0u8; 1]).await;
                if n? != 0 {
                    return Err(Error::new(ERROR_MORE_DATA.to_hresult(), "the body is longer than allowed"));
                }
                return Ok(body);
            }
        }
    }

    /// Sends the whole response: `response`'s status and headers, then
    /// `body`. A `Content-Length` is added if the response has none.
    pub async fn respond<B: IoBuf + 'static>(&self, response: Response, body: B) -> (Result<()>, B) {
        let mut response = response;
        // 1xx, 204 and 304 carry no body, and so no length either.
        let bodiless = response.status < 200 || response.status == 204 || response.status == 304;
        if !bodiless && !response.headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("content-length")) {
            response.headers.push(("Content-Length".into(), IoBuf::len(&body).to_string()));
        }
        let (r, body) = self.send_head(response, Some(body), false).await;
        (r, body.unwrap())
    }

    /// Sends the status and headers only; the body follows through
    /// [`send_body`](Self::send_body). Give a `Content-Length` -- or
    /// `Transfer-Encoding: chunked` and chunk the body yourself -- or the
    /// client cannot tell where the body ends except by the connection
    /// closing.
    pub async fn start_response(&self, response: Response) -> Result<()> {
        self.send_head::<Vec<u8>>(response, None, true).await.0
    }

    /// Sends part of the body; `last` ends the response.
    pub async fn send_body<B: IoBuf + 'static>(&self, chunk: B, last: bool) -> (Result<()>, B) {
        let handle = self.server.queue.handle;
        let id = self.id;
        let flags = if last { 0 } else { HTTP_SEND_RESPONSE_FLAG_MORE_DATA };
        let fut = unsafe {
            submit(&self.server.queue, (chunk, HTTP_DATA_CHUNK::default()), move |ovl, (chunk, data)| {
                *data = memory_chunk(chunk.stable_ptr(), IoBuf::len(chunk));
                issue(HttpSendResponseEntityBody(
                    handle,
                    id,
                    flags,
                    Some(core::slice::from_ref(data)),
                    None,
                    None,
                    None,
                    Some(ovl),
                    None,
                ))
            })
        };
        let (result, (chunk, _)) = fut.await;
        (result.map(|_| ()), chunk)
    }

    async fn send_head<B: IoBuf + 'static>(
        &self,
        response: Response,
        body: Option<B>,
        more: bool,
    ) -> (Result<()>, Option<B>) {
        self.responded.store(true, Ordering::Release);
        let handle = self.server.queue.handle;
        let id = self.id;
        let payload = ResponsePayload::new(response, body);
        let flags = if more { HTTP_SEND_RESPONSE_FLAG_MORE_DATA } else { 0 };
        let fut = unsafe {
            submit(&self.server.queue, payload, move |ovl, p| {
                // Pointers are set here, at the payload's final address.
                p.wire();
                issue(HttpSendHttpResponse(
                    handle,
                    id,
                    flags,
                    &p.response,
                    None,
                    None,
                    None,
                    None,
                    Some(ovl),
                    None,
                ))
            })
        };
        let (result, payload) = fut.await;
        (result.map(|_| ()), payload.body)
    }
}

impl Drop for HttpRequest {
    fn drop(&mut self) {
        if !self.responded.load(Ordering::Acquire) {
            cancel_request(self.server.queue.handle, self.id);
        }
    }
}

/// `HttpCancelHttpRequest`, waited for here. The queue is bound to the
/// pool's port, so the call carries an event with its low bit set to keep
/// its completion off the port.
fn cancel_request(queue: HANDLE, id: u64) {
    use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};
    let Ok(event) = (unsafe { CreateEventW(None, true, false, None) }) else {
        return;
    };
    let mut ovl = OVERLAPPED {
        hEvent: HANDLE((event.0 as usize | 1) as *mut core::ffi::c_void),
        ..Default::default()
    };
    let code = unsafe { HttpCancelHttpRequest(queue, id, Some(&mut ovl)) };
    if code == ERROR_IO_PENDING.0 {
        unsafe { WaitForSingleObject(event, INFINITE) };
    }
    unsafe {
        let _ = CloseHandle(event);
    }
}

fn memory_chunk(ptr: *const u8, len: usize) -> HTTP_DATA_CHUNK {
    HTTP_DATA_CHUNK {
        DataChunkType: HttpDataChunkFromMemory,
        Anonymous: HTTP_DATA_CHUNK_0 {
            FromMemory: HTTP_DATA_CHUNK_0_0 {
                pBuffer: ptr as *mut core::ffi::c_void,
                BufferLength: len as u32,
            },
        },
    }
}

/// A response's status and headers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
}

impl Response {
    /// `status`, with the standard reason phrase where there is one.
    pub fn new(status: u16) -> Response {
        let reason = match status {
            200 => "OK",
            201 => "Created",
            204 => "No Content",
            301 => "Moved Permanently",
            302 => "Found",
            304 => "Not Modified",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            500 => "Internal Server Error",
            503 => "Service Unavailable",
            _ => "",
        };
        Response {
            status,
            reason: reason.to_owned(),
            headers: Vec::new(),
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Response {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn reason(mut self, reason: &str) -> Response {
        self.reason = reason.to_owned();
        self
    }
}

/// A response as http.sys wants it, owning everything its pointers name.
struct ResponsePayload<B> {
    response: HTTP_RESPONSE_V2,
    reason: Vec<u8>,
    /// `(known index or None, name, value)`, as bytes.
    headers: Vec<(Option<usize>, Vec<u8>, Vec<u8>)>,
    unknown: Vec<HTTP_UNKNOWN_HEADER>,
    chunk: HTTP_DATA_CHUNK,
    body: Option<B>,
}

impl<B: IoBuf> ResponsePayload<B> {
    fn new(response: Response, body: Option<B>) -> Self {
        let mut taken = [false; 30];
        let headers = response
            .headers
            .into_iter()
            .map(|(name, value)| {
                // A known header goes in its slot -- once; a repeat (a
                // second Set-Cookie, say) goes with the unknown ones.
                let known = RESPONSE_HEADERS
                    .iter()
                    .position(|k| k.eq_ignore_ascii_case(&name))
                    .filter(|&i| !std::mem::replace(&mut taken[i], true));
                (known, name.into_bytes(), value.into_bytes())
            })
            .collect();
        ResponsePayload {
            response: HTTP_RESPONSE_V2::default(),
            reason: response.reason.into_bytes(),
            headers,
            unknown: Vec::new(),
            chunk: HTTP_DATA_CHUNK::default(),
            body,
        }
        .with_status(response.status)
    }

    fn with_status(mut self, status: u16) -> Self {
        self.response.Base.StatusCode = status;
        self
    }

    /// Points the response at the payload's own storage.
    fn wire(&mut self) {
        let base = &mut self.response.Base;
        base.Version = HTTP_VERSION {
            MajorVersion: 1,
            MinorVersion: 1,
        };
        base.ReasonLength = self.reason.len() as u16;
        base.pReason = windows_core::PCSTR(self.reason.as_ptr());
        self.unknown.clear();
        for (known, name, value) in &self.headers {
            match known {
                Some(i) => {
                    base.Headers.KnownHeaders[*i] = HTTP_KNOWN_HEADER {
                        RawValueLength: value.len() as u16,
                        pRawValue: windows_core::PCSTR(value.as_ptr()),
                    }
                }
                None => self.unknown.push(HTTP_UNKNOWN_HEADER {
                    NameLength: name.len() as u16,
                    RawValueLength: value.len() as u16,
                    pName: windows_core::PCSTR(name.as_ptr()),
                    pRawValue: windows_core::PCSTR(value.as_ptr()),
                }),
            }
        }
        base.Headers.UnknownHeaderCount = self.unknown.len() as u16;
        base.Headers.pUnknownHeaders = self.unknown.as_mut_ptr();
        // No chunk at all for an empty body: http.sys refuses a zero-length
        // one with E_INVALIDARG (measured, on a 204).
        if let Some(body) = self.body.as_ref().filter(|b| IoBuf::len(*b) != 0) {
            self.chunk = memory_chunk(body.stable_ptr(), IoBuf::len(body));
            base.EntityChunkCount = 1;
            base.pEntityChunks = &mut self.chunk;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tp::block_on;
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
        let rt = Runtime::new();
        let (port, url) = prefix("get");
        let server = HttpServer::bind(&rt, &[&url]).unwrap();
        let c = client(
            port,
            format!(
                "GET /get/hello%20there?x=1 HTTP/1.1\r\nHost: localhost:{port}\r\nX-Test: yes\r\nUser-Agent: probe\r\nConnection: close\r\n\r\n"
            )
            .into_bytes(),
        );
        let req = block_on(server.accept()).unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/get/hello there");
        assert_eq!(req.query, "?x=1");
        assert_eq!(req.raw_url, "/get/hello%20there?x=1");
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
        let rt = Runtime::new();
        let (port, url) = prefix("post");
        let server = HttpServer::bind(&rt, &[&url]).unwrap();
        let payload: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let mut request = format!(
            "POST /post/ HTTP/1.1\r\nHost: localhost:{port}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        )
        .into_bytes();
        request.extend_from_slice(&payload);
        let c = client(port, request);
        let req = block_on(server.accept()).unwrap();
        assert_eq!(req.method, "POST");
        let body = block_on(req.read_body_to_end(1 << 20)).unwrap();
        assert_eq!(body, payload);
        block_on(req.respond(Response::new(204), Vec::new())).0.unwrap();
        let (head, _) = split(&c.join().unwrap());
        assert!(head.starts_with("HTTP/1.1 204"), "{head}");
    }

    /// Larger than the first buffer: received again by ID, not lost.
    #[test]
    fn a_large_request_is_received_whole() {
        let rt = Runtime::new();
        let (port, url) = prefix("big");
        let server = HttpServer::bind(&rt, &[&url]).unwrap();
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
        let rt = Runtime::new();
        let (port, url) = prefix("stream");
        let server = HttpServer::bind(&rt, &[&url]).unwrap();
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
        let rt = Runtime::new();
        let (port, url) = prefix("drop");
        let server = HttpServer::bind(&rt, &[&url]).unwrap();
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
        let rt = Runtime::new();
        let (port, url) = prefix("cancel");
        let server = HttpServer::bind(&rt, &[&url]).unwrap();
        assert!(block_on(crate::tp::timeout(&rt, Duration::from_millis(100), server.accept())).is_err());
        let c = client(
            port,
            format!("GET /cancel/ HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n").into_bytes(),
        );
        let req = block_on(server.accept()).unwrap();
        block_on(req.respond(Response::new(200), b"after".to_vec())).0.unwrap();
        assert_eq!(split(&c.join().unwrap()).1, b"after");
    }
}
