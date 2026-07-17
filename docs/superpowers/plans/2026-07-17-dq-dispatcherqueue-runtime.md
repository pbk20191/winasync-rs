# dq: DispatcherQueue single-threaded async runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `src/dq/` — a single-threaded async runtime whose executor is a WinRT `DispatcherQueue`: `run(root)` pumps the message loop, `spawn_local` returns an awaitable `LocalJoinHandle`, timers use `DispatcherQueueTimer`, and the waker marshals cross-thread wakes (from IO-completion pool threads) back onto the DQ thread via `TryEnqueue`.

**Architecture:** `!Send` tasks are owned locally on the DQ thread. The `Waker` holds only `Send` data (an agile `DispatcherQueue` + a `Send` ready-queue + a task id); `wake()` pushes the id and `TryEnqueue`s a drain handler that runs on the DQ thread and polls ready tasks via a thread-local runtime handle. This bridges Send wakers (called from `windows-future` completion handlers on arbitrary threads) to `!Send` local futures. Consumes `comio` transports and any WinRT async API uniformly.

**Tech Stack:** Rust (edition 2024), `winio-winui3` 0.4.0 (`Microsoft.UI.Dispatching`), `windows`/`windows-core`, `windows-future`, existing `src/comio/` and `src/tp/`.

## Global Constraints

- Platform: Windows only. Requires the Windows App SDK bootstrap already present in `main.rs` (`PackageDependency::initialize_version`) before creating a DispatcherQueue.
- Waker boundary: the waker and everything it captures must be `Send + Sync`; the actual future poll happens ONLY on the DQ thread. Never touch `!Send` task state off the DQ thread.
- No panic across a COM/FFI callback boundary; task polls use `catch_unwind`.
- `DispatcherQueueHandler::new(F)` requires `F: Fn() -> Result<()> + Send + 'static` (verified winio-winui3 mod.rs:434) — the enqueued closure must be `Send` and capture no `!Send` data.
- Commit after every task. Never use `--no-verify`.

## Reference facts (verified in crate sources)

- `winio_winui3::Microsoft::UI::Dispatching`: `DispatcherQueue::TryEnqueue<P0: Param<DispatcherQueueHandler>>(&self, cb) -> Result<bool>`; `DispatcherQueue::CreateTimer() -> Result<DispatcherQueueTimer>`; `DispatcherQueueController::CreateOnDedicatedThread() -> Result<..>`; `DispatcherQueue::RunEventLoop()`, `EnqueueEventLoopExit()`; existing `main.rs` uses `DispatcherQueueController::CreateOnCurrentThread()` + `DispatcherQueue()` + `EnsureSystemDispatcherQueue()` + `RunEventLoop()`.
- `DispatcherQueueTimer`: `SetInterval(TimeSpan)`, `SetIsRepeating(bool)`, `Start()`, `Stop()`, `Tick(handler) -> Result<i64>`.
- `windows-future` `AsyncFuture` wakes via a standard `Waker` from the completing thread (async.rs:74).
- `windows::Foundation::TimeSpan { Duration: i64 }` in 100ns units.

## File structure

```
src/dq/
  mod.rs      DispatcherRuntime, run(), run_dedicated(), spawn_local, thread-local current runtime
  waker.rs    RawWaker over (agile DispatcherQueue + Arc<ReadyQueue> + task id); wake = enqueue drain
  task.rs     LocalTask (Rc<RefCell>, scheduled flag), LocalJoinHandle, JoinError
  timer.rs    sleep(dur)/interval(period) via DispatcherQueueTimer
```

---

### Task 1: `dq` skeleton + ready-queue + thread-local runtime

**Files:**
- Create: `src/dq/mod.rs`
- Modify: `src/main.rs` (add `mod dq;`)

