use super::super::super::{block_on, timeout, Runtime};
use std::time::Duration;
use super::*;

fn temp_path(name: &str) -> String {
    let mut p = std::env::temp_dir();
    p.push(format!("tp_io_test_{}_{name}", std::process::id()));
    p.to_string_lossy().into_owned()
}

/// `PageBuffers` is now an ordinary owned IO buffer as well, so the same
/// pages serve a buffered `read_at`/`write_at` and an unbuffered
/// scatter/gather — one allocation, either path.
#[test]
fn page_buffers_work_as_a_plain_io_buffer() {
    let path = temp_path("pagebuf_plain.bin");
    let rt = Runtime::new();
    block_on(async {
        let f = File::create(&rt, &path).unwrap();
        let mut out = PageBuffers::new(1).unwrap();
        let n = out.len();
        out.as_mut_slice()[..5].copy_from_slice(b"pages");

        // Straight through the generic write path, no Vec involved.
        let (written, out) = f.write_at(out, 0).await;
        assert_eq!(written.unwrap(), n);

        let back = PageBuffers::new(1).unwrap();
        let (read, back) = f.read_at(back, 0).await;
        assert_eq!(read.unwrap(), n);
        assert_eq!(&back.as_slice()[..5], b"pages");
        // And it is still the scatter/gather type it always was.
        assert_eq!(back.pages(), 1);
        assert_eq!(out.pages(), 1);
    });
    let _ = std::fs::remove_file(&path);
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

/// A borrowed buffer at an offset, and end of file reading as 0.
#[test]
fn unchecked_file_ops_take_borrowed_buffers() {
    let rt = Runtime::new();
    let path = std::env::temp_dir().join(format!("tp_unchecked_{}.bin", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    let file = File::create(&rt, &path).unwrap();
    // SAFETY: each future is awaited to completion.
    let (n, _) = block_on(unsafe { file.write_at_unchecked(&b"0123456789"[..], 0, crate::tp::io_event().unwrap()) });
    assert_eq!(n.unwrap(), 10);
    let mut part = [0u8; 4];
    let (n, _) = block_on(unsafe { file.read_at_unchecked(&mut part[..], 3, crate::tp::io_event().unwrap()) });
    assert_eq!(&part[..n.unwrap()], b"3456");
    let (n, _) = block_on(unsafe { file.read_at_unchecked(&mut part[..], 100, crate::tp::io_event().unwrap()) });
    assert_eq!(n.unwrap(), 0, "end of file is a zero-length read");
    drop(file);
    let _ = std::fs::remove_file(&path);
}

/// Two handles on one file: locks are per handle, so this is how they
/// conflict.
fn two_handles(rt: &Runtime, tag: &str) -> (String, File, File) {
    let path = std::env::temp_dir()
        .join(format!("tp_lock_{tag}_{}.bin", std::process::id()))
        .to_string_lossy()
        .into_owned();
    std::fs::write(&path, [0u8; 4096]).unwrap();
    let a = File::open(rt, &path).unwrap();
    let b = File::open(rt, &path).unwrap();
    (path, a, b)
}

#[test]
fn an_exclusive_lock_keeps_others_out_until_dropped() {
    let rt = Runtime::new();
    let (path, a, b) = two_handles(&rt, "excl");
    let held = block_on(a.lock(0, 100, true)).unwrap();
    assert!(block_on(b.try_lock(50, 10, false)).unwrap().is_none(), "overlap admitted");
    assert!(block_on(b.try_lock(100, 10, true)).unwrap().is_some(), "a disjoint range was refused");

    // A waiting lock gets the range once it is released.
    let waiting = std::thread::spawn({
        let rt = rt.clone();
        move || {
            let _ = &rt;
            block_on(b.lock(0, 100, true)).map(|_| ())
        }
    });
    std::thread::sleep(Duration::from_millis(100));
    assert!(!waiting.is_finished(), "a lock went through an exclusive one");
    drop(held);
    waiting.join().unwrap().unwrap();
    drop(a);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn shared_locks_share() {
    let rt = Runtime::new();
    let (path, a, b) = two_handles(&rt, "shared");
    let _one = block_on(a.lock(0, 64, false)).unwrap();
    let two = block_on(b.try_lock(0, 64, false)).unwrap();
    assert!(two.is_some(), "a shared lock refused another shared one");
    drop((a, b));
    let _ = std::fs::remove_file(&path);
}

/// A waiting request dropped by a timeout is withdrawn, not granted
/// later behind the caller's back.
#[test]
fn a_dropped_wait_is_withdrawn() {
    let rt = Runtime::new();
    let (path, a, b) = two_handles(&rt, "cancel");
    let held = block_on(a.lock(0, 8, true)).unwrap();
    let waited = block_on(timeout(&rt, Duration::from_millis(100), b.lock(0, 8, true)));
    assert!(waited.is_err(), "the lock was granted while held");
    drop(held);
    std::thread::sleep(Duration::from_millis(100));
    // Had the cancelled request been granted, `a` could not lock again.
    assert!(block_on(a.try_lock(0, 8, true)).unwrap().is_some(), "the withdrawn request took the range");
    drop((a, b));
    let _ = std::fs::remove_file(&path);
}

/// A lock granted to a future nobody polls again is released, not
/// leaked: the completion sees the grant and, being the last owner,
/// unlocks.
#[test]
fn a_grant_nobody_collects_is_released() {
    let rt = Runtime::new();
    let (path, a, b) = two_handles(&rt, "orphan");
    let held = block_on(a.lock(0, 8, true)).unwrap();
    let orphan = b.lock(0, 8, true); // issued now, waiting in the kernel
    drop(held); // granted to `orphan` ...
    std::thread::sleep(Duration::from_millis(100));
    drop(orphan); // ... which is dropped without ever seeing it
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut free = false;
    while !free && std::time::Instant::now() < deadline {
        free = block_on(a.try_lock(0, 8, true)).unwrap().is_some();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(free, "the orphaned grant was never released");
    drop((a, b));
    let _ = std::fs::remove_file(&path);
}
