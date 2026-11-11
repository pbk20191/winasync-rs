//! Directory change notifications (`ReadDirectoryChangesW`) over TP_IO.
//!
//! A [`DirectoryWatcher`] holds a directory open; each [`read`](DirectoryWatcher::read)
//! waits for changes and fills a buffer with `FILE_NOTIFY_INFORMATION`
//! records, which [`iter_changes`] turns into [`Change`]s.
//!
//! The kernel keeps collecting changes between reads, in a buffer the size
//! of the *first* read's -- so size that one for the bursts you expect. When
//! more happens than it holds, the changes are dropped and the next read
//! reports zero bytes: the caller has lost track and should rescan the
//! directory.

use crate::common::buffer::BufResult;
use super::{submit, IoRes, Issue};
use crate::common::buffer::{mark_read, IoBuf, IoBufMut};
use crate::tp::Runtime;
use std::path::PathBuf;
use std::sync::Arc;
use windows::Win32::Foundation::{E_INVALIDARG, ERROR_IO_PENDING, ERROR_NOTIFY_ENUM_DIR};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_ACTION, FILE_ACTION_ADDED, FILE_ACTION_MODIFIED,
    FILE_ACTION_REMOVED, FILE_ACTION_RENAMED_NEW_NAME, FILE_ACTION_RENAMED_OLD_NAME,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_core::HSTRING;

/// What a watcher reports changes to (`FILE_NOTIFY_CHANGE_*`). Combine with
/// `|`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotifyFilter(u32);

impl NotifyFilter {
    /// A file created, deleted or renamed.
    pub const FILE_NAME: Self = Self(1);
    /// A directory created, deleted or renamed.
    pub const DIR_NAME: Self = Self(2);
    pub const ATTRIBUTES: Self = Self(4);
    /// Reported when the size change reaches the disk, which caching can
    /// delay.
    pub const SIZE: Self = Self(8);
    /// Likewise subject to caching.
    pub const LAST_WRITE: Self = Self(16);
    pub const LAST_ACCESS: Self = Self(32);
    pub const CREATION: Self = Self(64);
    pub const SECURITY: Self = Self(256);
}

impl core::ops::BitOr for NotifyFilter {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// What happened to a [`Change`]'s path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
    /// The name before a rename; the next change is usually its
    /// [`RenamedTo`](Self::RenamedTo).
    RenamedFrom,
    RenamedTo,
    /// An action this crate does not know.
    Other(u32),
}

impl From<FILE_ACTION> for ChangeKind {
    fn from(action: FILE_ACTION) -> Self {
        match action {
            FILE_ACTION_ADDED => Self::Added,
            FILE_ACTION_REMOVED => Self::Removed,
            FILE_ACTION_MODIFIED => Self::Modified,
            FILE_ACTION_RENAMED_OLD_NAME => Self::RenamedFrom,
            FILE_ACTION_RENAMED_NEW_NAME => Self::RenamedTo,
            other => Self::Other(other.0),
        }
    }
}

/// One change, with its path relative to the watched directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    pub path: PathBuf,
}

/// The changes in the first `n` bytes a [`DirectoryWatcher::read`] filled.
///
/// Stops at a malformed record rather than reading past the buffer.
pub fn iter_changes(bytes: &[u8]) -> impl Iterator<Item = Change> + '_ {
    use std::os::windows::ffi::OsStringExt;
    // FILE_NOTIFY_INFORMATION: NextEntryOffset, Action, FileNameLength (all
    // u32), then the name in UTF-16, not terminated.
    const HEADER: usize = 12;
    let mut at = Some(0usize);
    std::iter::from_fn(move || {
        let start = at?;
        let header = bytes.get(start..start + HEADER)?;
        let word = |i: usize| u32::from_ne_bytes(header[i..i + 4].try_into().unwrap());
        let (next, action, name_len) = (word(0) as usize, word(4), word(8) as usize);
        let name = bytes.get(start + HEADER..start + HEADER + name_len)?;
        let wide: Vec<u16> = name
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_ne_bytes(c))
            .collect();
        at = (next != 0).then_some(start + next);
        Some(Change {
            kind: FILE_ACTION(action).into(),
            path: PathBuf::from(std::ffi::OsString::from_wide(&wide)),
        })
    })
}