**Interfaces:**
- Produces:
  - `pub struct DispatcherRuntime { inner: Rc<RuntimeInner> }` (constructed later).
  - `pub(crate) struct RuntimeInner { queue: DispatcherQueue, ready: Arc<ReadyQueue>, tasks: RefCell<Slab<Rc<dyn TaskObj>>> }` (types filled in Task 3; stub now).
  - `pub(crate) struct ReadyQueue(Mutex<VecDeque<u64>>)` with `push(id)`, `drain_into(&mut Vec<u64>)`.
  - thread-local `CURRENT: RefCell<Option<Weak<RuntimeInner>>>` + `pub(crate) fn with_current<R>(f: impl FnOnce(&Rc<RuntimeInner>) -> R) -> R`.

- [ ] **Step 1: Create `src/dq/mod.rs` skeleton**

```rust
//! Single-threaded async runtime whose executor is a WinRT DispatcherQueue.
//! `!Send` futures run on the DQ thread; a Send waker marshals cross-thread
//! wakes back via TryEnqueue. See docs/superpowers/specs for the design.
#![allow(dead_code)]

pub mod task;
pub mod timer;
pub mod waker;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Send + Sync queue of task ids ready to be polled, drained on the DQ thread.
pub(crate) struct ReadyQueue(Mutex<VecDeque<u64>>);

impl ReadyQueue {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(VecDeque::new())))
    }
    pub(crate) fn push(&self, id: u64) {
        self.0.lock().unwrap().push_back(id);
    }
    pub(crate) fn drain_into(&self, out: &mut Vec<u64>) {
        let mut q = self.0.lock().unwrap();
        out.extend(q.drain(..));
    }
}
```

- [ ] **Step 2: Add a smoke test for ReadyQueue**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ready_queue_fifo() {
        let q = ReadyQueue::new();
        q.push(1); q.push(2);
        let mut out = Vec::new();
        q.drain_into(&mut out);
        assert_eq!(out, vec![1, 2]);
    }
}
```

- [ ] **Step 3: Wire module + verify**

Add `mod dq;` to `src/main.rs`. Create empty `src/dq/task.rs`, `src/dq/timer.rs`, `src/dq/waker.rs` with a `//!` line each so the `pub mod` compiles.

Run: `cargo test dq::tests::ready_queue_fifo`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/dq/ src/main.rs
git commit -m "feat(dq): module skeleton + ready-queue"
```

---

### Task 2: `waker.rs` — Send waker that enqueues a DQ drain

**Files:**
- Modify: `src/dq/waker.rs`
- Modify: `src/dq/mod.rs` (add `with_current`, thread-local, drain entry point)

**Interfaces:**
- Consumes: `ReadyQueue`, `winio_winui3::Microsoft::UI::Dispatching::{DispatcherQueue, DispatcherQueueHandler}`.
- Produces:
  - `pub(crate) struct WakerData { queue: DispatcherQueue, ready: Arc<ReadyQueue>, id: u64 }` (agile — `DispatcherQueue` is enqueue-safe from any thread).
  - `pub(crate) fn make_waker(data: Arc<WakerData>) -> std::task::Waker`.
  - The waker's `wake` pushes `id` to `ready` then `queue.TryEnqueue(drain_handler)` where `drain_handler` calls `crate::dq::drain_ready()`.
  - `pub(crate) fn drain_ready()` in `mod.rs`: on the DQ thread, drain ready ids and poll each task (`TaskObj::poll_task`).

**Design notes:**
- `WakerData` must be `Send + Sync`. `DispatcherQueue` COM interface is agile for enqueue; assert with `unsafe impl Send/Sync for WakerData {}` (documented: only `TryEnqueue` is called cross-thread).
- `RawWakerVTable` over `Arc<WakerData>`: clone = `Arc::increment_strong_count`; wake/wake_by_ref = push id + enqueue drain; drop = `Arc::from_raw`.
- The enqueued `DispatcherQueueHandler::new(move || { crate::dq::drain_ready(); Ok(()) })` captures nothing `!Send` (it captures nothing at all — it reaches the runtime via the thread-local).

- [ ] **Step 1: Write the failing test** (waker wakes a task id from another thread → drain sees it)

Because `drain_ready` needs a live runtime + DQ thread, test the *plumbing* in isolation instead: pushing via a waker enqueues onto the queue. Use a real dedicated-thread Dispatcher_queue is heavy; instead unit-test `WakerData`→`ready` push:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dq::ReadyQueue;
    // A waker built over a ReadyQueue pushes the id on wake (queue enqueue is
    // covered by integration tests that own a DispatcherQueue).
    #[test]
    fn wake_pushes_id() {
        let ready = ReadyQueue::new();
        // Construct WakerData with a null-ish queue is impossible (needs a real
        // DispatcherQueue). So this test only asserts ReadyQueue push path used
        // by wake via a direct call:
        ready.push(42);
        let mut out = Vec::new();
        ready.drain_into(&mut out);
        assert_eq!(out, vec![42]);
    }
}
```

