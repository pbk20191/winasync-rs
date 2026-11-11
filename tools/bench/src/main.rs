//! winasync against tokio and compio, on the same workloads and the same
//! machine:
//!
//! * **echo** -- `conns` TCP connections over loopback, each doing `iters`
//!   round trips of a `msg`-byte message to an echo server running on the
//!   same runtime. Reports round trips per second, round-trip latency
//!   percentiles, and heap allocations per round trip.
//! * **file** -- `reads` random 4 KiB reads of a `file_mb` MiB file at queue
//!   depth `qd`. The file is in the OS cache, so what is measured is the
//!   runtime's submission and completion path, not the disk. Reports reads
//!   per second and allocations per read.
//!
//! Allocations are counted by a wrapping global allocator over the whole
//! run, including the harness's own (one task and one result vector per
//! connection), which is why `iters` should be large.
//!
//! ```text
//! cargo run --release -- all conns=64 iters=2000 msg=64 qd=32 reads=200000
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- counting
struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

// ---------------------------------------------------------------- config
#[derive(Clone, Copy)]
struct Config {
    conns: usize,
    iters: usize,
    msg: usize,
    qd: usize,
    reads: usize,
    file_mb: u64,
    /// winasync only: `set_skip_completion_on_success` on every handle.
    skip: bool,
}

const READ: usize = 4096;

fn parse(args: &[String]) -> (String, Config) {
    let mut cfg = Config {
        conns: 64,
        iters: 2000,
        msg: 64,
        qd: 32,
        reads: 200_000,
        file_mb: 256,
        skip: false,
    };
    let mut which = String::from("all");
    for a in args {
        match a.split_once('=') {
            Some(("conns", v)) => cfg.conns = v.parse().unwrap(),
            Some(("iters", v)) => cfg.iters = v.parse().unwrap(),
            Some(("msg", v)) => cfg.msg = v.parse().unwrap(),
            Some(("qd", v)) => cfg.qd = v.parse().unwrap(),
            Some(("reads", v)) => cfg.reads = v.parse().unwrap(),
            Some(("file_mb", v)) => cfg.file_mb = v.parse().unwrap(),
            Some(("skip", v)) => cfg.skip = v == "1",
            Some((k, _)) => panic!("unknown option {k}"),
            None => which = a.clone(),
        }
    }
    (which, cfg)
}

// ---------------------------------------------------------------- results
struct Stats {
    elapsed: Duration,
    ops: usize,
    allocs: u64,
    lat: Vec<u64>,
}

fn measure(f: impl FnOnce() -> (usize, Vec<u64>)) -> Stats {
    let a0 = ALLOCS.load(Relaxed);
    let t0 = Instant::now();
    let (ops, mut lat) = f();
    let elapsed = t0.elapsed();
    let allocs = ALLOCS.load(Relaxed) - a0;
    lat.sort_unstable();
    Stats { elapsed, ops, allocs, lat }
}

fn pct(lat: &[u64], p: f64) -> f64 {
    if lat.is_empty() {
        return 0.0;
    }
    let i = ((lat.len() - 1) as f64 * p).round() as usize;
    lat[i] as f64 / 1000.0
}

