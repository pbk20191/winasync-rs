use super::*;
use crate::common::block_on;
use std::time::{Duration, Instant};

fn temp_file(tag: &str, contents: &[u8]) -> (String, File) {
    let path = std::env::temp_dir()
        .join(format!("ioring_{tag}_{}.bin", std::process::id()))
        .to_string_lossy()
        .into_owned();
    std::fs::write(&path, contents).unwrap();
    let file = File::open(&path).unwrap();
    (path, file)
}

fn in_flight(ring: &IoRing) -> u32 {
    ring.inner.ring.lock().unwrap().in_flight
}

/// Waits for the ring to have nothing in flight -- cancelled operations
/// still have to complete.
fn settle(ring: &IoRing) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while in_flight(ring) > 0 && Instant::now() < deadline {
        ring.drive();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(in_flight(ring), 0, "operations were left in flight");
}

#[test]
fn write_then_read() {
    let ring = IoRing::new(8).unwrap();
    let (path, file) = temp_file("rw", b"");
    let (n, _) = block_on(ring.write(&file, b"ring-written".to_vec(), 0));
    assert_eq!(n.unwrap(), 12);
    // Spare capacity, not zeroed bytes: the length ends at the data.
    let (n, got) = block_on(ring.read(&file, Vec::with_capacity(64), 0));
    assert_eq!(n.unwrap(), 12);
    assert_eq!(got, b"ring-written");
    block_on(ring.flush(&file)).unwrap();
    drop(file);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn reading_past_the_end_reads_nothing() {
    let ring = IoRing::new(4).unwrap();
    let (path, file) = temp_file("eof", b"short");
    let (n, _) = block_on(ring.read(&file, vec![0u8; 16], 1000));
    assert_eq!(n.unwrap(), 0);
    drop(file);
    let _ = std::fs::remove_file(&path);
}

/// The point of a ring: operations created together go to the kernel in
/// one submission.
#[test]
fn operations_created_together_are_submitted_together() {
    let ring = IoRing::new(32).unwrap();
    let data: Vec<u8> = (0..16u8).flat_map(|i| [i; 512]).collect();
    let (path, file) = temp_file("batch", &data);
    let ops: Vec<_> = (0..16u64)
        .map(|i| ring.read(&file, Vec::with_capacity(512), i * 512))
        .collect();
    assert_eq!(ring.submit().unwrap(), 16, "the reads did not go as one batch");
    for (i, op) in ops.into_iter().enumerate() {
        let (n, got) = block_on(op);
        assert_eq!(n.unwrap(), 512);
        assert!(got.iter().all(|&b| b == i as u8), "read {i} got the wrong bytes");
    }
    drop(file);
    let _ = std::fs::remove_file(&path);
}

/// A `Batch`: the reads collected go in one submission, and the results
/// come back in the order the reads were added.
#[test]
fn a_batch_submits_once_and_resolves_in_order() {
    let ring = IoRing::new(32).unwrap();
    let data: Vec<u8> = (0..16u8).flat_map(|i| [i; 512]).collect();
    let (path, file) = temp_file("batch_in_order", &data);
    // Added in reverse, so order of results is order of adding, not of offset.
    let batch = ring.read_batch(&file, (0..16u64).rev().map(|i| (vec![0u8; 512], i * 512)));
    assert_eq!(batch.len(), 16);
    assert_eq!(ring.inner.ring.lock().unwrap().unsubmitted, 16, "built, not yet submitted");
    let results = block_on(batch);
    assert_eq!(ring.inner.ring.lock().unwrap().unsubmitted, 0);
    assert_eq!(results.len(), 16);
    for (slot, (n, got)) in results.into_iter().enumerate() {
        let i = 15 - slot as u8;
        assert_eq!(n.unwrap(), 512);
        assert!(got.iter().all(|&b| b == i), "result {slot} is not read {i}");
    }
    // Collected from any operations, the empty batch included.
    let empty: Batch<ReadOp<File, Vec<u8>>> = std::iter::empty().collect();
    assert!(block_on(empty).is_empty());
    drop(file);
    let _ = std::fs::remove_file(&path);
}

/// A batch larger than the ring: the overflow waits for room, and the
/// batch still resolves with every result. Dropped half-way, it cancels
/// and the ring settles.
#[test]
fn a_batch_past_the_queue_completes_and_a_dropped_one_settles() {
    let ring = IoRing::new(2).unwrap();
    let (path, file) = temp_file("batch_room", &[9u8; 4096]);
    let batch = ring.read_batch(&file, (0..40u64).map(|i| (vec![0u8; 64], i * 64)));
    let results = block_on(batch);
    assert_eq!(results.len(), 40);
    assert!(results.iter().all(|(n, b)| *n.as_ref().unwrap() == 64 && b.iter().all(|&x| x == 9)));

    let batch = ring.read_batch(&file, (0..40u64).map(|i| (vec![0u8; 64], i * 64)));
    assert_eq!(batch.pending(), 40);
    drop(batch);
    settle(&ring);
    drop(file);
    let _ = std::fs::remove_file(&path);
}

/// More operations than the completion queue holds: the rest wait for
/// room instead of overflowing it, and all of them finish.
#[test]
fn operations_past_the_queue_wait_for_room() {
    let ring = IoRing::new(1).unwrap();
    let (path, file) = temp_file("room", &[7u8; 64]);
    let ops: Vec<_> = (0..10u64).map(|i| ring.read(&file, vec![0u8; 4], i * 4)).collect();
    assert!(in_flight(&ring) <= 2, "more in flight than the completion queue holds");
    for op in ops {
        let (n, got) = block_on(op);
        assert_eq!(n.unwrap(), 4);
        assert_eq!(&got[..4], &[7u8; 4]);
    }
    drop(file);
    let _ = std::fs::remove_file(&path);
}

/// Without a pump nothing arrives until the caller drives the ring.
#[test]
fn a_manual_ring_delivers_when_driven() {
    let ring = IoRing::manual(4).unwrap();
    let (path, file) = temp_file("manual", b"driven");
    let mut op = ring.read(&file, Vec::with_capacity(16), 0);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    assert!(Pin::new(&mut op).poll(&mut cx).is_pending());
    let deadline = Instant::now() + Duration::from_secs(5);
    let (n, got) = loop {
        ring.drive();
        if let Poll::Ready(r) = Pin::new(&mut op).poll(&mut cx) {
            break r;
        }
        assert!(Instant::now() < deadline, "never completed");
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(&got[..n.unwrap()], b"driven");
    drop(file);
    let _ = std::fs::remove_file(&path);
}

/// A future dropped before it finishes leaves nothing behind once the
/// kernel is done with its buffer.
#[test]
fn a_dropped_operation_is_settled() {
    let ring = IoRing::new(8).unwrap();
    let (path, file) = temp_file("drop", &[1u8; 4096]);
    for i in 0..8u64 {
        drop(ring.read(&file, vec![0u8; 512], i * 512));
    }
    settle(&ring);
    drop(file);
    let _ = std::fs::remove_file(&path);
}

/// Borrowed buffers, and an early drop that returns.
#[test]
fn unchecked_operations_take_borrowed_buffers() {
    let ring = IoRing::new(4).unwrap();
    let (path, file) = temp_file("borrow", b"");
    let out = *b"borrowed";
    let mut back = [0u8; 8];
    // SAFETY: each future is awaited or dropped, never forgotten.
    unsafe {
        let (n, _) = block_on(ring.write_unchecked(&file, &out[..], 0));
        assert_eq!(n.unwrap(), 8);
        let (n, _) = block_on(ring.read_unchecked(&file, &mut back[..], 0));
        assert_eq!(n.unwrap(), 8);
        drop(ring.read_unchecked(&file, &mut back[..], 0));
    }
    assert_eq!(&back, b"borrowed");
    settle(&ring);
    drop(file);
    let _ = std::fs::remove_file(&path);
}

/// Slices and foreign memory work like any other buffer.
#[test]
fn slices_and_raw_buffers() {
    use crate::common::buffer::{RawBuf, Region};
    let ring = IoRing::new(4).unwrap();
    let (path, file) = temp_file("bufs", b"0123456789");
    let mut head = Vec::with_capacity(16);
    head.extend_from_slice(b"at:");
    let (n, rest) = block_on(ring.read(&file, head.slice(3..7), 2));
    assert_eq!(n.unwrap(), 4);
    assert_eq!(rest.into_inner(), b"at:2345");
    let region = Region::new(4096).unwrap();
    let raw = unsafe { RawBuf::from_raw_parts(region.base(), 0, 4096, region) };
    let (n, raw) = block_on(ring.read(&file, raw, 0));
    assert_eq!(n.unwrap(), 10);
    assert_eq!(raw.as_slice(), b"0123456789");
    drop(file);
    let _ = std::fs::remove_file(&path);
}

#[cfg(feature = "tp")]
#[test]
fn pumped_by_a_tp_runtime() {
    let rt = crate::tp::Runtime::new();
    let ring = IoRing::with_runtime(&rt, 4).unwrap();
    let (path, file) = temp_file("tp", b"on tp's pool");
    let (n, got) = block_on(ring.read(&file, Vec::with_capacity(32), 0));
    assert_eq!(&got[..n.unwrap()], b"on tp's pool");
    drop(file);
    let _ = std::fs::remove_file(&path);
}

/// The ring goes with its last owner, operations it still holds included.
#[test]
fn a_ring_with_abandoned_operations_tears_down() {
    let (path, file) = temp_file("teardown", &[2u8; 8192]);
    let ring = IoRing::new(8).unwrap();
    for i in 0..8u64 {
        drop(ring.read(&file, vec![0u8; 1024], i * 1024));
    }
    let started = Instant::now();
    drop(ring);
    assert!(started.elapsed() < Duration::from_secs(5), "teardown hung");
    drop(file);
    let _ = std::fs::remove_file(&path);
}

/// Shared std files, borrowed std files and borrowed handles, overlapped
/// or not.
#[test]
fn handles_of_every_kind() {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsHandle;
    let ring = IoRing::new(8).unwrap();
    let (path, file) = temp_file("handles", b"handles!");
    drop(file);

    let shared = Arc::new(std::fs::File::open(&path).unwrap());
    let (n, got) = block_on(ring.read(&shared, Vec::with_capacity(16), 0));
    assert_eq!(&got[..n.unwrap()], b"handles!", "Arc<std::fs::File>");

    let overlapped = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(windows::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED.0)
        .open(&path)
        .unwrap();
    let mut buf = [0u8; 16];
    // SAFETY: each future is awaited to completion.
    unsafe {
        let (n, _) = block_on(ring.read_unchecked(&&overlapped, &mut buf[..], 0));
        assert_eq!(&buf[..n.unwrap()], b"handles!", "&std::fs::File, overlapped");
        let borrowed = overlapped.as_handle();
        let (n, _) = block_on(ring.read_unchecked(&borrowed, &mut buf[..], 4));
        assert_eq!(&buf[..n.unwrap()], b"les!", "BorrowedHandle");
    }
    drop((shared, overlapped));
    let _ = std::fs::remove_file(&path);
}

/// A handle bound to a completion port is refused, not mis-delivered.
#[test]
fn a_port_bound_handle_is_refused() {
    use windows::Win32::System::IO::CreateIoCompletionPort;
    let ring = IoRing::new(4).unwrap();
    let (path, file) = temp_file("port", b"bound");
    drop(file);
    let handle = unsafe {
        CreateFileW(
            &HSTRING::from(path.as_str()),
            GENERIC_READ.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            windows::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED,
            None,
        )
        .unwrap()
    };
    let port = unsafe { CreateIoCompletionPort(handle, None, 1, 1).unwrap() };
    // SAFETY: a fresh handle this test owns and hands over.
    let bound = unsafe { File::from_raw_handle(handle) };
    let (r, _) = block_on(ring.read(&bound, vec![0u8; 8], 0));
    assert_eq!(r.unwrap_err().code(), windows::Win32::Foundation::E_INVALIDARG);
    drop(bound);
    unsafe {
        let _ = CloseHandle(port);
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn registered_files_are_read_by_index() {
    let ring = IoRing::new(8).unwrap();
    let (path, file) = temp_file("regfile", b"by index");
    let files = block_on(ring.register_files(std::slice::from_ref(&file))).unwrap();
    assert_eq!(files.len(), 1);
    let registered = files.get(0).unwrap();
    // The caller's handle can go: the ring keeps its own.
    drop(file);
    let (n, got) = block_on(ring.read(&registered, Vec::with_capacity(16), 0));
    assert_eq!(&got[..n.unwrap()], b"by index");
    assert!(files.get(1).is_none());
    drop((registered, files, ring));
    let _ = std::fs::remove_file(&path);
}

/// A slab and arbitrary owned buffers, both ways, and part of one.
#[test]
fn registered_buffers_go_both_ways() {
    let ring = IoRing::new(8).unwrap();
    let (path, file) = temp_file("regbuf", b"");
    let slab = block_on(ring.register_slab(4096, 4).unwrap()).unwrap();
    assert_eq!((slab.len(), slab.available()), (4, 4));

    let mut out = slab.take().unwrap();
    out.set_len(12);
    out.copy_from_slice(b"registered!!");
    let (n, out) = block_on(ring.write(&file, out, 0));
    assert_eq!(n.unwrap(), 12);
    drop(out);
    assert_eq!(slab.available(), 4, "a buffer did not come back");

    let (n, back) = block_on(ring.read(&file, slab.take().unwrap(), 0));
    assert_eq!(n.unwrap(), 12);
    assert_eq!(&back[..], b"registered!!");
    drop(back);

    // Part of a registered buffer is still addressed as registered.
    let (n, part) = block_on(ring.read(&file, slab.take().unwrap().slice(100..104), 2));
    assert_eq!(n.unwrap(), 4);
    let part = part.into_inner();
    assert_eq!(part.len(), 104);
    assert_eq!(&part[100..104], b"gist");

    // Any owned buffers, replacing the slab.
    let vecs = block_on(ring.register_buffers(vec![vec![0u8; 64], vec![0u8; 64]])).unwrap();
    let (n, got) = block_on(ring.read(&file, vecs.take_index(1).unwrap(), 0));
    assert_eq!(&got[..n.unwrap()], b"registered!!");
    // The slab's buffers belong to the replaced registration now.
    let (r, _) = block_on(ring.read(&file, slab.take().unwrap(), 0));
    assert_eq!(r.unwrap_err().code(), windows::Win32::Foundation::E_INVALIDARG);
    drop(file);
    let _ = std::fs::remove_file(&path);
}

/// A replaced table's indexes are refused -- including by operations
/// issued before the replacement was confirmed, which wait for it --
/// rather than reaching what the new table has at the same index.
#[test]
fn a_replaced_registration_is_refused() {
    let ring = IoRing::new(8).unwrap();
    let (pa, a) = temp_file("repl_a", b"AAAA");
    let (pb, b) = temp_file("repl_b", b"BBBB");
    let first = block_on(ring.register_files(std::slice::from_ref(&a))).unwrap();
    let old = first.get(0).unwrap();

    let replacing = ring.register_files(std::slice::from_ref(&b));
    let stale = ring.read(&old, vec![0u8; 4], 0); // built while pending
    let second = block_on(replacing).unwrap();
    let (r, _) = block_on(stale);
    assert_eq!(r.unwrap_err().code(), windows::Win32::Foundation::E_INVALIDARG);

    let (n, got) = block_on(ring.read(&second.get(0).unwrap(), vec![0u8; 4], 0));
    assert_eq!(&got[..n.unwrap()], b"BBBB");
    drop((a, b));
    let _ = std::fs::remove_file(&pa);
    let _ = std::fs::remove_file(&pb);
}

/// A registration the kernel refuses (an empty table, measured) leaves
/// none in force: the old indexes are refused, not handed a table in an
/// unknown state.
#[test]
fn a_refused_registration_leaves_none() {
    let ring = IoRing::new(8).unwrap();
    let (path, file) = temp_file("refused", b"kept");
    let files = block_on(ring.register_files(std::slice::from_ref(&file))).unwrap();
    let empty: [File; 0] = [];
    assert!(block_on(ring.register_files(&empty)).is_err());
    let (r, _) = block_on(ring.read(&files.get(0).unwrap(), vec![0u8; 8], 0));
    assert_eq!(r.unwrap_err().code(), windows::Win32::Foundation::E_INVALIDARG);
    // A fresh registration works again.
    let again = block_on(ring.register_files(std::slice::from_ref(&file))).unwrap();
    let (n, got) = block_on(ring.read(&again.get(0).unwrap(), vec![0u8; 8], 0));
    assert_eq!(&got[..n.unwrap()], b"kept");
    drop(file);
    let _ = std::fs::remove_file(&path);
}

/// One cold-pool trial: a pumped ring whose event has fired is torn down
/// before the default pool has ever dispatched, then a second pumped ring
/// must still deliver. Runs only as a child of [`cold_pool_teardown_sweep`]
/// (the hazard needs a pool with no threads yet, which only a fresh process
/// has); exits 3 if the victim never hears its completion.
#[test]
fn cold_pool_teardown_child() {
    if std::env::var_os("WINASYNC_STALL_CHILD").is_none() {
        return;
    }
    let (path, file) = temp_file("stall", b"x");
    if std::env::var_os("WINASYNC_STALL_NO_TRIGGER").is_none() {
        // The trigger: the read completes and signals the pump's event,
        // which queues its callback on the cold pool; this thread pops the
        // completion itself, spinning rather than sleeping so the pool has
        // had no time to make a thread, and then the drop closes the wait
        // microseconds behind its disarm with that callback still queued.
        let ring = IoRing::new(4).unwrap();
        let op = ring.read(&file, vec![0u8; 1], 0);
        let _ = ring.submit();
        while in_flight(&ring) > 0 {
            ring.drive();
        }
        drop(op);
        drop(ring);
    }
    // Opened without sharing, so the victim opens its own.
    drop(file);
    // The victim: delivered only by the pump.
    let (tx, rx) = std::sync::mpsc::channel();
    let path2 = path.clone();
    std::thread::spawn(move || {
        let file = File::open(&path2).unwrap();
        let ring = IoRing::new(4).unwrap();
        let (n, _) = block_on(ring.read(&file, vec![0u8; 1], 0));
        let _ = tx.send(n.unwrap());
    });
    let heard = rx.recv_timeout(Duration::from_secs(5)).is_ok();
    let _ = std::fs::remove_file(&path);
    if !heard {
        std::process::exit(3);
    }
}

/// Runs [`cold_pool_teardown_child`] in `WINASYNC_STALL_RUNS` (default 1000)
/// fresh processes and counts the ones whose pool stalled. Ignored: it takes
/// minutes, and it is a measurement of the teardown rather than a unit test.
///
/// Background: closing a `TP_WAIT` shortly after disarming it, with its
/// callback queued, on a default pool that has not made a thread yet, can
/// leave that pool unable to dispatch anything (MikeGrier/windows-threadpool-sys,
/// measurements of 2026-09-28..30: about 2.5 per 1000 cold runs with a
/// cancelling teardown, 0 in 20000 with `WaitForThreadpoolWaitCallbacks(FALSE)`
/// between the disarm and the close).
#[test]
#[ignore = "minutes-long cold-pool sweep; run by hand"]
fn cold_pool_teardown_sweep() {
    let runs: usize = std::env::var("WINASYNC_STALL_RUNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
    let exe = std::env::current_exe().unwrap();
    let started = Instant::now();
    let mut stalled = 0;
    let mut other = 0;
    for i in 0..runs {
        let status = std::process::Command::new(&exe)
            .args(["--exact", "ioring::tests::cold_pool_teardown_child", "--test-threads=1", "-q"])
            .env("WINASYNC_STALL_CHILD", "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        match status.code() {
            Some(0) => {}
            Some(3) => stalled += 1,
            _ => other += 1,
        }
        if (i + 1) % 100 == 0 {
            eprintln!("{}/{runs}: stalled={stalled} other={other} ({:?})", i + 1, started.elapsed());
        }
    }
    eprintln!("cold-pool sweep: runs={runs} stalled={stalled} other={other} in {:?}", started.elapsed());
    assert_eq!(other, 0, "children failed for another reason");
    assert_eq!(stalled, 0, "the pool stalled in {stalled} of {runs} cold runs");
}

#[test]
fn another_rings_registration_is_refused() {
    let one = IoRing::new(4).unwrap();
    let two = IoRing::new(4).unwrap();
    let (path, file) = temp_file("cross", b"x");
    let slab = block_on(one.register_slab(64, 1).unwrap()).unwrap();
    let (r, _) = block_on(two.read(&file, slab.take().unwrap(), 0));
    assert_eq!(r.unwrap_err().code(), windows::Win32::Foundation::E_INVALIDARG);
    drop(file);
    let _ = std::fs::remove_file(&path);
}
