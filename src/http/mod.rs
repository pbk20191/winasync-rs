//! An HTTP server on http.sys, the kernel's HTTP stack.
//!
//! [`HttpServer::bind`] creates a server session, a URL group and a request
//! queue and binds them; every request, body read and response is then one
//! overlapped call on the queue's handle. http.sys does the parsing, the
//! connection management and the port sharing -- several processes can serve
//! different paths under the same port -- and this module turns its records
//! into owned Rust values.
//!
//! # Where completions run
//!
//! The queue's handle is registered with one of three, chosen at bind time
//! (a handle is bound to one completion port for good):
//!
//! * [`bind`](HttpServer::bind): the process's default thread pool. Needs
//!   nothing else; the futures can be awaited by any executor.
//! * `bind_on`, with `tp`: a `tp::Runtime`'s pool, which then stays alive
//!   while the server does.
//! * `bind_to_port`, with `iocp`: an `iocp::CompletionPort` the caller
//!   drains. Every completion dequeued from it must go through
//!   `CompletionPort::dispatch`, which finishes the operation and wakes its
//!   future; one that never does
//!   leaves its future pending, and the server's memory with it.
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

use crate::common::buffer::{mark_read, IoBuf, IoBufMut};
use crate::common::opfuture::submit;
use crate::common::overlapped::{Binding, IoHandle, IoReg, Issue};
use std::net::SocketAddr;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_HANDLE_EOF, ERROR_IO_PENDING, ERROR_MORE_DATA, HANDLE, NO_ERROR, WIN32_ERROR,
};
use windows::Win32::Networking::HttpServer::{
    HTTP_INITIALIZE,
    Http503ResponseVerbosityBasic, Http503ResponseVerbosityFull, Http503ResponseVerbosityLimited,
    HttpAddUrlToUrlGroup, HttpCancelHttpRequest, HttpCloseServerSession, HttpCloseUrlGroup,
    HttpCreateRequestQueue, HttpCreateServerSession, HttpCreateUrlGroup, HttpDataChunkFromFileHandle,
    HttpDataChunkFromMemory, HttpEnabledStateActive, HttpEnabledStateInactive, HttpInitialize,
    HttpReceiveHttpRequest, HttpReceiveRequestEntityBody, HttpRemoveUrlFromUrlGroup, HttpSendHttpResponse,
    HttpSendResponseEntityBody, HttpServer503VerbosityProperty, HttpServerBindingProperty,
    HttpServerQueueLengthProperty, HttpServerStateProperty, HttpServerTimeoutsProperty,
    HttpSetRequestQueueProperty, HttpSetServerSessionProperty, HttpSetUrlGroupProperty,
    HttpShutdownRequestQueue, HttpTerminate, HttpWaitForDisconnect, HTTPAPI_VERSION, HTTP_BINDING_INFO,
    HTTP_BYTE_RANGE, HTTP_DATA_CHUNK, HTTP_DATA_CHUNK_0, HTTP_DATA_CHUNK_0_0,
    HTTP_DATA_CHUNK_0_1, HTTP_INITIALIZE_SERVER, HTTP_KNOWN_HEADER, HTTP_PROPERTY_FLAGS,
    HTTP_RECEIVE_HTTP_REQUEST_FLAGS, HTTP_REQUEST_FLAG_MORE_ENTITY_BODY_EXISTS,
    HTTP_RESPONSE_V2, HTTP_SEND_RESPONSE_FLAG_MORE_DATA, HTTP_SERVER_PROPERTY, HTTP_STATE_INFO,
    HTTP_TIMEOUT_LIMIT_INFO, HTTP_UNKNOWN_HEADER, HTTP_VERSION,
    HttpAuthStatusFailure, HttpAuthStatusNotAuthenticated, HttpAuthStatusSuccess,
    HttpRequestAuthTypeBasic, HttpRequestAuthTypeDigest, HttpRequestAuthTypeKerberos,
    HttpRequestAuthTypeNTLM, HttpRequestAuthTypeNegotiate, HttpRequestInfoTypeAuth,
    HttpRequestInfoTypeRequestTiming, HttpRequestInfoTypeSslProtocol, HttpServerAuthenticationProperty,
    HTTP_AUTH_ENABLE_BASIC, HTTP_AUTH_ENABLE_DIGEST, HTTP_AUTH_ENABLE_KERBEROS, HTTP_AUTH_ENABLE_NEGOTIATE,
    HTTP_AUTH_ENABLE_NTLM, HTTP_REQUEST_AUTH_INFO, HTTP_REQUEST_FLAG_HTTP2, HTTP_REQUEST_FLAG_HTTP3,
    HTTP_REQUEST_FLAG_IP_ROUTED, HTTP_REQUEST_INFO, HTTP_REQUEST_TIMING_INFO, HTTP_REQUEST_V2,
    HTTP_SERVER_AUTHENTICATION_BASIC_PARAMS, HTTP_SERVER_AUTHENTICATION_DIGEST_PARAMS,
    HTTP_SERVER_AUTHENTICATION_INFO, HTTP_SSL_PROTOCOL_INFO,
    HttpAddFragmentToCache, HttpCachePolicyTimeToLive, HttpCachePolicyUserInvalidates,
    HttpDataChunkFromFragmentCache, HttpFlushResponseCache, HTTP_CACHE_POLICY,
    HTTP_CREATE_REQUEST_QUEUE_FLAG_OPEN_EXISTING, HTTP_DATA_CHUNK_0_2, HTTP_FLUSH_RESPONSE_FLAG_RECURSIVE,
};
use windows::Win32::Storage::FileSystem::GetFileSizeEx;
use windows::Win32::System::IO::OVERLAPPED;
use windows_core::{Error, HSTRING, Result};

