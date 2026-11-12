//! The http.sys server as a WinRT class: `Pbk20191.WinAsync.Comio.HttpServer`
//! and the `HttpRequest` it hands out. Thin over [`crate::http`]; the
//! futures run through [`super::future_op`].

#![allow(non_snake_case)]

use super::bindings::Pbk20191::WinAsync::Comio as rc;
use super::bindings::Pbk20191::WinAsync::Comio::{IHttpRequest_Impl, IHttpServer_Impl};
use super::future_op::{spawn_action, spawn_op};
use super::unload::ModuleRef;
use crate::common::buffer::RawBuf;
use crate::http as h;
use std::sync::Mutex;
use windows::Foundation::IClosable_Impl;
use windows_future::{IAsyncAction, IAsyncOperation};
use windows::Storage::Streams::IBuffer;
use super::asyncop::Keep;
use windows_core::{ComObject, Error, Ref, Result, HRESULT, HSTRING};
use winasync_implement::implement;

const RO_E_CLOSED: HRESULT = HRESULT(0x80000013u32 as i32);
const E_ILLEGAL_METHOD_CALL: HRESULT = HRESULT(0x8000000Eu32 as i32);

#[implement(
    rc::HttpServer,
    windows::Foundation::IClosable,
    Export = Pbk20191_WinAsync_Comio_HttpServer,
    ExportCfg = "feature = \"static-exports\""
)]
pub struct HttpServer {
    inner: Mutex<Option<h::HttpServer>>,
    urls: Mutex<Vec<String>>,
    _module: ModuleRef,
}

impl HttpServer {
    pub fn new() -> ComObject<HttpServer> {
        ComObject::new(HttpServer {
            inner: Mutex::new(None),
            urls: Mutex::new(Vec::new()),
            _module: ModuleRef::new(),
        })
    }
}

impl Default for HttpServer {
    fn default() -> Self {
        HttpServer {
            inner: Mutex::new(None),
            urls: Mutex::new(Vec::new()),
            _module: ModuleRef::new(),
        }
    }
}

impl HttpServer_Impl {
    fn server(&self) -> Result<h::HttpServer> {
        self.inner.lock().unwrap().clone().ok_or_else(|| Error::from_hresult(E_ILLEGAL_METHOD_CALL))
    }
}

impl IHttpServer_Impl for HttpServer_Impl {
    fn BindAsync(&self, url: &HSTRING) -> Result<IAsyncAction> {
        let url = url.to_string();
        let mut g = self.inner.lock().unwrap();
        match g.as_ref() {
            Some(server) => server.add_url(&url)?,
            None => *g = Some(h::HttpServer::bind(&[&url])?),
        }
        self.urls.lock().unwrap().push(url);
        Ok(super::settled::action_done())
    }

    fn AcceptAsync(&self) -> Result<IAsyncOperation<rc::HttpRequest>> {
        let server = self.server()?;
        Ok(spawn_op(async move {
            let req = server.accept().await?;
            Ok(HttpRequest::wrap(req).into_interface())
        }))
    }

    fn SetAuthentication(&self, schemes: u32, realm: &HSTRING) -> Result<()> {
        self.server()?.set_authentication(h::AuthSchemes::from_bits(schemes), &realm.to_string(), "")
    }

    fn Shutdown(&self) -> Result<()> {
        self.server()?.shutdown()
    }

    fn Information(&self) -> Result<HSTRING> {
        Ok(HSTRING::from(self.urls.lock().unwrap().join(" ")))
    }
}

impl IClosable_Impl for HttpServer_Impl {
    fn Close(&self) -> Result<()> {
        self.inner.lock().unwrap().take();
        Ok(())
    }
}

#[implement(
    rc::HttpRequest,
    windows::Foundation::IClosable,
    Export = Pbk20191_WinAsync_Comio_HttpRequest,
    ExportCfg = "feature = \"static-exports\""
)]
pub struct HttpRequest {
    inner: Mutex<Option<h::HttpRequest>>,
    _module: ModuleRef,
}

impl HttpRequest {
    fn wrap(req: h::HttpRequest) -> ComObject<HttpRequest> {
        ComObject::new(HttpRequest {
            inner: Mutex::new(Some(req)),
            _module: ModuleRef::new(),
        })
    }
}

