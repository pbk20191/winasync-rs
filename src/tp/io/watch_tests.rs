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
