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
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
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
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        self.read_with(buf, true)
    }

    fn read_with<B: IoBufMut + 'static>(
        &self,
        buf: B,
        extended: bool,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
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
mod tests {
    use super::super::super::{block_on, timeout};
    use super::*;
    use std::time::{Duration, Instant};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tp_watch_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Collects changes until `done` says enough, or five seconds pass.
    fn collect(
        rt: &Runtime,
        watcher: &DirectoryWatcher,
        done: impl Fn(&[Change]) -> bool,
    ) -> Vec<Change> {
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(&seen) && Instant::now() < deadline {
            if let Ok((n, buf)) = block_on(timeout(rt, Duration::from_millis(500), watcher.read(Vec::with_capacity(4096)))) {
                let n = n.unwrap();
                seen.extend(iter_changes(&buf[..n]));
            }
        }
        seen
    }

    #[test]
    fn creates_renames_and_deletes_are_reported() {
        let rt = Runtime::new();
        let dir = temp_dir("names");
        let watcher =
            DirectoryWatcher::open(&rt, dir.to_str().unwrap(), false, NotifyFilter::FILE_NAME).unwrap();
        // The first read sets up the kernel's buffer; start it before the
        // changes so none are missed.
        let first = std::thread::spawn({
            let dir = dir.clone();
            move || {
                std::thread::sleep(Duration::from_millis(100));
                std::fs::write(dir.join("a.txt"), b"x").unwrap();
                std::fs::rename(dir.join("a.txt"), dir.join("b.txt")).unwrap();
                std::fs::remove_file(dir.join("b.txt")).unwrap();
            }
        });
        let want = [
            Change { kind: ChangeKind::Added, path: "a.txt".into() },
            Change { kind: ChangeKind::RenamedFrom, path: "a.txt".into() },
            Change { kind: ChangeKind::RenamedTo, path: "b.txt".into() },
            Change { kind: ChangeKind::Removed, path: "b.txt".into() },
        ];
        let seen = collect(&rt, &watcher, |seen| want.iter().all(|w| seen.contains(w)));
        first.join().unwrap();
        for w in &want {
            assert!(seen.contains(w), "missing {w:?}; saw {seen:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_recursive_watch_names_paths_below_the_directory() {
        let rt = Runtime::new();
        let dir = temp_dir("deep");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let watcher =
            DirectoryWatcher::open(&rt, dir.to_str().unwrap(), true, NotifyFilter::FILE_NAME).unwrap();
        let writer = std::thread::spawn({
            let dir = dir.clone();
            move || {
                std::thread::sleep(Duration::from_millis(100));
                std::fs::write(dir.join("sub").join("deep.txt"), b"x").unwrap();
            }
        });
        let want = Change {
            kind: ChangeKind::Added,
            path: PathBuf::from("sub").join("deep.txt"),
        };
        let seen = collect(&rt, &watcher, |seen| seen.contains(&want));
        writer.join().unwrap();
        assert!(seen.contains(&want), "saw {seen:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// More changes than the kernel's buffer holds come back as zero bytes.
    #[test]
    fn an_overflow_reads_as_zero_bytes() {
        let rt = Runtime::new();
        let dir = temp_dir("overflow");
        let watcher =
            DirectoryWatcher::open(&rt, dir.to_str().unwrap(), false, NotifyFilter::FILE_NAME).unwrap();
        // A small first read sizes the kernel's buffer small.
        let pending = watcher.read(Vec::with_capacity(64));
        let (n, _) = block_on(async {
            // Let the read reach the kernel, then flood it.
            let flood = {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(100));
                    for i in 0..200 {
                        std::fs::write(dir.join(format!("f{i:03}.txt")), b"x").unwrap();
                    }
                })
            };
            let r = pending.await;
            flood.join().unwrap();
            r
        });
        n.unwrap();
        // The first read may have caught the first few; the kernel kept
        // collecting into a 64-byte buffer meanwhile, which 200 creates
        // overflow.
        let (n, _) = block_on(timeout(&rt, Duration::from_secs(5), watcher.read(Vec::with_capacity(64))))
            .expect("no read after the flood");
        assert_eq!(n.unwrap(), 0, "an overflow did not read as zero bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_misaligned_buffer_is_refused() {
        let rt = Runtime::new();
        let dir = temp_dir("align");
        let watcher =
            DirectoryWatcher::open(&rt, dir.to_str().unwrap(), false, NotifyFilter::FILE_NAME).unwrap();
        let buf = Vec::with_capacity(128).slice(1..);
        let (r, _) = block_on(watcher.read(buf));
        assert_eq!(r.unwrap_err().code(), E_INVALIDARG);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Extended records carry the file's identity: a rename keeps its ID,
    /// and the size shows up once written.
    #[test]
    fn extended_records_carry_ids_and_sizes() {
        let rt = Runtime::new();
        let dir = temp_dir("extended");
        let watcher = DirectoryWatcher::open(
            &rt,
            dir.to_str().unwrap(),
            false,
            NotifyFilter::FILE_NAME | NotifyFilter::SIZE | NotifyFilter::LAST_WRITE,
        )
        .unwrap();
        let writer = std::thread::spawn({
            let dir = dir.clone();
            move || {
                std::thread::sleep(Duration::from_millis(100));
                std::fs::write(dir.join("e.txt"), b"12345").unwrap();
                std::fs::rename(dir.join("e.txt"), dir.join("f.txt")).unwrap();
            }
        });
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        let done = |seen: &[ExtendedChange]| seen.iter().any(|c| c.kind == ChangeKind::RenamedTo);
        while !done(&seen) && Instant::now() < deadline {
            if let Ok((n, buf)) =
                block_on(timeout(&rt, Duration::from_millis(500), watcher.read_extended(Vec::with_capacity(8192))))
            {
                let n = n.unwrap();
                seen.extend(iter_extended_changes(&buf[..n]));
            }
        }
        writer.join().unwrap();
        let from = seen.iter().find(|c| c.kind == ChangeKind::RenamedFrom).expect("no rename-from");
        let to = seen.iter().find(|c| c.kind == ChangeKind::RenamedTo).expect("no rename-to");
        assert_eq!(to.path, PathBuf::from("f.txt"));
        assert_ne!(to.file_id, 0);
        assert_eq!(from.file_id, to.file_id, "a rename changed the file ID");
        assert_ne!(to.parent_file_id, 0);
        assert_eq!(to.size, 5, "saw {seen:?}");
        assert!(filetime(to.last_written).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn records_parse_and_stop_at_a_short_buffer() {
        let mut bytes = Vec::new();
        for (i, name) in ["x", "yz"].iter().enumerate() {
            let wide: Vec<u8> = name.encode_utf16().flat_map(|u| u.to_ne_bytes()).collect();
            let next = if i == 0 { 12 + 4 } else { 0 };
            bytes.extend_from_slice(&(next as u32).to_ne_bytes());
            bytes.extend_from_slice(&1u32.to_ne_bytes());
            bytes.extend_from_slice(&(wide.len() as u32).to_ne_bytes());
            bytes.extend_from_slice(&wide);
            // Records start on 4-byte boundaries, as the kernel lays them out.
            while i == 0 && bytes.len() % 4 != 0 {
                bytes.push(0);
            }
        }
        let all: Vec<_> = iter_changes(&bytes).collect();
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].path, PathBuf::from("yz"));
        // Truncated: the second record's name runs off the end.
        assert_eq!(iter_changes(&bytes[..bytes.len() - 1]).count(), 1);
    }
}