impl HttpRequest_Impl {
    fn with<R>(&self, f: impl FnOnce(&h::HttpRequest) -> R) -> Result<R> {
        match self.inner.lock().unwrap().as_ref() {
            Some(req) => Ok(f(req)),
            None => Err(Error::from_hresult(RO_E_CLOSED)),
        }
    }
    fn text(&self, f: impl FnOnce(&h::HttpRequest) -> String) -> Result<HSTRING> {
        self.with(f).map(HSTRING::from)
    }
}

/// A WinRT buffer as an owned buffer for the core: the bytes stay where
/// `IBufferByteAccess` says while the kept buffer lives.
fn raw_buf(buffer: &IBuffer, len: usize) -> Result<RawBuf> {
    let ptr = super::buffer::buffer_ptr(buffer)?;
    let total = buffer.Capacity()? as usize;
    let keep = Keep::new(buffer)?;
    // SAFETY: the buffer's bytes live, at that address, while `keep` does.
    Ok(unsafe { RawBuf::from_raw_parts(ptr, len.min(total), total, keep) })
}

impl IHttpRequest_Impl for HttpRequest_Impl {
    fn Method(&self) -> Result<HSTRING> {
        self.text(|r| r.method().to_owned())
    }
    fn RawUrl(&self) -> Result<HSTRING> {
        self.text(|r| r.raw_url().to_owned())
    }
    fn Url(&self) -> Result<HSTRING> {
        self.text(|r| r.url.clone())
    }
    fn Path(&self) -> Result<HSTRING> {
        self.text(|r| r.path.clone())
    }
    fn Query(&self) -> Result<HSTRING> {
        self.text(|r| r.query.clone())
    }
    fn Headers(&self) -> Result<HSTRING> {
        self.text(|r| {
            let mut out = String::new();
            for (name, value) in r.headers() {
                out.push_str(name);
                out.push_str(": ");
                out.push_str(&String::from_utf8_lossy(value));
                out.push_str("\r\n");
            }
            out
        })
    }
    fn RemoteAddress(&self) -> Result<HSTRING> {
        self.text(|r| r.remote_addr.map(|a| a.to_string()).unwrap_or_default())
    }
    fn LocalAddress(&self) -> Result<HSTRING> {
        self.text(|r| r.local_addr.map(|a| a.to_string()).unwrap_or_default())
    }
    fn HasBody(&self) -> Result<bool> {
        self.with(|r| r.has_body())
    }
    fn AuthStatus(&self) -> Result<u32> {
        self.with(|r| match r.auth.as_ref().map(|a| a.status) {
            None => 0,
            Some(h::AuthStatus::Success) => 1,
            Some(h::AuthStatus::NotAuthenticated) => 2,
            Some(h::AuthStatus::Failure) | Some(h::AuthStatus::Other(_)) => 3,
        })
    }
    fn AuthPackage(&self) -> Result<HSTRING> {
        self.text(|r| r.auth.as_ref().map(|a| a.package.clone()).unwrap_or_default())
    }
    fn Header(&self, name: &HSTRING) -> Result<HSTRING> {
        let name = name.to_string();
        self.text(|r| r.header(&name).unwrap_or("").to_owned())
    }

    fn ReadBodyAsync(&self, buffer: Ref<'_, IBuffer>) -> Result<IAsyncOperation<u32>> {
        let buffer = buffer.ok()?.clone();
        let buf = raw_buf(&buffer, 0)?;
        let done = Keep::new(&buffer)?;
        let fut = self.with(|r| r.read_body(buf))?;
        Ok(spawn_op(async move {
            let (n, _buf) = fut.await;
            let n = n?;
            done.resolve()?.SetLength(n as u32)?;
            Ok(n as u32)
        }))
    }

    fn RespondAsync(&self, status: u32, headers: &HSTRING, body: Ref<'_, IBuffer>) -> Result<IAsyncAction> {
        let body = body.ok()?.clone();
        let buf = raw_buf(&body, body.Length()? as usize)?;
        let mut response = h::Response::new(status as u16);
        for line in headers.to_string().lines() {
            if let Some((name, value)) = line.split_once(':') {
                response = response.header(name.trim(), value.trim());
            }
        }
        // The request goes with the response: nothing may be read after.
        let req = self
            .inner
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| Error::from_hresult(RO_E_CLOSED))?;
        Ok(spawn_action(async move { req.respond(response, buf).await.0 }))
    }
}

impl IClosable_Impl for HttpRequest_Impl {
    fn Close(&self) -> Result<()> {
        // Dropped unanswered, the request is cancelled.
        self.inner.lock().unwrap().take();
        Ok(())
    }
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