fn row(name: &str, s: &Stats) {
    let per_s = s.ops as f64 / s.elapsed.as_secs_f64();
    if s.lat.is_empty() {
        println!(
            "{name:<26}{per_s:>12.0}{:>10.1}{:>10.1}{:>12.2}",
            s.elapsed.as_secs_f64() * 1000.0,
            0.0,
            s.allocs as f64 / s.ops as f64
        );
    } else {
        println!(
            "{name:<26}{per_s:>12.0}{:>10.1}{:>10.1}{:>12.2}",
            pct(&s.lat, 0.50),
            pct(&s.lat, 0.99),
            s.allocs as f64 / s.ops as f64
        );
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn offset(rng: &mut Rng, file_len: u64) -> u64 {
    (rng.next() % (file_len / READ as u64)) * READ as u64
}

// ---------------------------------------------------------------- echo: winasync tp
fn tp_echo(cfg: Config) -> (usize, Vec<u64>) {
    use winasync::tp::{Runtime, SocketOptions, TcpListener, TcpStream};
    let rt = Runtime::new();
    let listener = TcpListener::bind(&rt, "127.0.0.1:0".parse().unwrap(), &SocketOptions::default()).unwrap();
    let addr = listener.local_addr().unwrap();
    let Config { conns, iters, msg, skip, .. } = cfg;
    let rt2 = rt.clone();
    rt.block_on(async move {
        let rt3 = rt2.clone();
        let server = rt2.spawn(async move {
            let mut tasks = Vec::with_capacity(conns);
            for _ in 0..conns {
                let s = listener.accept().await.unwrap();
                if skip {
                    s.set_skip_completion_on_success().unwrap();
                }
                tasks.push(rt3.spawn(async move {
                    let mut buf = vec![0u8; msg];
                    loop {
                        let (n, b) = s.read(buf).await;
                        let n = n.unwrap();
                        if n == 0 {
                            break;
                        }
                        let mut b = b;
                        b.truncate(n);
                        let (w, mut b) = s.write(b).await;
                        assert_eq!(w.unwrap(), n);
                        b.resize(msg, 0);
                        buf = b;
                    }
                }));
            }
            for t in tasks {
                t.await.unwrap();
            }
        });
        let mut clients = Vec::with_capacity(conns);
        for _ in 0..conns {
            let rt4 = rt2.clone();
            clients.push(rt2.spawn(async move {
                let s = TcpStream::connect(&rt4, addr, &SocketOptions::default()).await.unwrap();
                if skip {
                    s.set_skip_completion_on_success().unwrap();
                }
                let mut out = vec![0xABu8; msg];
                let mut inb = vec![0u8; msg];
                let mut lat = Vec::with_capacity(iters);
                for _ in 0..iters {
                    let t = Instant::now();
                    let (w, o) = s.write(out).await;
                    assert_eq!(w.unwrap(), msg);
                    out = o;
                    let mut got = 0;
                    while got < msg {
                        let (n, b) = s.read(inb).await;
                        got += n.unwrap();
                        inb = b;
                    }
                    lat.push(t.elapsed().as_nanos() as u64);
                }
                drop(s);
                lat
            }));
        }
        let mut lat = Vec::with_capacity(conns * iters);
        for c in clients {
            lat.extend(c.await.unwrap());
        }
        server.await.unwrap();
        (conns * iters, lat)
    })
}

// ---------------------------------------------------------------- echo: tokio
fn tokio_echo(cfg: Config) -> (usize, Vec<u64>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rt = tokio::runtime::Runtime::new().unwrap();
    let Config { conns, iters, msg, .. } = cfg;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut tasks = Vec::with_capacity(conns);
            for _ in 0..conns {
                let (mut s, _) = listener.accept().await.unwrap();
                tasks.push(tokio::spawn(async move {
                    let mut buf = vec![0u8; msg];
                    loop {
                        let n = s.read(&mut buf).await.unwrap();
                        if n == 0 {
                            break;
                        }
                        s.write_all(&buf[..n]).await.unwrap();
                    }
                }));
            }
            for t in tasks {
                t.await.unwrap();
            }
        });
        let mut clients = Vec::with_capacity(conns);
        for _ in 0..conns {
            clients.push(tokio::spawn(async move {
                let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
                let out = vec![0xABu8; msg];
                let mut inb = vec![0u8; msg];
                let mut lat = Vec::with_capacity(iters);
                for _ in 0..iters {
                    let t = Instant::now();
                    s.write_all(&out).await.unwrap();
                    s.read_exact(&mut inb).await.unwrap();
                    lat.push(t.elapsed().as_nanos() as u64);
                }
                drop(s);
                lat
            }));
        }
        let mut lat = Vec::with_capacity(conns * iters);
        for c in clients {
            lat.extend(c.await.unwrap());
        }
        server.await.unwrap();
        (conns * iters, lat)
    })
}