const API: HTTPAPI_VERSION = HTTPAPI_VERSION {
    HttpApiMajorVersion: 2,
    HttpApiMinorVersion: 0,
};

/// `HTTP_BYTE_RANGE_TO_EOF` from http.h; not in the `windows` crate.
const HTTP_BYTE_RANGE_TO_EOF: u64 = u64::MAX;

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

/// One `HttpInitialize`, undone by its `HttpTerminate` on drop. The API
/// counts them -- each initialize needs its own terminate, with the same
/// flags -- so every server holds one, and the last server gone is the last
/// terminate.
struct ApiInit(HTTP_INITIALIZE);

impl ApiInit {
    fn new(what: HTTP_INITIALIZE) -> Result<ApiInit> {
        check(unsafe { HttpInitialize(API, what, None) })?;
        Ok(ApiInit(what))
    }
}

impl Drop for ApiInit {
    fn drop(&mut self) {
        unsafe {
            let _ = HttpTerminate(self.0, None);
        }
    }
}

/// The session and URL group a server owns; closed after the queue, and
/// before the API is released.
struct Session {
    session: u64,
    group: u64,
    // A field, so it drops after `drop` below has closed the two IDs.
    _api: ApiInit,
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
        }
    }
}

struct Server {
    // Field order is drop order: the queue closes before its URL group and
    // session, and they before the API is released.
    queue: Arc<IoReg>,
    session: Session,
}

/// An http.sys server: a request queue bound to some URLs.
#[derive(Clone)]
pub struct HttpServer {
    inner: Arc<Server>,
}

impl HttpServer {
    /// Serves `urls` -- fully qualified prefixes such as
    /// `http://localhost:8080/app/` -- from a new request queue whose
    /// completions run on the process's default thread pool. See the module
    /// docs for which prefixes need a reservation.
    pub fn bind(urls: &[&str]) -> Result<HttpServer> {
        Self::bind_with(urls, Binding::process_default(), None)
    }

    /// [`bind`](Self::bind), with the request queue given `name`, so that
    /// other processes can [`open`](Self::open) it and take requests from
    /// it too -- the way IIS shares a queue between worker processes. The
    /// name is global to the machine and fails with `ERROR_ALREADY_EXISTS`
    /// if taken.
    pub fn bind_named(name: &str, urls: &[&str]) -> Result<HttpServer> {
        Self::bind_with(urls, Binding::process_default(), Some(name))
    }

    /// Opens the request queue another server created with
    /// [`bind_named`](Self::bind_named): requests to that server's URLs are
    /// then taken by whichever `accept` -- here or there -- asks first.
    /// This handle serves no URLs of its own.
    pub fn open(name: &str) -> Result<HttpServer> {
        Self::open_with(name, Binding::process_default())
    }

    /// [`open`](Self::open), with completions on `rt`'s pool.
    #[cfg(feature = "tp")]
    pub fn open_on(rt: &crate::tp::Runtime, name: &str) -> Result<HttpServer> {
        Self::open_with(name, Binding::Pool(rt.inner.pool_ref()))
    }

    /// [`open`](Self::open), with completions queued on `port`; see
    /// [`bind_to_port`](Self::bind_to_port).
    #[cfg(feature = "iocp")]
    pub fn open_to_port(port: &crate::iocp::CompletionPort, name: &str) -> Result<HttpServer> {
        Self::open_with(name, Binding::Port(port.port_ref()))
    }

