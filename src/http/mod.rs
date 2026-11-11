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
    HTTP_REQUEST_FLAG_IP_ROUTED, HTTP_REQUEST_INFO, HTTP_REQUEST_TIMING_INFO, HTTP_REQUEST_V1, HTTP_REQUEST_V2,
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
    /// again. `ttl` is `None` to keep it until flushed -- but a cache keeps
    /// nothing for certain: measured on one machine, a fragment was gone
    /// after the first response that used it, and the next send failed
    /// with `ERROR_PATH_NOT_FOUND`. A caller adds it again on that.
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
            if more {
                // Too small: the byte count is the size needed, and the
                // partial record already names the request.
                id = unsafe { &*(buf.as_ptr() as *const HTTP_REQUEST_V2) }.Base.RequestId;
                size = bytes.max(size * 2);
                continue;
            }
            return Ok(unsafe { HttpRequest::parse(&self.inner, buf) });
        }
    }
}

mod request;
mod response;
pub mod ssl;
mod types;
#[cfg(test)]
mod tests;

use request::*;
use response::*;
pub use crate::common::buffer::BufResult;
pub use request::HttpRequest;
pub use response::{CachePolicy, Response};
pub use types::{AuthInfo, AuthScheme, AuthSchemes, AuthStatus, SslProtocol, Timeouts, Verbosity503};
