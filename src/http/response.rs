//! A response as the caller describes it, and as http.sys wants it.

use super::*;

/// And for responses: the same first 20, then the response ones.
pub(super) const RESPONSE_HEADERS: [&str; 30] = [
    "Cache-Control", "Connection", "Date", "Keep-Alive", "Pragma", "Trailer",
    "Transfer-Encoding", "Upgrade", "Via", "Warning", "Allow", "Content-Length",
    "Content-Type", "Content-Encoding", "Content-Language", "Content-Location",
    "Content-MD5", "Content-Range", "Expires", "Last-Modified", "Accept-Ranges", "Age",
    "ETag", "Location", "Proxy-Authenticate", "Retry-After", "Server", "Set-Cookie",
    "Vary", "WWW-Authenticate",
];

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

pub(super) fn cache_policy(policy: CachePolicy) -> Option<HTTP_CACHE_POLICY> {
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
            response = response.header("WWW-Authenticate", format!("Digest realm=\"{realm}\""));
        }
        if schemes.0 & HTTP_AUTH_ENABLE_BASIC != 0 {
            response = response.header("WWW-Authenticate", format!("Basic realm=\"{realm}\""));
        }
        response
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Response {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn reason(mut self, reason: &str) -> Response {
        self.reason = reason.to_owned();
        self
    }
}

/// A response as http.sys wants it, owning everything its pointers name.
pub(super) struct ResponsePayload<S> {
    pub(super) response: HTTP_RESPONSE_V2,
    reason: Vec<u8>,
    /// `(known index or None, name, value)`, as bytes.
    headers: Vec<(Option<usize>, Vec<u8>, Vec<u8>)>,
    unknown: Vec<HTTP_UNKNOWN_HEADER>,
    chunk: HTTP_DATA_CHUNK,
    pub(super) body: Option<S>,
    pub(super) policy: Option<HTTP_CACHE_POLICY>,
}

// Every pointer in the records points into the payload itself, and the body
// source is `Send`.
unsafe impl<S: Send> Send for ResponsePayload<S> {}

impl<S: BodySource> ResponsePayload<S> {
    pub(super) fn new(response: Response, body: Option<S>) -> Self {
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
    pub(super) fn wire(&mut self) {
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
