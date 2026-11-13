//! Files a ring can operate on: owned handles and the trait any handle
//! can answer.

use super::*;

/// A file for ring operations. Cheap to clone; the handle closes with the
/// last clone *and* the last operation that uses it.
#[derive(Clone)]
pub struct File {
    pub(super) handle: Arc<OwnedHandle>,
}

pub(super) struct OwnedHandle(HANDLE);

// A kernel handle, usable from any thread.
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

impl File {
    /// Opens an existing file for reading and writing.
    pub fn open(path: &str) -> Result<File> {
        Self::create_file(path, OPEN_EXISTING)
    }

    /// Creates a file, or truncates an existing one, for reading and writing.
    pub fn create(path: &str) -> Result<File> {
        Self::create_file(path, CREATE_ALWAYS)
    }

    pub(super) fn create_file(
        path: &str,
        disposition: windows::Win32::Storage::FileSystem::FILE_CREATION_DISPOSITION,
    ) -> Result<File> {
        // Not FILE_FLAG_OVERLAPPED: a ring does not need it, and a handle
        // without it still works with every other API.
        let handle = unsafe {
            CreateFileW(
                &HSTRING::from(path),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_READ,
                None,
                disposition,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )?
        };
        Ok(File {
            handle: Arc::new(OwnedHandle(handle)),
        })
    }

}

impl From<std::fs::File> for File {
    fn from(file: std::fs::File) -> Self {
        use std::os::windows::io::IntoRawHandle;
        File {
            handle: Arc::new(OwnedHandle(HANDLE(file.into_raw_handle()))),
        }
    }
}

impl From<std::os::windows::io::OwnedHandle> for File {
    fn from(handle: std::os::windows::io::OwnedHandle) -> Self {
        use std::os::windows::io::IntoRawHandle;
        File {
            handle: Arc::new(OwnedHandle(HANDLE(handle.into_raw_handle()))),
        }
    }
}

impl File {
    /// Takes ownership of `handle`: it is closed with the last clone and the
    /// last operation that uses it.
    ///
    /// # Safety
    /// `handle` must be an open file handle that nothing else will close.
    pub unsafe fn from_raw_handle(handle: HANDLE) -> File {
        File {
            handle: Arc::new(OwnedHandle(handle)),
        }
    }
}

/// A handle a ring operation can work on.
///
/// An operation keeps a clone of it until the kernel is done, which is how
/// a [`File`] or an `Arc<std::fs::File>` stays open for as long as the ring
/// needs it. The safe operations want `'static` handles; the
/// `*_unchecked` ones also take borrowed ones -- `&std::fs::File`, a
/// `BorrowedHandle` -- because their futures wait for the kernel when
/// dropped.
///
/// Any file handle works, opened with `FILE_FLAG_OVERLAPPED` or without,
/// except one bound to an I/O completion port. Measured: an operation on
/// such a handle completes at once with `E_INVALIDARG`, and nothing reaches
/// the port. So a `tp::File`, which its runtime's pool owns, is refused this
/// way rather than mis-delivered -- until it leaves that pool:
/// `tp::File::detach` hands back the bare handle, and `File::from` takes it.
///
/// # Safety
/// [`raw_handle`](Self::raw_handle) must return a handle that stays open for
/// as long as the value it was called on exists -- the contract
/// `AsHandle` has.
pub unsafe trait RingHandle: Send + Sync {
    fn raw_handle(&self) -> HANDLE;

    /// Where this handle sits among a ring's registered files, if it is one
    /// ([`RegisteredFile`]); `None` for everything else.
    #[doc(hidden)]
    fn ring_registration(&self) -> Option<FileRegistration> {
        None
    }
}

/// A registered file's place in its ring; see [`RegisteredFile`].
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileRegistration {
    pub ring: usize,
    pub generation: u64,
    pub index: u32,
}

unsafe impl RingHandle for File {
    fn raw_handle(&self) -> HANDLE {
        self.handle.0
    }
}

/// Any shared owner of a handle: `Arc<std::fs::File>`, `Arc<OwnedHandle>`.
unsafe impl<T: std::os::windows::io::AsHandle + Send + Sync> RingHandle for Arc<T> {
    fn raw_handle(&self) -> HANDLE {
        use std::os::windows::io::AsRawHandle;
        HANDLE(self.as_handle().as_raw_handle())
    }
}

/// A borrow: `&std::fs::File`. With a `'static` borrow any operation takes
/// it; with a shorter one, only the `*_unchecked` operations.
unsafe impl<T: std::os::windows::io::AsHandle + Sync> RingHandle for &T {
    fn raw_handle(&self) -> HANDLE {
        use std::os::windows::io::AsRawHandle;
        HANDLE(self.as_handle().as_raw_handle())
    }
}

unsafe impl RingHandle for std::os::windows::io::BorrowedHandle<'_> {
    fn raw_handle(&self) -> HANDLE {
        use std::os::windows::io::AsRawHandle;
        HANDLE(self.as_raw_handle())
    }
}

/// The reference an entry names `file` by: its registered index, or its raw
/// handle. Not validated -- [`BuildCtx::file_ref`] is what checks.
pub(super) fn handle_ref<H: RingHandle>(file: &H) -> IORING_HANDLE_REF {
    match file.ring_registration() {
        Some(r) => IORING_HANDLE_REF {
            Kind: IORING_REF_REGISTERED,
            Handle: IORING_HANDLE_REF_0 { Index: r.index },
        },
        None => IORING_HANDLE_REF {
            Kind: IORING_REF_RAW,
            Handle: IORING_HANDLE_REF_0 {
                Handle: file.raw_handle(),
            },
        },
    }
}

// ---------------------------------------------------------------------------
// The ring
// ---------------------------------------------------------------------------
