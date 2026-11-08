//! Async named-pipe IO over TP_IO with a raw `(Result, Vec<u8>)` API (the
//! `tp`-native counterpart to comio's WinRT-shaped named pipe).

use super::{submit, IoRes, Issue};
use crate::tp::Runtime;
use std::sync::Arc;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, WaitNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
    PIPE_WAIT,
};
use windows_core::{Error, HSTRING};

/// A connected named-pipe endpoint (server- or client-side); duplex, sequential.
pub struct NamedPipeStream {
    res: Arc<IoRes>,
}

impl NamedPipeStream {
    /// Reads into `buf`; resolves to the byte count and hands the buffer back.
    pub fn read(
        &self,
        buf: Vec<u8>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<u8>)> + Send + use<> {
        let handle = self.res.handle;
        let fut = unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                match ReadFile(handle, Some(&mut buf[..]), None, Some(ovl)) {
                    Ok(()) => Issue::Pending,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        };
        async move { fut.await }
    }

    /// Writes `buf`; resolves to the byte count and hands the buffer back.
    pub fn write(
        &self,
        buf: Vec<u8>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<u8>)> + Send + use<> {
        let handle = self.res.handle;
        let fut = unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                match WriteFile(handle, Some(&buf[..]), None, Some(ovl)) {
                    Ok(()) => Issue::Pending,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        };
        async move { fut.await }
    }
}

/// Server side: each `accept` creates a fresh overlapped pipe instance and
/// waits for a client to connect to it.
pub struct NamedPipeListener {
    rt: Runtime,
    name: HSTRING,
}

impl NamedPipeListener {
    /// Binds a listener to `name` (e.g. `\\.\pipe\foo`). No instance is created
    /// until the first `accept`.
    pub fn bind(rt: &Runtime, name: &str) -> NamedPipeListener {
        NamedPipeListener {
            rt: rt.clone(),
            name: HSTRING::from(name),
        }
    }

    /// Creates a new pipe instance and asynchronously waits for a client.
    pub async fn accept(&self) -> windows_core::Result<NamedPipeStream> {
        let handle = unsafe {
            CreateNamedPipeW(
                &self.name,
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0),
                PIPE_TYPE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                4096,
                0,
                None,
            )
        };
        if handle.is_invalid() {
            return Err(Error::from_thread());
        }
        let res = match IoRes::register(&self.rt.inner, handle, false) {
            Ok(res) => res,
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err(e);
            }
        };
        let h = handle;
        let fut = unsafe {
            submit(&res, (), move |ovl, _| {
                // ERROR_PIPE_CONNECTED (a client connected between
                // CreateNamedPipeW and here) reports success but queues NO
                // completion, so classify it as Failed — submit then balances
                // the StartThreadpoolIo and delivers it synchronously; accept
                // treats it as "connected" below.
                match ConnectNamedPipe(h, Some(ovl)) {
                    Ok(()) => Issue::Pending,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        };
        let (result, ()) = fut.await;
        match result {
            Ok(_) => {}
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {}
            Err(e) => return Err(e),
        }
        Ok(NamedPipeStream { res })
    }
}

/// Client side.
pub struct NamedPipeClient;

impl NamedPipeClient {
    /// Opens the client end of an existing named pipe, waiting once via
    /// `WaitNamedPipeW` if every instance is momentarily busy.
    pub fn connect(rt: &Runtime, name: &str) -> windows_core::Result<NamedPipeStream> {
        let wname = HSTRING::from(name);
        let handle = match Self::open(&wname) {
            Ok(h) => h,
            Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() => {
                unsafe { WaitNamedPipeW(&wname, 5000) }.ok()?;
                Self::open(&wname)?
            }
            Err(e) => return Err(e),
        };
        match IoRes::register(&rt.inner, handle, false) {
            Ok(res) => Ok(NamedPipeStream { res }),
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                Err(e)
            }
        }
    }

    fn open(wname: &HSTRING) -> windows_core::Result<HANDLE> {
        unsafe {
            CreateFileW(
                wname,
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{block_on, Runtime};
    use super::*;

    #[test]
    fn named_pipe_echo() {
        let rt = Runtime::new();
        let name = format!(r"\\.\pipe\tp_pipe_echo_{}", std::process::id());
        let rt2 = rt.clone();
        let sname = name.clone();
        let server = std::thread::spawn(move || {
            let listener = NamedPipeListener::bind(&rt2, &sname);
            let s = block_on(listener.accept()).unwrap();
            let (n, b) = block_on(s.read(vec![0u8; 16]));
            let n = n.unwrap();
            block_on(s.write(b[..n].to_vec())).0.unwrap();
            n
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let client = NamedPipeClient::connect(&rt, &name).unwrap();
        block_on(client.write(b"hello".to_vec())).0.unwrap();
        let (n, b) = block_on(client.read(vec![0u8; 16]));
        assert_eq!(&b[..n.unwrap()], b"hello");
        assert_eq!(server.join().unwrap(), 5);
    }
}
