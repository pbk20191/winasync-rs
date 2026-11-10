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
use windows::Win32::Foundation::{CloseHandle, E_INVALIDARG, ERROR_IO_PENDING, ERROR_NOTIFY_ENUM_DIR};
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
        match IoRes::register(&rt.inner, handle, false) {
            Ok(res) => Ok(DirectoryWatcher {
                res,
                recursive,
                filter,
            }),
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                Err(e)
            }
        }
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
        let handle = self.res.handle;
        let (recursive, filter) = (self.recursive, FILE_NOTIFY_CHANGE(self.filter.0));
        let fut = unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                let ptr = buf.stable_mut_ptr();
                if !(ptr as usize).is_multiple_of(4) {
                    return Issue::Failed(E_INVALIDARG);
                }
                match ReadDirectoryChangesW(
                    handle,
                    ptr.cast(),
                    IoBuf::bytes_total(buf) as u32,
                    recursive,
                    filter,
                    None,
                    Some(ovl),
                    None,
                ) {
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