(The real cross-thread wake is exercised in Task 4's `run` integration test; DispatcherQueue can't be cheaply mocked.)

- [ ] **Step 2: Run to verify current state**

Run: `cargo test dq::waker::tests::wake_pushes_id`
Expected: PASS once `waker.rs` compiles (it will, since the test avoids constructing `WakerData`).

- [ ] **Step 3: Implement `waker.rs`**

```rust
use crate::dq::ReadyQueue;
use std::sync::Arc;
use std::task::{RawWaker, RawWakerVTable, Waker};
use winio_winui3::Microsoft::UI::Dispatching::{DispatcherQueue, DispatcherQueueHandler};

pub(crate) struct WakerData {
    pub(crate) queue: DispatcherQueue,
    pub(crate) ready: Arc<ReadyQueue>,
    pub(crate) id: u64,
}
unsafe impl Send for WakerData {}
unsafe impl Sync for WakerData {}

impl WakerData {
    fn schedule(&self) {
        self.ready.push(self.id);
        let _ = self.queue.TryEnqueue(&DispatcherQueueHandler::new(|| {
            crate::dq::drain_ready();
            Ok(())
        }));
    }
}

static VTABLE: RawWakerVTable = RawWakerVTable::new(
    |p| { unsafe { Arc::increment_strong_count(p as *const WakerData) }; RawWaker::new(p, &VTABLE) },
    |p| { let d = unsafe { Arc::from_raw(p as *const WakerData) }; d.schedule(); },
    |p| unsafe { (*(p as *const WakerData)).schedule() },
    |p| unsafe { drop(Arc::from_raw(p as *const WakerData)) },
);

pub(crate) fn make_waker(data: Arc<WakerData>) -> Waker {
    unsafe { Waker::from_raw(RawWaker::new(Arc::into_raw(data) as *const (), &VTABLE)) }
}
```

Add to `src/dq/mod.rs` the thread-local + `drain_ready` (stub polling until Task 3 defines `TaskObj`):

```rust
use std::cell::RefCell;
use std::rc::{Rc, Weak};

thread_local! {
    pub(crate) static CURRENT: RefCell<Option<Weak<RuntimeInner>>> = const { RefCell::new(None) };
}

pub(crate) fn with_current<R>(f: impl FnOnce(&Rc<RuntimeInner>) -> R) -> R {
    CURRENT.with(|c| {
        let weak = c.borrow().clone().expect("no dq runtime on this thread");
        let rc = weak.upgrade().expect("dq runtime dropped");
        f(&rc)
    })
}

pub(crate) fn drain_ready() {
    // Defined fully in Task 3 once RuntimeInner/TaskObj exist.
    with_current(|rt| rt.drain());
}
```

