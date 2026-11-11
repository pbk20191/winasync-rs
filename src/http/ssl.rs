//! http.sys's SSL configuration: which certificate answers on a port.
//!
//! This is machine-wide state (`netsh http add sslcert` edits the same
//! table) and setting it needs an elevated process; reading it does not.
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