/// One change with everything `ReadDirectoryChangesExW` reports about the
/// file. Times are `FILETIME`s (100 ns since 1601; see [`filetime`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtendedChange {
    pub kind: ChangeKind,
    pub path: PathBuf,
    /// The file's ID on its volume -- stable across renames, so a
    /// [`ChangeKind::RenamedFrom`] and its [`ChangeKind::RenamedTo`] share it.
    pub file_id: i64,
    pub parent_file_id: i64,
    pub size: i64,
    pub allocated_size: i64,
    pub attributes: u32,
    pub created: i64,
    pub last_written: i64,
    pub last_changed: i64,
    pub last_accessed: i64,
}

/// A `FILETIME` as a `SystemTime`; `None` before the Unix epoch.
pub fn filetime(ft: i64) -> Option<std::time::SystemTime> {
    // 100 ns intervals between 1601-01-01 and 1970-01-01.
    const UNIX_EPOCH_AS_FILETIME: i64 = 116_444_736_000_000_000;
    let since_unix = ft.checked_sub(UNIX_EPOCH_AS_FILETIME)?;
    let since_unix = u64::try_from(since_unix).ok()?;
    Some(std::time::UNIX_EPOCH + std::time::Duration::from_nanos(since_unix.saturating_mul(100)))
}

/// The changes in the first `n` bytes a [`DirectoryWatcher::read_extended`]
/// filled. Stops at a malformed record rather than reading past the buffer.
pub fn iter_extended_changes(bytes: &[u8]) -> impl Iterator<Item = ExtendedChange> + '_ {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::Storage::FileSystem::FILE_NOTIFY_EXTENDED_INFORMATION;
    let name_at = core::mem::offset_of!(FILE_NOTIFY_EXTENDED_INFORMATION, FileName);
    let mut at = Some(0usize);
    std::iter::from_fn(move || {
        let start = at?;
        // The fixed part, read unaligned so the caller's buffer alignment is
        // not this parser's problem.
        let fixed = bytes.get(start..start + name_at)?;
        let mut record = FILE_NOTIFY_EXTENDED_INFORMATION::default();
        unsafe {
            core::ptr::copy_nonoverlapping(
                fixed.as_ptr(),
                (&mut record as *mut FILE_NOTIFY_EXTENDED_INFORMATION).cast::<u8>(),
                name_at,
            )
        };
        let name_len = record.FileNameLength as usize;
        let name = bytes.get(start + name_at..start + name_at + name_len)?;
        let wide: Vec<u16> = name.as_chunks::<2>().0.iter().map(|&c| u16::from_ne_bytes(c)).collect();
        let next = record.NextEntryOffset as usize;
        at = (next != 0).then_some(start + next);
        Some(ExtendedChange {
            kind: record.Action.into(),
            path: PathBuf::from(std::ffi::OsString::from_wide(&wide)),
            file_id: record.FileId,
            parent_file_id: record.ParentFileId,
            size: record.FileSize,
            allocated_size: record.AllocatedLength,
            attributes: record.FileAttributes,
            created: record.CreationTime,
            last_written: record.LastModificationTime,
            last_changed: record.LastChangeTime,
            last_accessed: record.LastAccessTime,
        })
    })
}

/// A directory being watched for changes.
pub struct DirectoryWatcher {
    res: Arc<IoRes>,
    recursive: bool,
    filter: NotifyFilter,
}

impl std::os::windows::io::AsHandle for DirectoryWatcher {
    fn as_handle(&self) -> std::os::windows::io::BorrowedHandle<'_> {
        // SAFETY: the registration owns the handle and outlives this borrow.
        unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(self.res.handle.0) }
    }
}