`RuntimeInner` is stubbed in Task 3; to compile now, add a minimal `pub(crate) struct RuntimeInner;` with `fn drain(&self) {}` placeholder and remove it in Task 3. (Cleaner: implement Task 2 and Task 3 back-to-back; if so, skip the stub and define `RuntimeInner` in Task 3 before running Task 2's compile.)

- [ ] **Step 4: Run to verify it compiles + passes**

Run: `cargo test dq::waker::tests::wake_pushes_id`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/dq/waker.rs src/dq/mod.rs
git commit -m "feat(dq): Send waker that enqueues a DQ drain via TryEnqueue"
```

---

### Task 3: `task.rs` — LocalTask, LocalJoinHandle, RuntimeInner + drain

**Files:**
- Modify: `src/dq/task.rs`, `src/dq/mod.rs`

**Interfaces:**
- Consumes: `waker::{WakerData, make_waker}`, `ReadyQueue`, `DispatcherQueue`.
- Produces:
  - `pub(crate) trait TaskObj { fn poll_task(&self, waker: &Waker) -> bool /*done*/; }`
  - `pub(crate) struct RuntimeInner { pub(crate) queue: DispatcherQueue, pub(crate) ready: Arc<ReadyQueue>, tasks: RefCell<Slab<Rc<dyn TaskObj>>> }` with `fn drain(&self)`, `fn spawn(self: &Rc<Self>, fut) -> (u64, LocalJoinHandle<T>)`.
  - `pub struct LocalJoinHandle<T>` : `Future<Output = Result<T, JoinError>>`, `fn abort(&self)`, `fn is_finished(&self)`.
  - `pub enum JoinError { Panicked(Box<dyn Any + Send>), Aborted }`.

**Design notes (single-threaded — no atomics):**
- `LocalTask<F>` holds `RefCell<Stage<F>>` (`Running(F)` / `Finished(thread::Result<F::Output>)` / `Aborted` / `Consumed`), a `Cell<bool> scheduled`, an `Rc<RuntimeInner>` (weak to avoid cycle: store `Weak<RuntimeInner>`), and a shared join slot `Rc<RefCell<JoinShared<T>>>` (result + waker).
- `drain(&self)`: pop ready ids; for each, build a `Waker` (from a fresh `Arc<WakerData>` with that id, `queue.clone()`, `ready.clone()`), call `task.poll_task(&waker)`; if it returns done, remove from slab and wake the join waker. Guard reentrancy: dedupe ids already polled this drain.
- `poll_task`: `catch_unwind(AssertUnwindSafe(|| Pin::new_unchecked(fut).poll(cx)))`; on `Ready`/panic store into stage + join slot and return true; on `Pending` return false.
- `LocalJoinHandle::poll`: read join slot; if result present return it; else store the outer waker. `abort()`: mark the task's stage `Aborted` and schedule it (so it is removed on next drain) — or immediately remove if not currently borrowed. Keep simple: set an `abort` flag the task checks; on next poll it returns `Aborted`.
- `Slab`: use a simple `HashMap<u64, Rc<dyn TaskObj>>` + a `Cell<u64>` id counter (no external crate).

- [ ] **Step 1: Write the failing test** — but LocalJoinHandle needs a running loop, so defer behavioral tests to Task 4. Here, add a compile-level unit test that a `RuntimeInner` can be constructed and `spawn` returns a handle without running:

Since constructing `RuntimeInner` needs a `DispatcherQueue` (heavy), gate the real test behind Task 4. For Task 3, write a pure unit test of `JoinShared` result plumbing:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn join_shared_delivers_value() {
        let shared = std::rc::Rc::new(std::cell::RefCell::new(JoinShared::<i32>::new()));
        shared.borrow_mut().set(Ok(5));
        assert!(matches!(shared.borrow_mut().take(), Some(Ok(5))));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test dq::task::tests::join_shared_delivers_value`
Expected: FAIL — `JoinShared` not defined.

- [ ] **Step 3: Implement `task.rs` + finish `RuntimeInner`/`drain` in `mod.rs`**

Implement per the design notes: `JoinShared<T>`, `LocalTask<F>`, `TaskObj`, `LocalJoinHandle<T>`, `JoinError`, and `RuntimeInner { queue, ready, tasks: RefCell<HashMap<u64, Rc<dyn TaskObj>>>, next_id: Cell<u64> }` with `drain` and `spawn`. Replace the Task 2 `RuntimeInner` stub. Wire `drain_ready` → `with_current(|rt| rt.drain())`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test dq::task::tests::join_shared_delivers_value`
Expected: PASS. `cargo check` clean.

- [ ] **Step 5: Commit**

```bash
git add src/dq/task.rs src/dq/mod.rs
git commit -m "feat(dq): LocalTask, LocalJoinHandle, RuntimeInner drain loop"
```

---

### Task 4: `run()` / `run_dedicated()` + `spawn_local` + integration test

**Files:**
- Modify: `src/dq/mod.rs`

**Interfaces:**
- Produces:
  - `impl DispatcherRuntime { pub fn run<F: Future>(root: F) -> F::Output; pub fn run_dedicated<F: Future + 'static, C: FnOnce() -> F + Send + 'static>(make: C) -> ...; pub fn spawn_local<F: Future + 'static>(fut: F) -> LocalJoinHandle<F::Output> where F::Output: 'static; }`
  - `spawn_local` is a free function using `with_current`.

**Design notes:**
- `run(root)`: create a DispatcherQueue on the current thread — mirror `main.rs`: `DispatcherQueueController::CreateOnCurrentThread()`, `.DispatcherQueue()`, `.EnsureSystemDispatcherQueue()`. Build `RuntimeInner`, install into `CURRENT` (as `Weak`), `spawn` the root future, capture its `LocalJoinHandle` in an `Rc<RefCell<Option<Output>>>`. Wrap root so on completion it stores the output and calls `queue.EnqueueEventLoopExit()`. `queue.RunEventLoop()`. After the loop, uninstall `CURRENT`, return the stored output. Shutdown the controller (`ShutdownQueue`) as `main.rs` does.
- The initial `spawn` must also enqueue a first drain so the root polls once (push id + TryEnqueue drain, or call `drain()` right after RunEventLoop starts — simplest: after spawn, `queue.TryEnqueue(drain)`).
- `run_dedicated(make)`: `CreateOnDedicatedThread()`, then `TryEnqueue` a Send closure that (on the DQ thread) builds `RuntimeInner`, installs CURRENT, spawns `make()`, and the loop runs on that thread. Returns a handle to await completion from the caller thread via a `Send` oneshot (e.g. `std::sync::mpsc`). Document that the output type must be `Send` to cross back.
- `spawn_local(fut)`: `with_current(|rt| rt.spawn(fut)).1` and push its id + enqueue a drain.

- [ ] **Step 1: Write the failing integration test**

```rust
#[cfg(test)]
mod run_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn run_returns_value() {
        let v = DispatcherRuntime::run(async { 21 * 2 });
        assert_eq!(v, 42);
    }

    #[test]
    fn spawn_local_join() {
        let v = DispatcherRuntime::run(async {
            let h = spawn_local(async { 7 });
            h.await.unwrap()
        });
        assert_eq!(v, 7);
    }

    #[test]
    fn cross_thread_wake_resumes_on_dq_thread() {
        // Await a comio-style op whose completion wakes from a pool thread; the
        // continuation must run on the DQ thread. Use tp::sleep spawned on tp,
        // signaling via a WinRT IAsyncOperation::spawn (completes on pool).
        let same = DispatcherRuntime::run(async {
            let dq_thread = std::thread::current().id();
            // windows_future op completes on a pool thread, wakes our waker.
            let _ = windows_future::IAsyncOperation::<i32>::spawn(|| Ok(1)).await;
            std::thread::current().id() == dq_thread
        });
        assert!(same, "continuation did not resume on the DQ thread");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test dq::run_tests`
Expected: FAIL — `DispatcherRuntime::run` not defined.

- [ ] **Step 3: Implement `run`/`run_dedicated`/`spawn_local`**

Follow the design notes, mirroring the DispatcherQueue lifecycle in `src/main.rs:18-20,60-62`. Ensure the Windows App SDK bootstrap is active: the tests must initialize it (add a helper `fn ensure_sdk()` using `winui3::bootstrap::PackageDependency::initialize_version(WindowsAppSDKVersion::V2)` kept alive for the test process via a `OnceLock`). Call `ensure_sdk()` at the top of each `run_tests` test.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test dq::run_tests -- --test-threads=1`
Expected: all PASS. (Serialize: each test owns a current-thread DispatcherQueue.)

- [ ] **Step 5: Commit**

```bash
git add src/dq/mod.rs
git commit -m "feat(dq): run/run_dedicated/spawn_local + cross-thread waker integration"
```

---

### Task 5: `timer.rs` — sleep / interval via DispatcherQueueTimer

**Files:**
- Modify: `src/dq/timer.rs`, `src/dq/mod.rs` (re-export)

**Interfaces:**
- Produces: `pub fn sleep(dur: Duration) -> Sleep;` `pub fn interval(period: Duration) -> Interval;` `Sleep: Future<Output=()>`; `Interval::tick(&mut self) -> Tick<'_>`.

**Design notes:**
- `sleep`: `with_current(|rt| rt.queue.CreateTimer())`; `SetInterval(TimeSpan{Duration: (dur.as_nanos()/100) as i64})`, `SetIsRepeating(false)`, a shared `Rc<RefCell<(bool fired, Option<Waker>)>>`; `Tick(handler)` sets fired + wakes; `Start()`. The Tick handler runs on the DQ thread (single-threaded), so `Rc`/`RefCell` is fine — no Send needed. `DispatcherQueueTimerHandler`/`TypedEventHandler` closure bound: confirm the winio-winui3 `Tick` handler type; capture the `Rc` (DQ-thread-only, so a non-Send closure is acceptable if the delegate allows it — if the delegate requires Send, store the shared state in a thread-local keyed by timer id instead).
- `interval`: `SetIsRepeating(true)` + a tick counter in the shared state; `tick().await` waits for the next increment.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dq::DispatcherRuntime;
    use std::time::{Duration, Instant};
    #[test]
    fn sleep_waits() {
        crate::dq::ensure_sdk();
        let elapsed = DispatcherRuntime::run(async {
            let start = Instant::now();
            sleep(Duration::from_millis(50)).await;
            start.elapsed()
        });
        assert!(elapsed >= Duration::from_millis(40), "{elapsed:?}");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test dq::timer::tests::sleep_waits -- --test-threads=1`
Expected: FAIL — `sleep` not defined.

- [ ] **Step 3: Implement `timer.rs`**

Implement `Sleep`, `Interval`, `interval`, `sleep` per notes. If the `Tick` delegate requires a `Send` closure, use the thread-local-keyed shared-state fallback described in the notes.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test dq::timer::tests::sleep_waits -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/dq/timer.rs src/dq/mod.rs
git commit -m "feat(dq): sleep/interval via DispatcherQueueTimer"
```

---

### Task 6: Integration with comio + `main.rs` demo

**Files:**
- Modify: `src/dq/mod.rs` (integration test), `src/main.rs` (demo)

**Interfaces:**
- Produces: `dq` demo function called from `main`, keeping the existing WinUI path and the `tp` demo intact.

- [ ] **Step 1: Write the failing integration test** (dq awaiting a comio named-pipe echo)

```rust
#[cfg(test)]
mod integration {
    use super::*;
    #[test]
    fn dq_named_pipe_echo() {
        crate::dq::ensure_sdk();
        let name = format!(r"\\.\pipe\dq_comio_{}", std::process::id());
        let sname = name.clone();
        // server on a plain thread using tp::block_on (comio is runtime-agnostic)
        let server = std::thread::spawn(move || {
            use crate::comio::{NamedPipeListener};
            use crate::comio::buffer::new_buffer;
            use crate::tp::block_on;
            use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
            use windows_core::Interface;
            let l = NamedPipeListener::create(&sname);
            let s = block_on(l.accept()).unwrap();
            let i: IInputStream = s.cast().unwrap();
            let o: IOutputStream = s.cast().unwrap();
            let rb = new_buffer(16).unwrap();
            let got: IBuffer = block_on(i.ReadAsync(&rb.cast().unwrap(), 16, InputStreamOptions(0)).unwrap()).unwrap();
            block_on(o.WriteAsync(&got).unwrap()).unwrap();
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let echoed = DispatcherRuntime::run(async move {
            use crate::comio::NamedPipeClient;
            use crate::comio::buffer::{new_buffer, buffer_ptr};
            use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
            use windows_core::Interface;
            let c = NamedPipeClient::connect(&name).await.unwrap();
            let o: IOutputStream = c.cast().unwrap();
            let i: IInputStream = c.cast().unwrap();
            let wb = new_buffer(4).unwrap();
            unsafe { buffer_ptr(&wb.clone().into()).unwrap().copy_from_nonoverlapping(b"echo".as_ptr(), 4); }
            wb.SetLength(4).unwrap();
            o.WriteAsync(&wb.cast().unwrap()).unwrap().await.unwrap();
            let rb = new_buffer(4).unwrap();
            let got: IBuffer = i.ReadAsync(&rb.cast().unwrap(), 4, InputStreamOptions(0)).unwrap().await.unwrap();
            unsafe { std::slice::from_raw_parts(buffer_ptr(&got).unwrap(), 4).to_vec() }
        });
        assert_eq!(&echoed, b"echo");
        server.join().unwrap();
    }
}
```

- [ ] **Step 2: Run to verify it fails, then passes after any wiring fixes**

Run: `cargo test dq::integration::dq_named_pipe_echo -- --test-threads=1`
Expected: PASS (all pieces exist from prior tasks). Fix import/visibility issues if any.

- [ ] **Step 3: Add a `dq` demo to `main.rs`**

Add a `fn dq_demo()` that `DispatcherRuntime::run`s a short async block: `spawn_local` a couple of tasks, `dq::timer::sleep`, and a named-pipe or AF_UNIX echo; print results. Call `dq_demo()` in `main()` after `tp_demo()`, before the existing WinUI section. Keep everything else unchanged.

- [ ] **Step 4: Run the app**

Run: `cargo run`
Expected: prints the `tp` demo lines, then the `dq` demo lines, then the existing WinUI output; clean exit.

- [ ] **Step 5: Full suite + commit**

Run: `cargo test -- --test-threads=1` → all PASS.

```bash
git add src/dq/mod.rs src/main.rs
git commit -m "feat(dq): comio integration + main.rs demo"
```

---

## Self-Review

- **Spec coverage:** executor/waker cross-thread bridge (Tasks 2–4), `spawn_local`→`LocalJoinHandle` with abort/panic (Task 3), `run`/`run_dedicated` both thread models (Task 4), timers (Task 5), comio integration + demo (Task 6). ✔
- **Type consistency:** `ReadyQueue` (T1) → `WakerData` (T2) → `RuntimeInner`/`TaskObj`/`drain` (T3) → `run`/`spawn_local` (T4); `LocalJoinHandle`/`JoinError` names stable across T3–T6. ✔
- **Risks flagged inline:** the `DispatcherQueueTimer::Tick` delegate Send-bound (Task 5 note, with thread-local fallback); DispatcherQueue tests must be single-threaded and SDK-bootstrapped (`ensure_sdk`, `--test-threads=1`). The Task 2/3 ordering note prevents a dangling `RuntimeInner` stub.
- **Placeholders:** Task 3 leaves full `LocalTask`/`LocalJoinHandle` bodies to implementation but pins all types, fields, and semantics; the single-threaded model makes them straightforward. Acceptable.
```