    fn open_with(name: &str, binding: Binding) -> Result<HttpServer> {
        let session = Session {
            session: 0,
            group: 0,
            _api: ApiInit::new(HTTP_INITIALIZE_SERVER)?,
        };
        let mut queue = HANDLE::default();
        check(unsafe {
            HttpCreateRequestQueue(
                API,
                &HSTRING::from(name),
                None,
                Some(HTTP_CREATE_REQUEST_QUEUE_FLAG_OPEN_EXISTING),
                &mut queue,
            )
        })?;
        let queue = IoReg::register(unsafe { IoHandle::from_handle(queue) }, binding)?;
        Ok(HttpServer {
            inner: Arc::new(Server { queue, session }),
        })
    }

    /// [`bind`](Self::bind), with completions on `rt`'s pool.
    #[cfg(feature = "tp")]
    pub fn bind_on(rt: &crate::tp::Runtime, urls: &[&str]) -> Result<HttpServer> {
        Self::bind_with(urls, Binding::Pool(rt.inner.pool_ref()), None)
    }

    /// [`bind`](Self::bind), with completions queued on `port`, which the
    /// caller drains and hands to
    /// [`CompletionPort::dispatch`](crate::iocp::CompletionPort::dispatch).
    /// The server keeps the port open.
    #[cfg(feature = "iocp")]
    pub fn bind_to_port(port: &crate::iocp::CompletionPort, urls: &[&str]) -> Result<HttpServer> {
        Self::bind_with(urls, Binding::Port(port.port_ref()), None)
    }