// ---------------------------------------------------------------- echo: compio
fn compio_echo(cfg: Config) -> (usize, Vec<u64>) {
    use compio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
    let rt = compio::runtime::Runtime::new().unwrap();
    let Config { conns, iters, msg, .. } = cfg;
    rt.block_on(async move {
        let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let server = compio::runtime::spawn(async move {
            let mut tasks = Vec::with_capacity(conns);
            for _ in 0..conns {
                let (mut s, _) = listener.accept().await.unwrap();
                tasks.push(compio::runtime::spawn(async move {
                    // compio reads into a Vec's spare capacity.
                    let mut buf: Vec<u8> = Vec::with_capacity(msg);
                    loop {
                        buf.clear();
                        let (n, b) = s.read(buf).await.into();
                        let n: usize = n.unwrap();
                        if n == 0 {
                            break;
                        }
                        let (r, b) = s.write_all(b).await.into();
                        r.unwrap();
                        buf = b;
                    }
                }));
            }
            for t in tasks {
                t.await.unwrap();
            }
        });
        let mut clients = Vec::with_capacity(conns);
        for _ in 0..conns {
            clients.push(compio::runtime::spawn(async move {
                let mut s = compio::net::TcpStream::connect(addr).await.unwrap();
                let mut out = vec![0xABu8; msg];
                let mut inb: Vec<u8> = Vec::with_capacity(msg);
                let mut lat = Vec::with_capacity(iters);
                for _ in 0..iters {
                    let t = Instant::now();
                    let (r, o) = s.write_all(out).await.into();
                    r.unwrap();
                    out = o;
                    inb.clear();
                    let (r, b) = s.read_exact(inb).await.into();
                    r.unwrap();
                    inb = b;
                    lat.push(t.elapsed().as_nanos() as u64);
                }
                drop(s);
                lat
            }));
        }
        let mut lat = Vec::with_capacity(conns * iters);
        for c in clients {
            lat.extend(c.await.unwrap());
        }
        server.await.unwrap();
        (conns * iters, lat)
    })
}

