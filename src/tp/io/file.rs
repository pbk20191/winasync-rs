//! Async file IO over TP_IO: files are opened with `FILE_FLAG_OVERLAPPED`
//! and read/written positionally with owned buffers (the buffer must live in
//! the op block so cancellation can hand it to the completion callback).

use super::{submit, IoRes, Issue, OpFuture};
use crate::tp::Runtime;
use core::ffi::c_void;
use std::sync::{Arc, OnceLock};
use windows::Win32::Foundation::{
    ERROR_HANDLE_EOF, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, ReadFileScatter, WriteFile, WriteFileGather, CREATE_ALWAYS,
    FILE_CREATION_DISPOSITION, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_NO_BUFFERING,
    FILE_FLAG_OVERLAPPED, FILE_SEGMENT_ELEMENT, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Memory::{
    VirtualAlloc, VirtualFree, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};

/// The system memory page size (cached), as required for the page-aligned
/// buffers of `ReadFileScatter`/`WriteFileGather`.
fn page_size() -> usize {
    static PS: OnceLock<usize> = OnceLock::new();
    *PS.get_or_init(|| {
        let mut si = SYSTEM_INFO::default();
        unsafe { GetSystemInfo(&mut si) };
        si.dwPageSize as usize
    })
}

/// A set of `pages` page-sized, page-aligned buffers (one `VirtualAlloc`
/// region), for the scatter/gather file IO. Each page becomes one
/// `FILE_SEGMENT_ELEMENT`. The whole region is addressable as one contiguous
/// byte slice for filling/inspecting.
pub struct PageBuffers {
    ptr: *mut u8,
    pages: usize,
    page_size: usize,
}

// The pointer is a private, owned VirtualAlloc region; moving it is sound.
unsafe impl Send for PageBuffers {}

impl PageBuffers {
    /// Allocates `pages` page-aligned pages (must be >= 1).
    pub fn new(pages: usize) -> windows_core::Result<Self> {
        let pages = pages.max(1);
        let page_size = page_size();
        let ptr = unsafe {
            VirtualAlloc(None, pages * page_size, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE)
        };
        if ptr.is_null() {
            return Err(windows_core::Error::from_thread());
        }
        Ok(Self {
            ptr: ptr as *mut u8,
            pages,
            page_size,
        })
    }

    /// The number of pages.
    pub fn pages(&self) -> usize {
        self.pages
    }

    /// The page size these buffers are aligned/sized to.
    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// The total byte length (`pages * page_size`).
    pub fn len(&self) -> usize {
        self.pages * self.page_size
    }

    /// Whether there are zero bytes (never true — there is always >= 1 page).
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The region as one contiguous byte slice.
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len()) }
    }

    /// The region as one contiguous mutable byte slice.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len()) }
    }
}

impl Drop for PageBuffers {
    fn drop(&mut self) {
        unsafe {
            let _ = VirtualFree(self.ptr as *mut c_void, 0, MEM_RELEASE);
        }
    }
}

/// Op-block payload for scatter/gather: owns the page buffers and the segment
/// array (built in place, read by the kernel for the op's duration).
struct VectorPayload {
    buffers: PageBuffers,
    segments: Vec<FILE_SEGMENT_ELEMENT>,
}

unsafe impl Send for VectorPayload {}