    fn bind_with(urls: &[&str], binding: Binding, name: Option<&str>) -> Result<HttpServer> {
        // From here on `Session::drop` undoes whatever succeeded.
        let mut session = Session {
            session: 0,
            group: 0,
            _api: ApiInit::new(HTTP_INITIALIZE_SERVER)?,
        };
        check(unsafe { HttpCreateServerSession(API, &mut session.session, None) })?;
        check(unsafe { HttpCreateUrlGroup(session.session, &mut session.group, None) })?;
        for url in urls {
            check(unsafe { HttpAddUrlToUrlGroup(session.group, &HSTRING::from(*url), 0, None) })?;
        }
        let mut queue = HANDLE::default();
        let name = name.map(HSTRING::from);
        check(unsafe {
            HttpCreateRequestQueue(
                API,
                name.as_ref().map_or(windows_core::PCWSTR::null(), |n| windows_core::PCWSTR(n.as_ptr())),
                None,
                None,
                &mut queue,
            )
        })?;
        let queue = IoReg::register(unsafe { IoHandle::from_handle(queue) }, binding)?;
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
            inner: Arc::new(Server { queue, session }),
        })
    }

    /// Serves one more URL prefix.
    pub fn add_url(&self, url: &str) -> Result<()> {
        check(unsafe { HttpAddUrlToUrlGroup(self.inner.session.group, &HSTRING::from(url), 0, None) })
    }

    /// Stops serving a prefix given to [`bind`](Self::bind) or
    /// [`add_url`](Self::add_url).
    pub fn remove_url(&self, url: &str) -> Result<()> {
        check(unsafe { HttpRemoveUrlFromUrlGroup(self.inner.session.group, &HSTRING::from(url), 0) })
    }

    /// Stops taking requests: a waiting [`accept`](Self::accept) fails with
    /// `ERROR_OPERATION_ABORTED`, and so does every later one. Requests
    /// already accepted can still be answered. Dropping the last handle does
    /// the same and more; this is for stopping first and dropping later.
    pub fn shutdown(&self) -> Result<()> {
        check(unsafe { HttpShutdownRequestQueue(self.inner.queue.handle) })
    }

    /// Whether http.sys routes requests here. Turned off, the prefixes stay
    /// registered -- no other process can take them -- while http.sys
    /// answers their requests itself (measured: 503).
    pub fn set_enabled(&self, enabled: bool) -> Result<()> {
        let state = HTTP_STATE_INFO {
            Flags: HTTP_PROPERTY_FLAGS { _bitfield: 1 },
            State: if enabled { HttpEnabledStateActive } else { HttpEnabledStateInactive },
        };
        check(unsafe {
            HttpSetUrlGroupProperty(
                self.inner.session.group,
                HttpServerStateProperty,
                (&state as *const HTTP_STATE_INFO).cast(),
                size_of::<HTTP_STATE_INFO>() as u32,
            )
        })
    }

    /// Sets http.sys's connection timeouts for this server. Set on the
    /// server's own session, which is the only place the header-wait and
    /// send-rate limits may be set.
    ///
    /// Measured: they apply to a connection once it has carried a request
    /// here -- one that has sent nothing yet belongs to no server, and keeps
    /// the machine-wide defaults -- and they are checked coarsely: a
    /// one-second idle timeout closed a kept-alive connection after five
    /// seconds on one machine and ten on another.
    pub fn set_timeouts(&self, timeouts: &Timeouts) -> Result<()> {
        let secs = |d: Option<Duration>| d.map_or(0, |d| d.as_secs().clamp(1, u16::MAX as u64) as u16);
        let info = HTTP_TIMEOUT_LIMIT_INFO {
            Flags: HTTP_PROPERTY_FLAGS { _bitfield: 1 },
            EntityBody: secs(timeouts.entity_body),
            DrainEntityBody: secs(timeouts.drain_entity_body),
            RequestQueue: secs(timeouts.request_queue),
            IdleConnection: secs(timeouts.idle_connection),
            HeaderWait: secs(timeouts.header_wait),
            MinSendRate: timeouts.min_send_rate.unwrap_or(0),
        };
        check(unsafe {
            HttpSetServerSessionProperty(
                self.inner.session.session,
                HttpServerTimeoutsProperty,
                (&info as *const HTTP_TIMEOUT_LIMIT_INFO).cast(),
                size_of::<HTTP_TIMEOUT_LIMIT_INFO>() as u32,
            )
        })
    }

    /// Has http.sys authenticate requests before they are delivered, with
    /// the Windows schemes in `schemes`. The request then carries the
    /// outcome in [`HttpRequest::auth`], whose docs say what arrives when;
    /// in short, answer a request that is not authenticated with
    /// [`Response::challenge`], and http.sys does the rest.
    ///
    /// `realm` is what Basic and Digest challenges name; `domain` is for
    /// Digest. Setting no scheme turns authentication off.
    pub fn set_authentication(&self, schemes: AuthSchemes, realm: &str, domain: &str) -> Result<()> {
        let mut realm: Vec<u16> = realm.encode_utf16().chain([0]).collect();
        let mut domain: Vec<u16> = domain.encode_utf16().chain([0]).collect();
        // A string is given only to the scheme it is for: measured, a
        // Digest parameter block filled while Digest is off is
        // `E_INVALIDARG`.
        let wide = |v: &mut Vec<u16>| {
            let len = (v.len() - 1) * 2;
            (len as u16, if len == 0 { windows_core::PWSTR::null() } else { windows_core::PWSTR(v.as_mut_ptr()) })
        };
        let mut info = HTTP_SERVER_AUTHENTICATION_INFO {
            Flags: HTTP_PROPERTY_FLAGS { _bitfield: 1 },
            AuthSchemes: schemes.0,
            // Off, http.sys appends Kerberos mutual-authentication data to
            // the final response itself; on, the application would have to.
            ReceiveMutualAuth: false,
            // The token is what a server wants; the packed context is for
            // handing the authentication to another process.
            ReceiveContextHandle: false,
            DisableNTLMCredentialCaching: false,
            ExFlags: 0,
            DigestParams: HTTP_SERVER_AUTHENTICATION_DIGEST_PARAMS::default(),
            BasicParams: HTTP_SERVER_AUTHENTICATION_BASIC_PARAMS::default(),
        };
        if schemes.0 & HTTP_AUTH_ENABLE_BASIC != 0 {
            let (len, ptr) = wide(&mut realm);
            info.BasicParams = HTTP_SERVER_AUTHENTICATION_BASIC_PARAMS {
                RealmLength: len,
                Realm: ptr,
            };
        }
        if schemes.0 & HTTP_AUTH_ENABLE_DIGEST != 0 {
            let (rlen, rptr) = wide(&mut realm);
            let (dlen, dptr) = wide(&mut domain);
            info.DigestParams = HTTP_SERVER_AUTHENTICATION_DIGEST_PARAMS {
                DomainNameLength: dlen,
                DomainName: dptr,
                RealmLength: rlen,
                Realm: rptr,
            };
        }
        check(unsafe {
            HttpSetUrlGroupProperty(
                self.inner.session.group,
                HttpServerAuthenticationProperty,
                (&info as *const HTTP_SERVER_AUTHENTICATION_INFO).cast(),
                size_of::<HTTP_SERVER_AUTHENTICATION_INFO>() as u32,
            )
        })
    }

    /// Drops what http.sys has cached for `url_prefix` -- responses sent
    /// with [`Response::cache_until_flushed`] or
    /// [`Response::cache_for`], and fragments -- and, with `recursive`,
    /// everything under it.
    pub fn flush_cache(&self, url_prefix: &str, recursive: bool) -> Result<()> {
        let flags = if recursive { HTTP_FLUSH_RESPONSE_FLAG_RECURSIVE } else { 0 };
        check(unsafe { HttpFlushResponseCache(self.inner.queue.handle, &HSTRING::from(url_prefix), flags, None) })
    }

    /// Puts `data` in http.sys's cache under `name` -- a URL under one of
    /// this server's prefixes, which is the name's only role -- to be sent
    /// as a response body by [`HttpRequest::respond_fragment`] and
    /// [`HttpRequest::send_fragment`] without passing through this process
    /// again. `ttl` is `None` to keep it until flushed.
    pub fn add_fragment(&self, name: &str, data: &[u8], ttl: Option<Duration>) -> Result<()> {
        let chunk = memory_chunk(data.as_ptr(), data.len());
        let policy = cache_policy(match ttl {
            Some(ttl) => CachePolicy::TimeToLive(ttl),
            None => CachePolicy::UntilFlushed,
        })
        .unwrap();
        check(unsafe { HttpAddFragmentToCache(self.inner.queue.handle, &HSTRING::from(name), &chunk, &policy, None) })
    }

    /// How many requests may wait in the queue for an `accept`; past it
    /// http.sys answers 503 itself. The default is 1000.
    pub fn set_queue_length(&self, length: u32) -> Result<()> {
        self.set_queue_property(HttpServerQueueLengthProperty, length)
    }

    /// How much http.sys says when it answers 503 for this server.
    pub fn set_503_verbosity(&self, verbosity: Verbosity503) -> Result<()> {
        let value = match verbosity {
            Verbosity503::Basic => Http503ResponseVerbosityBasic,
            Verbosity503::Limited => Http503ResponseVerbosityLimited,
            Verbosity503::Full => Http503ResponseVerbosityFull,
        };
        self.set_queue_property(HttpServer503VerbosityProperty, value.0 as u32)
    }

    fn set_queue_property(&self, property: HTTP_SERVER_PROPERTY, value: u32) -> Result<()> {
        check(unsafe {
            HttpSetRequestQueueProperty(
                self.inner.queue.handle,
                property,
                (&value as *const u32).cast(),
                4,
                None,
                None,
            )
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
            let record = unsafe { &*(buf.as_ptr() as *const HTTP_REQUEST_V2) };
            if more {
                // Too small: the byte count is the size needed, and the
                // partial record already names the request.
                id = record.Base.RequestId;
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

/// # Safety
/// `info` is the auth record of a request being parsed, whose pointers are
/// valid for the call; the token handle is taken over.
unsafe fn parse_auth(info: &HTTP_REQUEST_AUTH_INFO) -> AuthInfo {
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
    /// `record` is a complete `HTTP_REQUEST_V2` whose pointers point into
    /// the buffer it sits in, which is alive for the call.
    unsafe fn parse(server: &Arc<Server>, record: &HTTP_REQUEST_V2) -> HttpRequest {
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
                http2: record.Flags & HTTP_REQUEST_FLAG_HTTP2 != 0,
                http3: record.Flags & HTTP_REQUEST_FLAG_HTTP3 != 0,
                ip_routed: record.Flags & HTTP_REQUEST_FLAG_IP_ROUTED != 0,
                auth,
                ssl_protocol,
                timing,
                other_info,
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
    ) -> (Result<()>, F) {
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
    pub async fn send_body<B: IoBuf + 'static>(&self, chunk: B, last: bool) -> (Result<()>, B) {
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
    ) -> (Result<()>, F) {
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
fn with_length(mut response: Response, len: u64) -> Response {
    let bodiless = response.status < 200 || response.status == 204 || response.status == 304;
    if !bodiless && !response.headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("content-length")) {
        response.headers.push(("Content-Length".into(), len.to_string()));
    }
    response
}

fn file_size(file: &impl AsRawHandle) -> Result<u64> {
    let mut size = 0i64;
    unsafe { GetFileSizeEx(HANDLE(file.as_raw_handle()), &mut size)? };
    Ok(size as u64)
}

/// Where a response body comes from, as one http.sys data chunk.
trait BodySource: Send + 'static {
    /// The chunk, pointing into `self`; `None` for nothing to send, since
    /// http.sys refuses a zero-length memory chunk (measured: `E_INVALIDARG`).
    fn chunk(&self) -> Option<HTTP_DATA_CHUNK>;
}

struct Memory<B>(B);

impl<B: IoBuf + 'static> BodySource for Memory<B> {
    fn chunk(&self) -> Option<HTTP_DATA_CHUNK> {
        let len = IoBuf::len(&self.0);
        (len != 0).then(|| memory_chunk(self.0.stable_ptr(), len))
    }
}

struct FileRange<F> {
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
struct Fragment(Vec<u16>);

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
struct Chunked<S> {
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

/// http.sys's connection timeouts ([`HttpServer::set_timeouts`]). `None`
/// leaves http.sys's default; durations are whole seconds, at least one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Timeouts {
    /// For the request body to arrive (default 2 minutes).
    pub entity_body: Option<Duration>,
    /// For the rest of a body nobody read, after the response (2 minutes).
    pub drain_entity_body: Option<Duration>,
    /// For a request to sit in the queue before an `accept` takes it (2
    /// minutes).
    pub request_queue: Option<Duration>,
    /// For an idle connection between requests (2 minutes).
    pub idle_connection: Option<Duration>,
    /// For the request's headers to arrive (2 minutes).
    pub header_wait: Option<Duration>,
    /// The slowest a client may take a response, in bytes per second (150).
    pub min_send_rate: Option<u32>,
}

/// Which authentication schemes http.sys offers ([`HttpServer::set_authentication`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthSchemes(u32);

impl AuthSchemes {
    pub const NONE: AuthSchemes = AuthSchemes(0);
    pub const BASIC: AuthSchemes = AuthSchemes(HTTP_AUTH_ENABLE_BASIC);
    pub const DIGEST: AuthSchemes = AuthSchemes(HTTP_AUTH_ENABLE_DIGEST);
    pub const NTLM: AuthSchemes = AuthSchemes(HTTP_AUTH_ENABLE_NTLM);
    pub const NEGOTIATE: AuthSchemes = AuthSchemes(HTTP_AUTH_ENABLE_NEGOTIATE);
    pub const KERBEROS: AuthSchemes = AuthSchemes(HTTP_AUTH_ENABLE_KERBEROS);
}

impl core::ops::BitOr for AuthSchemes {
    type Output = AuthSchemes;
    fn bitor(self, rhs: AuthSchemes) -> AuthSchemes {
        AuthSchemes(self.0 | rhs.0)
    }
}

/// How http.sys's authentication of a request ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthStatus {
    /// The caller is who [`AuthInfo::token`] says.
    Success,
    /// The request carried no credentials.
    NotAuthenticated,
    /// The credentials were refused; [`AuthInfo::sec_status`] says why.
    Failure,
    /// A status this crate does not know.
    Other(i32),
}

/// The scheme a request authenticated with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthScheme {
    None,
    Basic,
    Digest,
    Ntlm,
    Negotiate,
    Kerberos,
    Other(i32),
}

/// What http.sys found out about who sent a request
/// ([`HttpRequest::auth`]).
#[derive(Debug)]
pub struct AuthInfo {
    pub status: AuthStatus,
    /// The security status behind a [`AuthStatus::Failure`].
    pub sec_status: windows_core::HRESULT,
    pub scheme: AuthScheme,
    /// The caller's access token on success: impersonate with it, or read
    /// its user. Closed when dropped.
    pub token: Option<OwnedHandle>,
    /// The security package that authenticated, e.g. `NTLM`.
    pub package: String,
    /// `ISC_RET_*`/`ASC_RET_*` attributes of the security context.
    pub context_attributes: u32,
}

/// The TLS parameters of the connection a request came on
/// ([`HttpRequest::ssl_protocol`]); `SP_PROT_*`, `CALG_*` values from
/// schannel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SslProtocol {
    pub protocol: u32,
    pub cipher: u32,
    pub cipher_strength: u32,
    pub hash: u32,
    pub hash_strength: u32,
    pub key_exchange: u32,
    pub key_exchange_strength: u32,
}

/// What http.sys's own 503 says ([`HttpServer::set_503_verbosity`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verbosity503 {
    /// Closes the connection with no response: the default.
    Basic,
    /// A 503 with a generic reason.
    Limited,
    /// A 503 with the specific reason. Needs an administrator.
    Full,
}