// ---------------------------------------------------------------- file: winasync tp
fn tp_file(cfg: Config, path: &str) -> (usize, Vec<u64>) {
    use winasync::tp::{File, Runtime};
    let rt = Runtime::new();
    let file = Arc::new(File::open(&rt, path).unwrap());
    if cfg.skip {
        file.set_skip_completion_on_success().unwrap();
    }
    let Config { qd, reads, file_mb, .. } = cfg;
    let per = reads / qd;
    let file_len = file_mb << 20;
    let rt2 = rt.clone();
    rt.block_on(async move {
        let mut tasks = Vec::with_capacity(qd);
        for i in 0..qd {
            let file = file.clone();
            tasks.push(rt2.spawn(async move {
                let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (i as u64 + 1));
                let mut buf = vec![0u8; READ];
                for _ in 0..per {
                    let (n, b) = file.read_at(buf, offset(&mut rng, file_len)).await;
                    assert_eq!(n.unwrap(), READ);
                    buf = b;
                }
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        (per * qd, Vec::new())
    })
}

// ---------------------------------------------------------------- file: winasync ioring
fn ioring_file(cfg: Config, path: &str) -> (usize, Vec<u64>) {
    use winasync::ioring::{File, IoRing};
    use winasync::tp::Runtime;
    let rt = Runtime::new();
    let Config { qd, reads, file_mb, .. } = cfg;
    let ring = IoRing::with_runtime(&rt, qd as u32).unwrap();
    let file = File::open(path).unwrap();
    let per = reads / qd;
    let file_len = file_mb << 20;
    let rt2 = rt.clone();
    rt.block_on(async move {
        let mut tasks = Vec::with_capacity(qd);
        for i in 0..qd {
            let ring = ring.clone();
            let file = file.clone();
            tasks.push(rt2.spawn(async move {
                let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (i as u64 + 1));
                let mut buf = vec![0u8; READ];
                for _ in 0..per {
                    let (n, b) = ring.read(&file, buf, offset(&mut rng, file_len)).await;
                    assert_eq!(n.unwrap(), READ);
                    buf = b;
                }
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        (per * qd, Vec::new())
    })
}

// ---------------------------------------------------------------- file: winasync ioring, batched
/// One task, `qd` reads per `Batch`: one `SubmitIoRing` per `qd` reads.
fn ioring_batch_file(cfg: Config, path: &str) -> (usize, Vec<u64>) {
    use winasync::ioring::{File, IoRing};
    use winasync::tp::Runtime;
    let rt = Runtime::new();
    let Config { qd, reads, file_mb, .. } = cfg;
    let ring = IoRing::with_runtime(&rt, qd as u32).unwrap();
    let file = File::open(path).unwrap();
    let rounds = reads / qd;
    let file_len = file_mb << 20;
    rt.block_on(async move {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut bufs: Vec<Vec<u8>> = (0..qd).map(|_| vec![0u8; READ]).collect();
        for _ in 0..rounds {
            let batch = ring.read_batch(&file, bufs.drain(..).map(|b| (b, offset(&mut rng, file_len))));
            for (n, b) in batch.await {
                assert_eq!(n.unwrap(), READ);
                bufs.push(b);
            }
        }
        (rounds * qd, Vec::new())
    })
}

// ---------------------------------------------------------------- file: winasync ioring, batched + registered
/// `ioring_batch_file` with the file and the buffers registered: the kernel
/// skips the handle lookup and the page probing per operation.
fn ioring_reg_file(cfg: Config, path: &str) -> (usize, Vec<u64>) {
    use winasync::ioring::{File, IoRing};
    use winasync::tp::Runtime;
    let rt = Runtime::new();
    let Config { qd, reads, file_mb, .. } = cfg;
    let ring = IoRing::with_runtime(&rt, qd as u32).unwrap();
    let file = File::open(path).unwrap();
    let rounds = reads / qd;
    let file_len = file_mb << 20;
    rt.block_on(async move {
        let files = ring.register_files(&[file]).await.unwrap();
        let file = files.get(0).unwrap();
        let slab = ring.register_slab(READ as u32, qd as u32).unwrap().await.unwrap();
        let mut bufs: Vec<_> = (0..qd).map(|_| slab.take().unwrap()).collect();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..rounds {
            let batch = ring.read_batch(&file, bufs.drain(..).map(|b| (b, offset(&mut rng, file_len))));
            for (n, b) in batch.await {
                assert_eq!(n.unwrap(), READ);
                bufs.push(b);
            }
        }
        (rounds * qd, Vec::new())
    })
}

// ---------------------------------------------------------------- file: winasync ioring, no pump
/// The ring's ceiling: no pump, no task -- one thread submits a batch and
/// spins on `drive` until it is done. What the kernel path costs by itself.
fn ioring_manual_file(cfg: Config, path: &str) -> (usize, Vec<u64>) {
    use std::task::{Context, Poll, Waker};
    use winasync::ioring::{File, IoRing};
    let Config { qd, reads, file_mb, .. } = cfg;
    let ring = IoRing::manual(qd as u32).unwrap();
    let file = File::open(path).unwrap();
    let rounds = reads / qd;
    let file_len = file_mb << 20;
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut bufs: Vec<Vec<u8>> = (0..qd).map(|_| vec![0u8; READ]).collect();
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    for _ in 0..rounds {
        let mut batch = ring.read_batch(&file, bufs.drain(..).map(|b| (b, offset(&mut rng, file_len))));
        let results = loop {
            if let Poll::Ready(r) = std::pin::Pin::new(&mut batch).poll(&mut cx) {
                break r;
            }
            ring.drive();
        };
        for (n, b) in results {
            assert_eq!(n.unwrap(), READ);
            bufs.push(b);
        }
    }
    (rounds * qd, Vec::new())
}

// ---------------------------------------------------------------- file: tokio
fn tokio_file(cfg: Config, path: &str) -> (usize, Vec<u64>) {
    // `tokio::fs` is `spawn_blocking` over std; positional reads go the same
    // way, with `seek_read` so tasks can share one handle.
    use std::os::windows::fs::FileExt;
    let rt = tokio::runtime::Runtime::new().unwrap();
    let file = Arc::new(std::fs::File::open(path).unwrap());
    let Config { qd, reads, file_mb, .. } = cfg;
    let per = reads / qd;
    let file_len = file_mb << 20;
    rt.block_on(async move {
        let mut tasks = Vec::with_capacity(qd);
        for i in 0..qd {
            let file = file.clone();
            tasks.push(tokio::spawn(async move {
                let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (i as u64 + 1));
                let mut buf = vec![0u8; READ];
                for _ in 0..per {
                    let off = offset(&mut rng, file_len);
                    let file = file.clone();
                    let (n, b) = tokio::task::spawn_blocking(move || {
                        let n = file.seek_read(&mut buf, off);
                        (n, buf)
                    })
                    .await
                    .unwrap();
                    assert_eq!(n.unwrap(), READ);
                    buf = b;
                }
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        (per * qd, Vec::new())
    })
}

// ---------------------------------------------------------------- file: compio
fn compio_file(cfg: Config, path: &str) -> (usize, Vec<u64>) {
    use compio::io::AsyncReadAtExt;
    let rt = compio::runtime::Runtime::new().unwrap();
    let Config { qd, reads, file_mb, .. } = cfg;
    let per = reads / qd;
    let file_len = file_mb << 20;
    let path = path.to_string();
    rt.block_on(async move {
        let file = std::rc::Rc::new(compio::fs::File::open(&path).await.unwrap());
        let mut tasks = Vec::with_capacity(qd);
        for i in 0..qd {
            let file = file.clone();
            tasks.push(compio::runtime::spawn(async move {
                let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (i as u64 + 1));
                let mut buf: Vec<u8> = Vec::with_capacity(READ);
                for _ in 0..per {
                    buf.clear();
                    let (r, b) = file.read_exact_at(buf, offset(&mut rng, file_len)).await.into();
                    r.unwrap();
                    assert_eq!(b.len(), READ);
                    buf = b;
                }
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        (per * qd, Vec::new())
    })
}

// ---------------------------------------------------------------- main
fn make_file(cfg: Config) -> String {
    let path = std::env::temp_dir().join("winasync-bench.bin");
    let want = cfg.file_mb << 20;
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) != want {
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(want).unwrap();
        // Touch every page so the file is really allocated and cached.
        use std::os::windows::fs::FileExt;
        let page = vec![0x5Au8; 1 << 20];
        for i in 0..cfg.file_mb {
            f.seek_write(&page, i << 20).unwrap();
        }
    }
    path.to_string_lossy().into_owned()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (which, cfg) = parse(&args);
    println!("cpus={} ", std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0));
    if which == "all" || which == "echo" {
        println!(
            "\nTCP echo: conns={} iters={} msg={}B  (round trips)",
            cfg.conns, cfg.iters, cfg.msg
        );
        println!("{:<26}{:>12}{:>10}{:>10}{:>12}", "runtime", "rt/s", "p50 us", "p99 us", "allocs/rt");
        for (name, f) in [
            ("winasync tp", tp_echo as fn(Config) -> (usize, Vec<u64>)),
            ("winasync tp+skip", |c| tp_echo(Config { skip: true, ..c })),
            ("tokio", tokio_echo),
            ("compio", compio_echo),
        ] {
            // Warm-up at a tenth of the size, then the measured run.
            let _ = f(Config { iters: (cfg.iters / 10).max(1), ..cfg });
            row(name, &measure(|| f(cfg)));
        }
    }
    if which == "all" || which == "file" {
        let path = make_file(cfg);
        println!(
            "\nrandom 4 KiB reads: file={}MiB qd={} reads={}  (cached)",
            cfg.file_mb, cfg.qd, cfg.reads
        );
        println!("{:<26}{:>12}{:>10}{:>10}{:>12}", "runtime", "reads/s", "ms", "", "allocs/read");
        for (name, f) in [
            ("winasync tp", tp_file as fn(Config, &str) -> (usize, Vec<u64>)),
            ("winasync tp+skip", |c, p| tp_file(Config { skip: true, ..c }, p)),
            ("winasync ioring", ioring_file),
            ("winasync ioring batch", ioring_batch_file),
            ("winasync ioring batch+reg", ioring_reg_file),
            ("winasync ioring manual", ioring_manual_file),
            ("tokio", tokio_file),
            ("compio", compio_file),
        ] {
            let _ = f(Config { reads: (cfg.reads / 10).max(cfg.qd), ..cfg }, &path);
            row(name, &measure(|| f(cfg, &path)));
        }
    }
}
