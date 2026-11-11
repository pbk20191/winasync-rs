use super::super::{block_on, yield_now, Runtime};
use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::time::Duration;

#[test]
fn spawn_join() {
    let rt = Runtime::new();
    let h = rt.spawn(async { 6 * 7 });
    assert_eq!(block_on(h).unwrap(), 42);
}

#[test]
fn yield_loop_exercises_notified_path() {
    let rt = Runtime::new();
    let h = rt.spawn(async {
        let mut n = 0u32;
        for _ in 0..100 {
            yield_now().await;
            n += 1;
        }
        n
    });
    assert_eq!(block_on(h).unwrap(), 100);
}

#[test]
fn many_tasks() {
    let rt = Runtime::new();
    let handles: Vec<_> = (0..1000u64)
        .map(|i| {
            rt.spawn(async move {
                yield_now().await;
                i
            })
        })
        .collect();
    let sum = block_on(async {
        let mut sum = 0;
        for h in handles {
            sum += h.await.unwrap();
        }
        sum
    });
    assert_eq!(sum, 999 * 1000 / 2);
}

#[test]
fn panic_propagates() {
    let rt = Runtime::new();
    let h = rt.spawn(async {
        panic!("boom");
    });
    match block_on(h) {
        Err(JoinError::Panicked(p)) => {
            assert_eq!(p.downcast_ref::<&str>(), Some(&"boom"));
        }
        other => panic!("expected panic, got {other:?}"),
    }
}

#[test]
fn abort_parked_task_drops_future() {
    struct SetOnDrop(Arc<AtomicBool>);
    impl Drop for SetOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    struct Forever;
    impl Future for Forever {
        type Output = ();
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
            Poll::Pending // parks without ever waking
        }
    }

    let dropped = Arc::new(AtomicBool::new(false));
    let rt = Runtime::new();
    let guard = SetOnDrop(dropped.clone());
    let h = rt.spawn(async move {
        let _guard = guard;
        Forever.await;
    });
    // Let the task reach IDLE before aborting.
    std::thread::sleep(Duration::from_millis(50));
    h.abort();
    match block_on(h) {
        Err(JoinError::Aborted) => {}
        other => panic!("expected abort, got {other:?}"),
    }
    assert!(dropped.load(Ordering::SeqCst));
}

#[test]
fn detached_task_still_runs() {
    let ran = Arc::new(AtomicBool::new(false));
    let rt = Runtime::new();
    let flag = ran.clone();
    drop(rt.spawn(async move {
        flag.store(true, Ordering::SeqCst);
    }));
    while !ran.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn spawn_stress() {
    let rt = Runtime::new();
    let counter = Arc::new(AtomicUsize::new(0));
    let handles: Vec<_> = (0..10_000)
        .map(|_| {
            let c = counter.clone();
            rt.spawn(async move {
                yield_now().await;
                c.fetch_add(1, Ordering::Relaxed);
            })
        })
        .collect();
    block_on(async {
        for h in handles {
            h.await.unwrap();
        }
    });
    assert_eq!(counter.load(Ordering::Relaxed), 10_000);
}

#[test]
fn pool_scope_and_join() {
    use super::super::Pool;
    let total = AtomicUsize::new(0);
    Pool::with_scope(|scope| {
        for i in 0..100usize {
            let total = &total;
            scope.submit(move || {
                total.fetch_add(i, Ordering::Relaxed);
            });
        }
    });
    assert_eq!(total.load(Ordering::Relaxed), 99 * 100 / 2);
}

#[test]
fn simple_ctx_released_by_finalization() {
    // If the environment's FinalizationCallback did not fire for
    // TrySubmitThreadpoolCallback objects, the SimpleCtx box (and this
    // Arc clone) would leak and the strong count would stay at 2.
    let marker = Arc::new(());
    let pool = super::super::Pool::new();
    let clone = marker.clone();
    pool.submit(move || {
        let _keep = &clone;
    });
    pool.join();
    for _ in 0..100 {
        if Arc::strong_count(&marker) == 1 {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("simple submission context leaked (finalization did not run)");
}