/// A response's status and headers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    /// Whether http.sys keeps the sent response and answers the next
    /// requests for the URL itself; see [`cache_for`](Self::cache_for).
    pub cache: CachePolicy,
}

/// http.sys's response cache for one response ([`Response::cache`]).
///
/// Measured: a cached response is served by http.sys to later `GET`s of
/// the same URL without the request ever reaching `accept`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CachePolicy {
    /// Not cached.
    #[default]
    None,
    /// Cached for this long (whole seconds).
    TimeToLive(Duration),
    /// Cached until [`HttpServer::flush_cache`].
    UntilFlushed,
}

fn cache_policy(policy: CachePolicy) -> Option<HTTP_CACHE_POLICY> {
    match policy {
        CachePolicy::None => None,
        CachePolicy::TimeToLive(ttl) => Some(HTTP_CACHE_POLICY {
            Policy: HttpCachePolicyTimeToLive,
            SecondsToLive: ttl.as_secs().clamp(1, u32::MAX as u64) as u32,
        }),
        CachePolicy::UntilFlushed => Some(HTTP_CACHE_POLICY {
            Policy: HttpCachePolicyUserInvalidates,
            SecondsToLive: 0,
        }),
    }
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
            cache: CachePolicy::None,
        }
    }

    /// Has http.sys cache the response for `ttl` once sent whole (with
    /// [`respond`](HttpRequest::respond) or
    /// [`respond_file`](HttpRequest::respond_file); a streamed response is
    /// not cacheable). Only a `GET` answer with a 200 is cached, and only
    /// while the response is cacheable by HTTP's rules.
    pub fn cache_for(mut self, ttl: Duration) -> Response {
        self.cache = CachePolicy::TimeToLive(ttl);
        self
    }

    /// [`cache_for`](Self::cache_for), until [`HttpServer::flush_cache`].
    pub fn cache_until_flushed(mut self) -> Response {
        self.cache = CachePolicy::UntilFlushed;
        self
    }

    /// A 401 that names `schemes` in `WWW-Authenticate` headers, one per
    /// scheme -- the initial challenge of [`HttpServer::set_authentication`].
    /// `realm` is for Basic and Digest. Measured: for Digest the header
    /// need only name the realm; http.sys rewrites it on the way out with
    /// the nonce, `qop` and algorithm it will verify against.
    pub fn challenge(schemes: AuthSchemes, realm: &str) -> Response {
        let mut response = Response::new(401);
        if schemes.0 & (HTTP_AUTH_ENABLE_NEGOTIATE | HTTP_AUTH_ENABLE_KERBEROS) != 0 {
            response = response.header("WWW-Authenticate", "Negotiate");
        }
        if schemes.0 & HTTP_AUTH_ENABLE_NTLM != 0 {
            response = response.header("WWW-Authenticate", "NTLM");
        }
        if schemes.0 & HTTP_AUTH_ENABLE_DIGEST != 0 {
            response = response.header("WWW-Authenticate", &format!("Digest realm=\"{realm}\""));
        }
        if schemes.0 & HTTP_AUTH_ENABLE_BASIC != 0 {
            response = response.header("WWW-Authenticate", &format!("Basic realm=\"{realm}\""));
        }
        response
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
struct ResponsePayload<S> {
    response: HTTP_RESPONSE_V2,
    reason: Vec<u8>,
    /// `(known index or None, name, value)`, as bytes.
    headers: Vec<(Option<usize>, Vec<u8>, Vec<u8>)>,
    unknown: Vec<HTTP_UNKNOWN_HEADER>,
    chunk: HTTP_DATA_CHUNK,
    body: Option<S>,
    policy: Option<HTTP_CACHE_POLICY>,
}

// Every pointer in the records points into the payload itself, and the body
// source is `Send`.
unsafe impl<S: Send> Send for ResponsePayload<S> {}

impl<S: BodySource> ResponsePayload<S> {
    fn new(response: Response, body: Option<S>) -> Self {
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
            policy: cache_policy(response.cache),
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
        if let Some(chunk) = self.body.as_ref().and_then(BodySource::chunk) {
            self.chunk = chunk;
            base.EntityChunkCount = 1;
            base.pEntityChunks = &mut self.chunk;
        }
    }
}

/// http.sys's SSL configuration: which certificate answers on a port.
///
/// This is machine-wide state (`netsh http add sslcert` edits the same
/// table) and setting it needs an elevated process; reading it does not.
pub mod ssl {
    use super::*;
    use windows::Win32::Networking::HttpServer::{
        HttpDeleteServiceConfiguration, HttpQueryServiceConfiguration, HttpServiceConfigQueryExact,
        HttpServiceConfigSSLCertInfo, HttpSetServiceConfiguration, HTTP_INITIALIZE_CONFIG,
        HTTP_SERVICE_CONFIG_SSL_KEY, HTTP_SERVICE_CONFIG_SSL_PARAM, HTTP_SERVICE_CONFIG_SSL_QUERY,
        HTTP_SERVICE_CONFIG_SSL_SET,
    };
    use windows::Win32::Networking::WinSock::{SOCKADDR, SOCKADDR_STORAGE};

    /// A certificate bound to an address ([`query_certificate`]).
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct SslBinding {
        /// The certificate's SHA-1 thumbprint.
        pub hash: Vec<u8>,
        /// The application that bound it.
        pub app_id: windows_core::GUID,
        /// The certificate store it is in, e.g. `MY`.
        pub store: String,
        /// `HTTP_SERVICE_CONFIG_SSL_FLAG_*`.
        pub flags: u32,
    }

    fn key(addr: SocketAddr, storage: &mut SOCKADDR_STORAGE) -> HTTP_SERVICE_CONFIG_SSL_KEY {
        crate::common::sockaddr::write_sockaddr(&addr, storage);
        HTTP_SERVICE_CONFIG_SSL_KEY {
            pIpPort: (storage as *mut SOCKADDR_STORAGE).cast::<SOCKADDR>(),
        }
    }

    /// Answers HTTPS on `addr` -- `0.0.0.0:443` for every interface -- with
    /// the certificate whose SHA-1 thumbprint is `hash`, from the local
    /// machine's `store` (`MY` for the personal store). `app_id` names the
    /// application for `netsh`'s benefit. Needs elevation; measured
    /// unelevated: `ERROR_ACCESS_DENIED`.
    pub fn bind_certificate(addr: SocketAddr, hash: &[u8], app_id: windows_core::GUID, store: &str) -> Result<()> {
        let _api = ApiInit::new(HTTP_INITIALIZE_CONFIG)?;
        let mut storage = SOCKADDR_STORAGE::default();
        let mut store: Vec<u16> = store.encode_utf16().chain([0]).collect();
        let set = HTTP_SERVICE_CONFIG_SSL_SET {
            KeyDesc: key(addr, &mut storage),
            ParamDesc: HTTP_SERVICE_CONFIG_SSL_PARAM {
                SslHashLength: hash.len() as u32,
                pSslHash: hash.as_ptr() as *mut core::ffi::c_void,
                AppId: app_id,
                pSslCertStoreName: windows_core::PWSTR(store.as_mut_ptr()),
                ..Default::default()
            },
        };
        check(unsafe {
            HttpSetServiceConfiguration(
                None,
                HttpServiceConfigSSLCertInfo,
                (&set as *const HTTP_SERVICE_CONFIG_SSL_SET).cast(),
                size_of::<HTTP_SERVICE_CONFIG_SSL_SET>() as u32,
                None,
            )
        })
    }

    /// Removes the binding on `addr`. Needs elevation.
    pub fn unbind_certificate(addr: SocketAddr) -> Result<()> {
        let _api = ApiInit::new(HTTP_INITIALIZE_CONFIG)?;
        let mut storage = SOCKADDR_STORAGE::default();
        let set = HTTP_SERVICE_CONFIG_SSL_SET {
            KeyDesc: key(addr, &mut storage),
            ParamDesc: Default::default(),
        };
        check(unsafe {
            HttpDeleteServiceConfiguration(
                None,
                HttpServiceConfigSSLCertInfo,
                (&set as *const HTTP_SERVICE_CONFIG_SSL_SET).cast(),
                size_of::<HTTP_SERVICE_CONFIG_SSL_SET>() as u32,
                None,
            )
        })
    }

    /// The certificate bound to exactly `addr`, if any.
    pub fn query_certificate(addr: SocketAddr) -> Result<Option<SslBinding>> {
        let _api = ApiInit::new(HTTP_INITIALIZE_CONFIG)?;
        let mut storage = SOCKADDR_STORAGE::default();
        let query = HTTP_SERVICE_CONFIG_SSL_QUERY {
            QueryDesc: HttpServiceConfigQueryExact,
            KeyDesc: key(addr, &mut storage),
            dwToken: 0,
        };
        let mut needed = 0u32;
        let mut buf: Vec<u64> = Vec::new();
        loop {
            let code = unsafe {
                HttpQueryServiceConfiguration(
                    None,
                    HttpServiceConfigSSLCertInfo,
                    Some((&query as *const HTTP_SERVICE_CONFIG_SSL_QUERY).cast()),
                    size_of::<HTTP_SERVICE_CONFIG_SSL_QUERY>() as u32,
                    Some(buf.as_mut_ptr().cast()),
                    (buf.len() * 8) as u32,
                    Some(&mut needed),
                    None,
                )
            };
            match code {
                0 => break,
                c if c == windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER.0 => {
                    buf = vec![0u64; (needed as usize).div_ceil(8)];
                }
                c if c == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND.0 => return Ok(None),
                c => return Err(Error::from_hresult(WIN32_ERROR(c).to_hresult())),
            }
        }
        let set = unsafe { &*(buf.as_ptr() as *const HTTP_SERVICE_CONFIG_SSL_SET) };
        let p = &set.ParamDesc;
        Ok(Some(unsafe {
            SslBinding {
                hash: if p.pSslHash.is_null() {
                    Vec::new()
                } else {
                    core::slice::from_raw_parts(p.pSslHash.cast::<u8>(), p.SslHashLength as usize).to_vec()
                },
                app_id: p.AppId,
                store: if p.pSslCertStoreName.is_null() {
                    String::new()
                } else {
                    p.pSslCertStoreName.to_string().unwrap_or_default()
                },
                flags: p.DefaultFlags,
            }
        }))
    }
}

#[cfg(test)]
mod tests {
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
        block_on(req.start_response(Response::new(200).header("Content-Length", &len.to_string()))).unwrap();
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
        block_on(req.send_fragment(&name, true)).unwrap();
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
}
