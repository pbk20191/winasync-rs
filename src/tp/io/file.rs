//! Async file IO over TP_IO: files are opened with `FILE_FLAG_OVERLAPPED`
//! and read/written positionally with owned buffers (the buffer must live in
//! the op block so cancellation can hand it to the completion callback).

use super::{submit, IoRes, Issue, OpFuture};
use crate::tp::Runtime;
use std::sync::Arc;
use windows::Win32::Foundation::{
    ERROR_HANDLE_EOF, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, CREATE_ALWAYS, FILE_CREATION_DISPOSITION,
    FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

/// An overlapped file handle bound to a [`Runtime`]'s pool.
pub struct File {
    res: Arc<IoRes>,
}

impl File {
    fn open_impl(
        rt: &Runtime,
        path: &str,
        access: u32,
        disposition: FILE_CREATION_DISPOSITION,
    ) -> windows_core::Result<File> {
        let handle = unsafe {
            CreateFileW(
                &windows_core::HSTRING::from(path),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                disposition,
                FILE_FLAG_OVERLAPPED,
                None,
            )?
        };
        match IoRes::register(&rt.inner, handle, false) {
            Ok(res) => Ok(File { res }),
            Err(e) => {
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(handle);
                }
                Err(e)
            }
        }
    }

    /// Opens an existing file for reading and writing.
    pub fn open(rt: &Runtime, path: &str) -> windows_core::Result<File> {
        Self::open_impl(rt, path, GENERIC_READ.0 | GENERIC_WRITE.0, OPEN_EXISTING)
    }

    /// Creates (or truncates) a file for reading and writing.
    pub fn create(rt: &Runtime, path: &str) -> windows_core::Result<File> {
        Self::open_impl(rt, path, GENERIC_READ.0 | GENERIC_WRITE.0, CREATE_ALWAYS)
    }

    /// The raw file handle (for `TcpStream::transmit_file`). The returned handle
    /// is only valid while this `File` (or a shared [`res_arc`](Self::res_arc)
    /// clone) is alive.
    pub(crate) fn handle(&self) -> windows::Win32::Foundation::HANDLE {
        self.res.handle
    }

    /// A clone of the refcounted IO resource, used to keep the file handle alive
    /// for the duration of a `TransmitFile` op issued on a *socket*.
    pub(crate) fn res_arc(&self) -> Arc<IoRes> {
        self.res.clone()
    }

    fn start_read(&self, buf: Vec<u8>, pos: u64) -> OpFuture<Vec<u8>> {
        let handle = self.res.handle;
        unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                (*ovl).Anonymous.Anonymous.Offset = pos as u32;
                (*ovl).Anonymous.Anonymous.OffsetHigh = (pos >> 32) as u32;
                match ReadFile(handle, Some(&mut buf[..]), None, Some(ovl)) {
                    Ok(()) => Issue::Pending,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        }
    }

    /// Reads into `buf` at absolute offset `pos`. Resolves to the number of
    /// bytes read (0 at end of file) and hands the buffer back.
    pub fn read_at(
        &self,
        buf: Vec<u8>,
        pos: u64,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<u8>)> + Send {
        let fut = self.start_read(buf, pos);
        async move {
            let (result, buf) = fut.await;
            match result {
                Err(e) if e.code() == ERROR_HANDLE_EOF.to_hresult() => (Ok(0), buf),
                other => (other, buf),
            }
        }
    }

    /// Writes `buf` at absolute offset `pos`. Resolves to the number of bytes
    /// written and hands the buffer back.
    pub fn write_at(
        &self,
        buf: Vec<u8>,
        pos: u64,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<u8>)> + Send {
        let handle = self.res.handle;
        unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                (*ovl).Anonymous.Anonymous.Offset = pos as u32;
                (*ovl).Anonymous.Anonymous.OffsetHigh = (pos >> 32) as u32;
                match WriteFile(handle, Some(&buf[..]), None, Some(ovl)) {
                    Ok(()) => Issue::Pending,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{block_on, Runtime};
    use super::*;

    fn temp_path(name: &str) -> String {
        let mut p = std::env::temp_dir();
        p.push(format!("tp_io_test_{}_{name}", std::process::id()));
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn file_roundtrip() {
        let rt = Runtime::new();
        let path = temp_path("roundtrip");
        let pattern: Vec<u8> = (0..1024 * 1024u32).map(|i| (i % 251) as u8).collect();

        block_on(async {
            let file = File::create(&rt, &path).unwrap();
            let (written, _) = file.write_at(pattern.clone(), 0).await;
            assert_eq!(written.unwrap(), pattern.len());

            // Read back in two chunks at different offsets.
            let (n, half) = file.read_at(vec![0u8; 512 * 1024], 0).await;
            assert_eq!(n.unwrap(), 512 * 1024);
            assert_eq!(half[..], pattern[..512 * 1024]);

            let (n, rest) = file.read_at(vec![0u8; 600 * 1024], 512 * 1024).await;
            assert_eq!(n.unwrap(), 512 * 1024); // only half the file remains
            assert_eq!(rest[..512 * 1024], pattern[512 * 1024..]);

            // Past EOF reads 0.
            let (n, _) = file.read_at(vec![0u8; 16], 2 * 1024 * 1024).await;
            assert_eq!(n.unwrap(), 0);
        });
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dropped_read_future_does_not_leak() {
        let rt = Runtime::new();
        let path = temp_path("cancel");
        block_on(async {
            let file = File::create(&rt, &path).unwrap();
            let (w, _) = file.write_at(vec![7u8; 4096], 0).await;
            w.unwrap();
            for _ in 0..1000 {
                // Start a read and drop the future immediately.
                drop(file.read_at(vec![0u8; 4096], 0));
            }
        });
        drop(rt);
        #[cfg(debug_assertions)]
        {
            // Completions drain quickly once the pool is gone. LIVE_OPS is
            // process-global, so wait out ops from concurrently running tests.
            for _ in 0..500 {
                if super::super::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert_eq!(
                super::super::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "op blocks leaked"
            );
        }
        let _ = std::fs::remove_file(&path);
    }
}