impl VectorPayload {
    /// Builds the null-terminated segment array pointing at each page.
    fn build_segments(&mut self) {
        self.segments.clear();
        for i in 0..self.buffers.pages {
            self.segments.push(FILE_SEGMENT_ELEMENT {
                Buffer: unsafe { self.buffers.ptr.add(i * self.buffers.page_size) } as *mut c_void,
            });
        }
        // NULL-terminator element.
        self.segments.push(FILE_SEGMENT_ELEMENT { Alignment: 0 });
    }
}

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
        extra_flags: FILE_FLAGS_AND_ATTRIBUTES,
    ) -> windows_core::Result<File> {
        let handle = unsafe {
            CreateFileW(
                &windows_core::HSTRING::from(path),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                disposition,
                FILE_FLAG_OVERLAPPED | extra_flags,
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
        Self::open_impl(
            rt,
            path,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
        )
    }

    /// Creates (or truncates) a file for reading and writing.
    pub fn create(rt: &Runtime, path: &str) -> windows_core::Result<File> {
        Self::open_impl(
            rt,
            path,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            CREATE_ALWAYS,
            FILE_FLAGS_AND_ATTRIBUTES(0),
        )
    }

    /// Creates (or truncates) a file opened with `FILE_FLAG_NO_BUFFERING`, as
    /// required by [`read_scatter`](Self::read_scatter) /
    /// [`write_gather`](Self::write_gather). With no buffering, all offsets and
    /// transfer sizes must be sector-aligned — the page-aligned, page-sized
    /// [`PageBuffers`] and whole-page offsets satisfy this.
    pub fn create_unbuffered(rt: &Runtime, path: &str) -> windows_core::Result<File> {
        Self::open_impl(
            rt,
            path,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            CREATE_ALWAYS,
            FILE_FLAG_NO_BUFFERING,
        )
    }

    /// Opens an existing file with `FILE_FLAG_NO_BUFFERING` for scatter/gather
    /// IO (see [`create_unbuffered`](Self::create_unbuffered)).
    pub fn open_unbuffered(rt: &Runtime, path: &str) -> windows_core::Result<File> {
        Self::open_impl(
            rt,
            path,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            OPEN_EXISTING,
            FILE_FLAG_NO_BUFFERING,
        )
    }

    /// Skips the completion packet whenever an op on this handle completes
    /// **synchronously**, finishing it on the issuing thread with no pool
    /// hand-off and no context switch.
    ///
    /// Worth little on most file workloads: measured on NTFS, even a re-read
    /// of just-written (cached) data comes back `ERROR_IO_PENDING`, so there
    /// is nothing to skip. It costs nothing to enable and the benefit depends
    /// entirely on whether the file system answers synchronously — profile
    /// before assuming it helps. The socket transports are where this mode
    /// actually pays.
    ///
    /// Enable it before issuing ops. Issue closures report a synchronous
    /// success as a fact and the core applies this mode, so there is no
    /// per-op contract to get wrong; semantics are unchanged, only sooner.
    pub fn set_skip_completion_on_success(&self) -> windows_core::Result<()> {
        self.res.set_skip_completion_on_success()
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
                    Ok(()) => Issue::SyncSuccess,
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
                    Ok(()) => Issue::SyncSuccess,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        }
    }

    /// Scatter-reads `buffers.len()` bytes at absolute offset `pos` into the
    /// page-aligned `buffers` via `ReadFileScatter` (one overlapped op filling
    /// every page). `pos` and the length are whole pages. Resolves to the number
    /// of bytes read (0 at end of file) and hands the buffers back.
    ///
    /// The file must be opened unbuffered (see
    /// [`open_unbuffered`](Self::open_unbuffered)); a buffered handle fails these
    /// with `ERROR_INVALID_PARAMETER`.
    pub fn read_scatter(
        &self,
        buffers: PageBuffers,
        pos: u64,
    ) -> impl Future<Output = (windows_core::Result<usize>, PageBuffers)> + Send + use<> {
        let handle = self.res.handle;
        let count = buffers.len() as u32;
        let cap = buffers.pages() + 1;
        let fut = unsafe {
            submit(
                &self.res,
                VectorPayload {
                    buffers,
                    segments: Vec::with_capacity(cap),
                },
                move |ovl, p| {
                    (*ovl).Anonymous.Anonymous.Offset = pos as u32;
                    (*ovl).Anonymous.Anonymous.OffsetHigh = (pos >> 32) as u32;
                    p.build_segments();
                    match ReadFileScatter(handle, p.segments.as_ptr(), count, None, ovl) {
                        Ok(()) => Issue::SyncSuccess,
                        Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                        Err(e) => Issue::Failed(e.code()),
                    }
                },
            )
        };
        async move {
            let (result, payload) = fut.await;
            let r = match result {
                Err(e) if e.code() == ERROR_HANDLE_EOF.to_hresult() => Ok(0),
                other => other,
            };
            (r, payload.buffers)
        }
    }

    /// Gather-writes `buffers.len()` bytes at absolute offset `pos` from the
    /// page-aligned `buffers` via `WriteFileGather` (one overlapped op draining
    /// every page). `pos` and the length are whole pages. Resolves to the number
    /// of bytes written and hands the buffers back.
    pub fn write_gather(
        &self,
        buffers: PageBuffers,
        pos: u64,
    ) -> impl Future<Output = (windows_core::Result<usize>, PageBuffers)> + Send + use<> {
        let handle = self.res.handle;
        let count = buffers.len() as u32;
        let cap = buffers.pages() + 1;
        let fut = unsafe {
            submit(
                &self.res,
                VectorPayload {
                    buffers,
                    segments: Vec::with_capacity(cap),
                },
                move |ovl, p| {
                    (*ovl).Anonymous.Anonymous.Offset = pos as u32;
                    (*ovl).Anonymous.Anonymous.OffsetHigh = (pos >> 32) as u32;
                    p.build_segments();
                    match WriteFileGather(handle, p.segments.as_ptr(), count, None, ovl) {
                        Ok(()) => Issue::SyncSuccess,
                        Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                        Err(e) => Issue::Failed(e.code()),
                    }
                },
            )
        };
        async move {
            let (result, payload) = fut.await;
            (result, payload.buffers)
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

    /// The socket side of skip-completion is covered in `net::tests`; this is
    /// the file side, which reaches the kernel through `ReadFile`/`WriteFile`
    /// rather than Winsock.
    ///
    /// Unlike the socket test this does **not** assert that the inline path
    /// fires. Measured on NTFS, every op here — including a re-read of
    /// just-written data — returns `ERROR_IO_PENDING`, so there is no
    /// synchronous completion to skip. The mode is correctly in effect; it
    /// simply has nothing to do on this workload. What matters is that
    /// enabling it changes no results.
    #[test]
    fn skip_completion_on_success_file_roundtrip() {
        let path = temp_path("skip_completion.bin");
        let rt = Runtime::new();

        block_on(async {
            let f = File::create(&rt, &path).unwrap();
            f.set_skip_completion_on_success().unwrap();
            let (n, _) = f.write_at(b"skip-completion".to_vec(), 0).await;
            assert_eq!(n.unwrap(), 15);
            // Re-read what we just wrote: it is in the cache, so the read
            // completes without ever going to the disk or the pool.
            let (n, buf) = f.read_at(vec![0u8; 15], 0).await;
            assert_eq!(n.unwrap(), 15);
            assert_eq!(&buf, b"skip-completion");
            // Past end of file: the EOF path is a synchronous *failure*,
            // mapped to Ok(0), and is unaffected by the mode.
            let (n, _) = f.read_at(vec![0u8; 8], 4096).await;
            assert_eq!(n.unwrap(), 0);
        });

        let _ = std::fs::remove_file(&path);
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
    fn scatter_gather_roundtrip() {
        let rt = Runtime::new();
        let path = temp_path("scattergather");
        block_on(async {
            let file = File::create_unbuffered(&rt, &path).unwrap();
            let ps = page_size();

            // Fill two pages with a per-byte pattern and gather-write them.
            let mut wb = PageBuffers::new(2).unwrap();
            for (i, b) in wb.as_mut_slice().iter_mut().enumerate() {
                *b = (i % 251) as u8;
            }
            let expected = wb.as_slice().to_vec();
            let (w, _wb) = file.write_gather(wb, 0).await;
            assert_eq!(w.unwrap(), 2 * ps);

            // Scatter-read them back into fresh pages.
            let rb = PageBuffers::new(2).unwrap();
            let (r, rb) = file.read_scatter(rb, 0).await;
            assert_eq!(r.unwrap(), 2 * ps);
            assert_eq!(rb.as_slice(), &expected[..]);
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
