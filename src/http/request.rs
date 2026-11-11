//! One request: read out of http.sys's record in place, answered through
//! the queue. The body sources a response can be built from live here too.

use crate::common::buffer::BufResult;
use super::*;

/// A request's method, as http.sys names it.
pub(super) fn verb_name(verb: i32) -> Option<&'static str> {
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
pub(super) const REQUEST_HEADERS: [&str; 41] = [
    "Cache-Control", "Connection", "Date", "Keep-Alive", "Pragma", "Trailer",
    "Transfer-Encoding", "Upgrade", "Via", "Warning", "Allow", "Content-Length",
    "Content-Type", "Content-Encoding", "Content-Language", "Content-Location",
    "Content-MD5", "Content-Range", "Expires", "Last-Modified", "Accept",
    "Accept-Charset", "Accept-Encoding", "Accept-Language", "Authorization", "Cookie",
    "Expect", "From", "Host", "If-Match", "If-Modified-Since", "If-None-Match",
    "If-Range", "If-Unmodified-Since", "Max-Forwards", "Proxy-Authorization", "Referer",
    "Range", "TE", "Translate", "User-Agent",
];

/// Bytes at `ptr` as text (http.sys hands header text over as bytes).
/// `len` bytes at `ptr`, borrowed for `'a` -- the receive buffer's life.
pub(super) unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        return &[];
    }
    unsafe { core::slice::from_raw_parts(ptr, len) }
}

/// [`bytes`] as text; empty if it is not UTF-8.
pub(super) unsafe fn ascii<'a>(ptr: *const u8, len: usize) -> &'a str {
    core::str::from_utf8(unsafe { bytes(ptr, len) }).unwrap_or("")
}

/// `len` *bytes* of UTF-16 at `ptr`.
pub(super) unsafe fn wide(ptr: *const u16, len_bytes: usize) -> String {
    if ptr.is_null() || len_bytes == 0 {
        return String::new();
    }
    String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(ptr, len_bytes / 2) })
}