impl DirectoryWatcher {
    /// Watches `path` -- and, if `recursive`, everything under it -- for
    /// the changes `filter` names.
    ///
    /// The directory is opened sharing read, write and delete, so the
    /// watcher does not stop anyone renaming or deleting what it watches.
    pub fn open(
        rt: &Runtime,
        path: &str,
        recursive: bool,
        filter: NotifyFilter,
    ) -> windows_core::Result<DirectoryWatcher> {
        let handle = unsafe {
            CreateFileW(
                &HSTRING::from(path),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                // Backup semantics is how a directory is opened at all.
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                None,
            )?
        };
        let res = crate::tp::io::register_on(&rt.inner, unsafe { crate::common::overlapped::IoHandle::from_handle(handle) })?;
        Ok(DirectoryWatcher {
            res,
            recursive,
            filter,
        })
    }

    /// Waits for changes and fills `buf` with them; resolves to the bytes
    /// written, for [`iter_changes`], and hands the buffer back.
    ///
    /// Zero bytes means changes were lost -- more happened than the
    /// kernel's buffer held -- and the directory should be rescanned. The
    /// buffer must be 4-byte aligned (a `Vec` is; a `Slice` at an odd offset
    /// may not be): `E_INVALIDARG` otherwise. Dropping the future cancels
    /// the wait.
    pub fn read<B: IoBufMut + 'static>(
        &self,
        buf: B,
    ) -> impl Future<Output = BufResult<usize, B>> + Send + use<B> {
        self.read_with(buf, false)
    }

    /// [`read`](Self::read), with records that also carry each file's ID,
    /// its parent's, its size, attributes and timestamps
    /// (`ReadDirectoryChangesExW`, Windows 10 1709 and later); parse them
    /// with [`iter_extended_changes`]. The buffer must be 8-byte aligned,
    /// as the records hold 64-bit fields.
    pub fn read_extended<B: IoBufMut + 'static>(
        &self,
        buf: B,
    ) -> impl Future<Output = BufResult<usize, B>> + Send + use<B> {
        self.read_with(buf, true)
    }

    fn read_with<B: IoBufMut + 'static>(
        &self,
        buf: B,
        extended: bool,
    ) -> impl Future<Output = BufResult<usize, B>> + Send + use<B> {
        use windows::Win32::Storage::FileSystem::{
            ReadDirectoryChangesExW, ReadDirectoryNotifyExtendedInformation,
        };
        let handle = self.res.handle;
        let (recursive, filter) = (self.recursive, FILE_NOTIFY_CHANGE(self.filter.0));
        let align = if extended { 8 } else { 4 };
        let fut = unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                let ptr = buf.stable_mut_ptr();
                if !(ptr as usize).is_multiple_of(align) {
                    return Issue::Failed(E_INVALIDARG);
                }
                let len = IoBuf::bytes_total(buf) as u32;
                let issued = if extended {
                    ReadDirectoryChangesExW(
                        handle,
                        ptr.cast(),
                        len,
                        recursive,
                        filter,
                        None,
                        Some(ovl),
                        None,
                        ReadDirectoryNotifyExtendedInformation,
                    )
                } else {
                    ReadDirectoryChangesW(handle, ptr.cast(), len, recursive, filter, None, Some(ovl), None)
                };
                match issued {
                    Ok(()) => Issue::SyncSuccess,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        };
        async move {
            let (result, buf) = fut.await;
            // An overflow can come back either way: as a success with no
            // bytes, or as this error. Both mean "rescan".
            let result = match result {
                Err(e) if e.code() == ERROR_NOTIFY_ENUM_DIR.to_hresult() => Ok(0),
                other => other,
            };
            let n = result.as_ref().ok().copied();
            (result, mark_read(n, buf))
        }
    }
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;
