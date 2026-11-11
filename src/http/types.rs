//! The plain values the server and its requests are configured and
//! described with.

use super::*;

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
pub struct AuthSchemes(pub(super) u32);

impl AuthSchemes {
    /// From the `HTTP_AUTH_ENABLE_*` bits.
    pub const fn from_bits(bits: u32) -> AuthSchemes {
        AuthSchemes(bits)
    }
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