pub(super) unsafe fn address(sa: *const windows::Win32::Networking::WinSock::SOCKADDR) -> Option<SocketAddr> {
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

/// # Safety
/// `info` is the auth record of a request being parsed, whose pointers are
/// valid for the call; the token handle is taken over.
pub(super) unsafe fn parse_auth(info: &HTTP_REQUEST_AUTH_INFO) -> AuthInfo {
    let status = match info.AuthStatus {
        s if s == HttpAuthStatusSuccess => AuthStatus::Success,
        s if s == HttpAuthStatusNotAuthenticated => AuthStatus::NotAuthenticated,
        s if s == HttpAuthStatusFailure => AuthStatus::Failure,
        s => AuthStatus::Other(s.0),
    };
    let scheme = match info.AuthType {
        t if t == HttpRequestAuthTypeBasic => AuthScheme::Basic,
        t if t == HttpRequestAuthTypeDigest => AuthScheme::Digest,
        t if t == HttpRequestAuthTypeNTLM => AuthScheme::Ntlm,
        t if t == HttpRequestAuthTypeNegotiate => AuthScheme::Negotiate,
        t if t == HttpRequestAuthTypeKerberos => AuthScheme::Kerberos,
        t if t.0 == 0 => AuthScheme::None,
        t => AuthScheme::Other(t.0),
    };
    unsafe {
        AuthInfo {
            status,
            sec_status: info.SecStatus,
            scheme,
            token: (!info.AccessToken.is_invalid()).then(|| OwnedHandle::from_raw_handle(info.AccessToken.0)),
            package: wide(info.pPackageName.0, info.PackageNameLength as usize),
            context_attributes: info.ContextAttributes,
        }
    }
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
    /// The record as received. Its pointers point into this same
    /// allocation, which a `Vec` keeps in place however the `Vec` moves;
    /// the method, raw URL and headers are read from it in place.
    buf: Vec<u64>,
    id: u64,
    /// The decoded URL, scheme and host included.
    pub url: String,
    /// The decoded path, without the query.
    pub path: String,
    /// The query, `?` included; empty if there is none.
    pub query: String,
    pub version: (u16, u16),
    pub remote_addr: Option<SocketAddr>,
    pub local_addr: Option<SocketAddr>,
    pub connection_id: u64,
    /// Arrived over HTTP/2.
    pub http2: bool,
    /// Arrived over HTTP/3.
    pub http3: bool,
    /// Routed by IP: the URL was registered for the address it came to
    /// rather than a host name.
    pub ip_routed: bool,
    /// Who sent it, when [`HttpServer::set_authentication`] is on.
    ///
    /// http.sys does not challenge on its own (measured, and documented
    /// under "Authentication (HTTP Server API)"). A request with no
    /// credentials is delivered marked [`AuthStatus::NotAuthenticated`], and
    /// the server either serves it anonymously or answers
    /// [`Response::challenge`] -- a 401 naming the schemes; http.sys appends
    /// nothing to a bare 401. The client's next request, with credentials,
    /// is http.sys's: it runs the interim legs of NTLM and Negotiate itself
    /// and delivers the request once more, [`AuthStatus::Success`] with the
    /// caller's token, or [`AuthStatus::Failure`] with the security status
    /// and no token, for the server to decide on (challenge again, or serve
    /// anonymously). A malformed `Authorization` header gets a 400 from
    /// http.sys and is never delivered. Digest works the same way, with
    /// http.sys completing the challenge header (see
    /// [`Response::challenge`]) and verifying the answer.
    pub auth: Option<AuthInfo>,
    /// The connection's TLS parameters, on HTTPS.
    pub ssl_protocol: Option<SslProtocol>,
    /// http.sys's timestamps for the request, in `QueryPerformanceCounter`
    /// ticks, indexed by `HTTP_REQUEST_TIMING_TYPE`; 0 where a step did not
    /// happen. Empty on a Windows without them.
    pub timing: Vec<u64>,
    /// Request infos this crate does not parse, as `(HTTP_REQUEST_INFO_TYPE,
    /// bytes)` -- the open end: TCP info, QUIC stats, token binding, sizing.
    pub other_info: Vec<(i32, Vec<u8>)>,
    has_body: bool,
    responded: AtomicBool,
}

impl HttpRequest {
    /// # Safety
    /// `buf` holds a complete `HTTP_REQUEST_V2` whose pointers point into
    /// `buf`.
    pub(super) unsafe fn parse(server: &Arc<Server>, buf: Vec<u64>) -> HttpRequest {
        let record = unsafe { &*(buf.as_ptr() as *const HTTP_REQUEST_V2) };
        unsafe {
            let mut auth = None;
            let mut ssl_protocol = None;
            let mut timing = Vec::new();
            let mut other_info = Vec::new();
            for i in 0..record.RequestInfoCount as usize {
                let info: &HTTP_REQUEST_INFO = &*record.pRequestInfo.add(i);
                let len = info.InfoLength as usize;
                match info.InfoType {
                    t if t == HttpRequestInfoTypeAuth && len >= size_of::<HTTP_REQUEST_AUTH_INFO>() => {
                        auth = Some(parse_auth(&*info.pInfo.cast::<HTTP_REQUEST_AUTH_INFO>()));
                    }
                    t if t == HttpRequestInfoTypeSslProtocol && len >= size_of::<HTTP_SSL_PROTOCOL_INFO>() => {
                        let p = &*info.pInfo.cast::<HTTP_SSL_PROTOCOL_INFO>();
                        ssl_protocol = Some(SslProtocol {
                            protocol: p.Protocol,
                            cipher: p.CipherType,
                            cipher_strength: p.CipherStrength,
                            hash: p.HashType,
                            hash_strength: p.HashStrength,
                            key_exchange: p.KeyExchangeType,
                            key_exchange_strength: p.KeyExchangeStrength,
                        });
                    }
                    t if t == HttpRequestInfoTypeRequestTiming && len >= size_of::<HTTP_REQUEST_TIMING_INFO>() => {
                        let p = &*info.pInfo.cast::<HTTP_REQUEST_TIMING_INFO>();
                        let n = (p.RequestTimingCount as usize).min(p.RequestTiming.len());
                        timing = p.RequestTiming[..n].to_vec();
                    }
                    t => {
                        let bytes = if info.pInfo.is_null() || len == 0 {
                            Vec::new()
                        } else {
                            core::slice::from_raw_parts(info.pInfo.cast::<u8>(), len).to_vec()
                        };
                        other_info.push((t.0, bytes));
                    }
                }
            }
            let record = &record.Base;
            let cooked = &record.CookedUrl;
            HttpRequest {
                server: server.clone(),
                id: record.RequestId,
                url: wide(cooked.pFullUrl.0, cooked.FullUrlLength as usize),
                path: wide(cooked.pAbsPath.0, cooked.AbsPathLength as usize),
                query: wide(cooked.pQueryString.0, cooked.QueryStringLength as usize),
                version: (record.Version.MajorVersion, record.Version.MinorVersion),
                remote_addr: address(record.Address.pRemoteAddress),
                local_addr: address(record.Address.pLocalAddress),
                connection_id: record.ConnectionId,
                http2: record.Flags & HTTP_REQUEST_FLAG_HTTP2 != 0,
                http3: record.Flags & HTTP_REQUEST_FLAG_HTTP3 != 0,
                ip_routed: record.Flags & HTTP_REQUEST_FLAG_IP_ROUTED != 0,
                auth,
                ssl_protocol,
                timing,
                other_info,
                has_body: record.Flags & HTTP_REQUEST_FLAG_MORE_ENTITY_BODY_EXISTS != 0,
                responded: AtomicBool::new(false),
                buf,
            }
        }
    }

    /// Whether a body follows the headers.
    pub fn has_body(&self) -> bool {
        self.has_body
    }

    fn base(&self) -> &HTTP_REQUEST_V1 {
        unsafe { &(*(self.buf.as_ptr() as *const HTTP_REQUEST_V2)).Base }
    }

    /// The method, upper case.
    pub fn method(&self) -> &str {
        let r = self.base();
        match verb_name(r.Verb.0) {
            Some(name) => name,
            None => unsafe { ascii(r.pUnknownVerb.0, r.UnknownVerbLength as usize) },
        }
    }

    /// The request's URL as received, before http.sys decoded it.
    pub fn raw_url(&self) -> &str {
        let r = self.base();
        unsafe { ascii(r.pRawUrl.0, r.RawUrlLength as usize) }
    }

    /// The headers, in place: the ones http.sys knows by ID first, then
    /// the rest. Values are bytes, as the client sent them.
    pub fn headers(&self) -> impl Iterator<Item = (&str, &[u8])> {
        let h = &self.base().Headers;
        let known = h
            .KnownHeaders
            .iter()
            .enumerate()
            .filter(|(_, k)| k.RawValueLength != 0)
            .map(|(i, k)| (REQUEST_HEADERS[i], unsafe { bytes(k.pRawValue.0, k.RawValueLength as usize) }));
        let unknown = if h.pUnknownHeaders.is_null() {
            &[][..]
        } else {
            unsafe { core::slice::from_raw_parts(h.pUnknownHeaders, h.UnknownHeaderCount as usize) }
        };
        let unknown = unknown.iter().map(|u| unsafe {
            (ascii(u.pName.0, u.NameLength as usize), bytes(u.pRawValue.0, u.RawValueLength as usize))
        });
        known.chain(unknown)
    }

    /// The first header named `name`, ignoring case.
    pub fn header_bytes(&self, name: &str) -> Option<&[u8]> {
        self.headers().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v)
    }

    /// The first header named `name`, ignoring case, as text; `None` if
    /// absent or not UTF-8 (see [`header_bytes`](Self::header_bytes)).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.header_bytes(name).and_then(|v| core::str::from_utf8(v).ok())
    }

    /// Reads the next part of the body into `buf`; resolves to the bytes
    /// read, 0 at the end, and hands the buffer back.
    pub fn read_body<B: IoBufMut + 'static>(
        &self,
        buf: B,
    ) -> impl Future<Output = BufResult<usize, B>> + Send + use<B> {
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
    pub async fn respond<B: IoBuf + 'static>(&self, response: Response, body: B) -> BufResult<(), B> {
        let response = with_length(response, IoBuf::len(&body) as u64);
        let (r, body) = self.send_head(response, Some(Memory(body)), false).await;
        (r, body.unwrap().0)
    }

    /// Sends the whole response with part of a file as the body: `len`
    /// bytes from `start`, or to the end with `None`. http.sys reads the
    /// file itself, in the kernel, so the bytes never pass through this
    /// process. A `Content-Length` is added if the response has none; the
    /// file is handed back when the response is sent.
    pub async fn respond_file<F: AsRawHandle + Send + 'static>(
        &self,
        response: Response,
        file: F,
        start: u64,
        len: Option<u64>,
    ) -> BufResult<(), F> {
        let len = match len {
            Some(len) => len,
            None => match file_size(&file) {
                Ok(size) => size.saturating_sub(start),
                Err(e) => return (Err(e), file),
            },
        };
        let response = with_length(response, len);
        let (r, body) = self.send_head(response, Some(FileRange { file, start, len }), false).await;
        (r, body.unwrap().file)
    }

    /// Sends the whole response with a fragment [`HttpServer::add_fragment`]
    /// put in http.sys's cache as the body. No `Content-Length` is added:
    /// http.sys knows the fragment's size.
    pub async fn respond_fragment(&self, response: Response, name: &str) -> Result<()> {
        let fragment = Fragment(name.encode_utf16().collect());
        self.send_head(response, Some(fragment), false).await.0
    }

    /// Sends a cached fragment as part of the body; `last` ends the
    /// response.
    pub async fn send_fragment(&self, name: &str, last: bool) -> Result<()> {
        let fragment = Fragment(name.encode_utf16().collect());
        self.send_part(fragment, last).await.0
    }

    /// Sends the status and headers only; the body follows through
    /// [`send_body`](Self::send_body) and [`send_file`](Self::send_file).
    /// Give a `Content-Length` -- or `Transfer-Encoding: chunked` and chunk
    /// the body yourself -- or the client cannot tell where the body ends
    /// except by the connection closing.
    pub async fn start_response(&self, response: Response) -> Result<()> {
        self.send_head::<Memory<Vec<u8>>>(response, None, true).await.0
    }

    /// Sends part of the body; `last` ends the response. An empty `chunk`
    /// sends nothing, so `send_body(Vec::new(), true)` just ends it.
    pub async fn send_body<B: IoBuf + 'static>(&self, chunk: B, last: bool) -> BufResult<(), B> {
        let (r, chunk) = self.send_part(Memory(chunk), last).await;
        (r, chunk.0)
    }

    /// Sends part of the body from a file, as
    /// [`respond_file`](Self::respond_file) does; `None` sends to the end of
    /// the file.
    pub async fn send_file<F: AsRawHandle + Send + 'static>(
        &self,
        file: F,
        start: u64,
        len: Option<u64>,
        last: bool,
    ) -> BufResult<(), F> {
        let len = len.unwrap_or(HTTP_BYTE_RANGE_TO_EOF);
        let (r, part) = self.send_part(FileRange { file, start, len }, last).await;
        (r, part.file)
    }

    /// Resolves when the client's connection closes -- the way to notice a
    /// client gone while a response is being worked out. It may come from
    /// either side: a response sent with `Connection: close` ends the
    /// connection too.
    pub fn wait_for_disconnect(&self) -> impl Future<Output = Result<()>> + Send + use<> {
        let handle = self.server.queue.handle;
        let connection = self.connection_id;
        let fut = unsafe {
            submit(&self.server.queue, (), move |ovl, _| {
                issue(HttpWaitForDisconnect(handle, connection, Some(ovl)))
            })
        };
        async move { fut.await.0.map(|_| ()) }
    }

    async fn send_part<S: BodySource>(&self, part: S, last: bool) -> (Result<()>, S) {
        let handle = self.server.queue.handle;
        let id = self.id;
        let flags = if last { 0 } else { HTTP_SEND_RESPONSE_FLAG_MORE_DATA };
        let payload = Chunked {
            source: part,
            chunk: HTTP_DATA_CHUNK::default(),
        };
        let fut = unsafe {
            submit(&self.server.queue, payload, move |ovl, p| {
                let chunk = p.source.chunk();
                let chunks = chunk.map(|c| {
                    p.chunk = c;
                    core::slice::from_ref(&p.chunk)
                });
                issue(HttpSendResponseEntityBody(
                    handle, id, flags, chunks, None, None, None, Some(ovl), None,
                ))
            })
        };
        let (result, p) = fut.await;
        (result.map(|_| ()), p.source)
    }

    async fn send_head<S: BodySource>(
        &self,
        response: Response,
        body: Option<S>,
        more: bool,
    ) -> (Result<()>, Option<S>) {
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
                    p.policy.as_ref().map(|c| c as *const HTTP_CACHE_POLICY),
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

/// `response` with a `Content-Length` of `len` unless it has one, or is a
/// status that carries no body (1xx, 204, 304) and so no length either.
pub(super) fn with_length(mut response: Response, len: u64) -> Response {
    let bodiless = response.status < 200 || response.status == 204 || response.status == 304;
    if !bodiless && !response.headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("content-length")) {
        response.headers.push(("Content-Length".into(), len.to_string()));
    }
    response
}

pub(super) fn file_size(file: &impl AsRawHandle) -> Result<u64> {
    let mut size = 0i64;
    unsafe { GetFileSizeEx(HANDLE(file.as_raw_handle()), &mut size)? };
    Ok(size as u64)
}

/// Where a response body comes from, as one http.sys data chunk.
pub(super) trait BodySource: Send + 'static {
    /// The chunk, pointing into `self`; `None` for nothing to send, since
    /// http.sys refuses a zero-length memory chunk (measured: `E_INVALIDARG`).
    fn chunk(&self) -> Option<HTTP_DATA_CHUNK>;
}

pub(super) struct Memory<B>(B);

impl<B: IoBuf + 'static> BodySource for Memory<B> {
    fn chunk(&self) -> Option<HTTP_DATA_CHUNK> {
        let len = IoBuf::len(&self.0);
        (len != 0).then(|| memory_chunk(self.0.stable_ptr(), len))
    }
}

pub(super) struct FileRange<F> {
    file: F,
    start: u64,
    /// Or `HTTP_BYTE_RANGE_TO_EOF`.
    len: u64,
}

impl<F: AsRawHandle + Send + 'static> BodySource for FileRange<F> {
    fn chunk(&self) -> Option<HTTP_DATA_CHUNK> {
        Some(HTTP_DATA_CHUNK {
            DataChunkType: HttpDataChunkFromFileHandle,
            Anonymous: HTTP_DATA_CHUNK_0 {
                FromFileHandle: HTTP_DATA_CHUNK_0_1 {
                    ByteRange: HTTP_BYTE_RANGE {
                        StartingOffset: self.start,
                        Length: self.len,
                    },
                    FileHandle: HANDLE(self.file.as_raw_handle()),
                },
            },
        })
    }
}

/// A fragment in http.sys's cache, by name.
pub(super) struct Fragment(Vec<u16>);

impl BodySource for Fragment {
    fn chunk(&self) -> Option<HTTP_DATA_CHUNK> {
        Some(HTTP_DATA_CHUNK {
            DataChunkType: HttpDataChunkFromFragmentCache,
            Anonymous: HTTP_DATA_CHUNK_0 {
                FromFragmentCache: HTTP_DATA_CHUNK_0_2 {
                    FragmentNameLength: (self.0.len() * 2) as u16,
                    pFragmentName: windows_core::PCWSTR(self.0.as_ptr()),
                },
            },
        })
    }
}

/// A body part in flight, with the chunk record that points into it.
pub(super) struct Chunked<S> {
    source: S,
    chunk: HTTP_DATA_CHUNK,
}

// The chunk's pointers point into `source`, which is `Send`; nothing else
// shares them.
unsafe impl<S: Send> Send for Chunked<S> {}

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
pub(super) fn cancel_request(queue: HANDLE, id: u64) {
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

pub(super) fn memory_chunk(ptr: *const u8, len: usize) -> HTTP_DATA_CHUNK {
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
