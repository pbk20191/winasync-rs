# COM-based async IO (named pipe / AF_UNIX) + DispatcherQueue async runtime

**Date:** 2026-07-17
**Status:** Design approved, pending spec review

## Context

The repo already has `src/tp/` — a multithreaded async runtime on the Win32
Thread Pool (TP_WORK/TIMER/WAIT/IO) with `Send + 'static` futures, raw
OVERLAPPED file/TCP IO, and a `block_on`.

This project adds a **single-threaded, UI/COM-affine** async story built on the
WinRT `DispatcherQueue`, plus **COM components that expose named pipe and
AF_UNIX transports through the same WinRT stream interfaces as `StreamSocket`**.

Why: `DispatcherQueue` is effectively a single-threaded executor — an ideal fit
for `!Send` futures that hold UI elements or STA COM pointers, and for awaiting
WinRT async APIs on the UI thread. But **named pipes and AF_UNIX have no WinRT
COM async API** (`StreamSocket` is IP/HostName only; there is no named-pipe
runtime class). To make them first-class in this world, we implement real COM
components (`IInputStream`/`IOutputStream`/`IAsyncOperationWithProgress`) over
Win32 overlapped IO, so they plug into `windows-future`, `DataReader`/
`DataWriter`, and the dispatcher runtime exactly like the built-in socket types.

### Feasibility (verified against crate sources)

- `windows-future` 0.3.2 implements `Future` for all four WinRT async
  interfaces; its Completed handler calls `waker.wake_by_ref()` from an
  arbitrary pool/RTWQ thread (`async.rs:74`). It also proves WinRT
  parameterized async interfaces are implementable in Rust via
  `#[implement(IAsyncOperationWithProgress<T,P>, IAsyncInfo)]`
  (`async_spawn.rs:112`).
- `IInputStream_Impl` (`: IClosable_Impl`) and `IOutputStream_Impl` are
  implementable (`windows` 0.62.2 Storage/Streams/mod.rs:2024, 2136);
  `ReadAsync → IAsyncOperationWithProgress<IBuffer,u32>`,
  `WriteAsync → IAsyncOperationWithProgress<u32,u32>`.
- `IBufferByteAccess` is in `Win32::System::WinRT`; `Buffer` concrete class and
  `IBuffer_Impl` exist.
- `AF_UNIX = 1`, `SOCKADDR_UN`, `CreateNamedPipeW`/`ConnectNamedPipe`/
  `DisconnectNamedPipe` all exist in the `windows` crate.
- Winsock `AcceptEx`/`ConnectEx`/`WSARecv`/`WSASend`, `TP_IO`
  (`CreateThreadpoolIo` with `pcbe = NULL` → default callback environment), and
  `CancelIoEx` are available (already used by `tp/`).

## Decisions (locked)

1. **dq entry point:** `run(root_future)` — creates a DispatcherQueue on the
   current thread, pumps `RunEventLoop`, and `EnqueueEventLoopExit` when the
   root completes. Also `run_dedicated(|| fut)` on a dedicated thread.
2. **`spawn_local` returns `LocalJoinHandle<T>`** — awaitable, `abort()`, panic
   propagated as `Result<T, JoinError>`, drop = detach.
3. **IO scope:** TCP, UDP, file, HTTP (WinRT-backed, awaited directly) **plus
   named pipe and AF_UNIX** implemented as **custom COM components**.
4. **async-op engine A (overlapped):** custom
   `#[implement(IAsyncOperationWithProgress…, IAsyncInfo)]` driven by TP_IO
   overlapped completion with `pcbe = NULL`. No blocked threads.
5. **comio is independent of dq** — it only produces standard WinRT async ops.

## Architecture

Two layers with a one-directional dependency (`dq` may use `comio`; `comio`
never uses `dq`).

### Layer 1 — `src/comio/` (COM components)

Mirrors the WinRT `StreamSocket` / `StreamSocketListener` model. One shared
duplex stream object; per-transport factories differ only in connect/accept.

- **`OverlappedDuplexStream`** — `#[implement(IInputStream, IOutputStream,
  IClosable)]` over a Win32 `HANDLE`/`SOCKET` plus its own `TP_IO`
  (`pcbe = NULL`).
  - `ReadAsync(buf: IBuffer, count, opts)` → starts overlapped
    `ReadFile`/`WSARecv` into the IBuffer's memory (pointer via
    `IBufferByteAccess`), returns a custom
    `IAsyncOperationWithProgress<IBuffer,u32>`; the TP_IO completion sets
    `IBuffer.Length` and fires the Completed handler.
  - `WriteAsync(buf: IBuffer)` → overlapped `WriteFile`/`WSASend` →
    `IAsyncOperationWithProgress<u32,u32>`.
  - `Close` (`IClosable`) cancels in-flight ops (`CancelIoEx`) and closes the
    handle/TP_IO.
  - Being real WinRT streams, they work with `DataReader`/`DataWriter` and any
    WinRT consumer.
- **Factories** (analogous to WinRT types):

  | comio component | WinRT analog | entry API |
  |---|---|---|
  | `NamedPipeClient` | `StreamSocket` (connect) | `ConnectAsync(name) -> OverlappedDuplexStream` |
  | `NamedPipeListener` | `StreamSocketListener` | `Create(name)` + `AcceptAsync()` (overlapped `ConnectNamedPipe`) |
  | `UnixStreamSocket` | `StreamSocket` | `ConnectAsync(path)` (AF_UNIX + `ConnectEx`) |
  | `UnixStreamListener` | `StreamSocketListener` | `Bind(path)` + `AcceptAsync()` (`AcceptEx`) |

- **`asyncop.rs`** — the custom `IAsyncOperationWithProgress<T,P>` +
  `IAsyncInfo` implementation. Holds the op result slot + Completed handler
  (single-assignment, thread-safe like `windows-future`'s SyncState).
  `IAsyncInfo::Cancel` → `CancelIoEx`.
- **`tpio.rs`** — TP_IO registration (`pcbe = NULL`) + a `repr(C)`
  OVERLAPPED-first, refcounted op block co-owned by the kernel while in flight;
  cancel = `CancelIoEx` + ref release; completion callback fills the async-op
  and releases the kernel ref. Mirrors `tp::io` patterns (a shared low-level
  extraction is deliberately deferred — YAGNI).
- **`buffer.rs`** — create `Buffer`, get raw `*mut u8` + capacity via
  `IBufferByteAccess`, set `Length` on completion.

### Layer 2 — `src/dq/` (DispatcherQueue runtime)

- **`mod.rs`** — `DispatcherRuntime`; `run(root_future)`,
  `run_dedicated(|| fut)`, `spawn_local(fut) -> LocalJoinHandle<T>`.
- **`waker.rs`** — `RawWaker` over Send-only data: an agile `DispatcherQueue`
  reference + a `Send + Sync` ready-queue + a `u64` task id. `wake()` pushes the
  id and `TryEnqueue`s a drain handler. The drain handler runs on the DQ thread,
  reaches the runtime via a thread-local, and polls the ready tasks. This is
  what lets a Send waker (called from a pool/RTWQ thread on IO completion) drive
  `!Send` local tasks safely.
- **`task.rs`** — `LocalTask` (single-threaded: `Rc<RefCell<..>>`, a small
  `scheduled`/`state` cell, no atomics). `LocalJoinHandle<T>`:
  `Future<Output = Result<T, JoinError>>`, `abort()`, panic via `catch_unwind`.
- **`timer.rs`** — `sleep(dur)` / `interval(period)` via
  `DispatcherQueueTimer` (`SetInterval`, `SetIsRepeating`, `Tick`, `Start`).

### Data flow (awaiting a named-pipe read on the UI thread)

```
dq task: stream.ReadAsync(buf,n,opts)?.await
   └─ windows-future AsyncFuture::poll sets Completed handler, returns Pending
      (our async-op is IAsyncOperationWithProgress, Status=Started)
   └─ ReadAsync already issued overlapped ReadFile on the pipe handle,
      armed via StartThreadpoolIo (pcbe=NULL default pool)
[pool thread] TP_IO completion callback:
   fill IBuffer.Length, store result, invoke async-op Completed handler
   └─ windows-future handler calls waker.wake_by_ref()   (POOL THREAD)
   └─ dq waker: push task id, DispatcherQueue.TryEnqueue(drain)
[DQ thread] drain handler: poll the task again
   └─ AsyncFuture::poll sees Status=Completed → GetResults → Ready(IBuffer)
   └─ task resumes on the UI/DQ thread
```

## Key invariants & edge cases

- **Waker boundary:** `!Send` tasks are owned locally on the DQ thread; the
  waker touches only Send data and only calls `TryEnqueue`. Every poll runs on
  the DQ thread.
- **OVERLAPPED lifetime:** the op block owns the IBuffer and handle refs while
  in flight; a dropped future/op does `CancelIoEx` only and lets the completion
  callback free the block (owned-buffer discipline, as in `tp::io`).
  `StartThreadpoolIo` is balanced 1:1 by a delivered completion or
  `CancelThreadpoolIo` on synchronous hard failure. Synchronous success still
  queues a completion (`FILE_SKIP_COMPLETION_PORT_ON_SUCCESS` not used).
- **COM object agility:** the implemented objects must be safe to touch from the
  completion (pool) thread and the DQ thread; they carry no thread-affine state
  beyond what is mutex/refcount protected.
- **Cancellation:** `LocalJoinHandle::abort` drops the task future; a comio
  async-op's `IAsyncInfo::Cancel` (and stream `Close`) issue `CancelIoEx`.
  WinRT-backed ops (TCP/file) are cooperatively dropped (windows-future does not
  auto-cancel — documented).
- **Panics:** dq task polls use `catch_unwind` → `JoinError::Panicked`. No panic
  may cross a COM/FFI callback boundary.
- **AF_UNIX/ConnectEx** requires a bound socket first (`bind` to an autobind /
  empty path); `AcceptEx` needs a pre-created accept socket + address buffer and
  `SO_UPDATE_ACCEPT_CONTEXT` after accept; document the version floor (AF_UNIX
  stream sockets: Windows 10 1803+).

## Cargo.toml additions

`[dependencies.windows]` features to add:
`Win32_System_Pipes` (named pipe fns), `Win32_System_WinRT` (`IBufferByteAccess`),
`Storage_Streams` (already present — `IInputStream`/`IOutputStream`/`Buffer`),
plus the `Win32_*` set already added for `tp/` (Foundation, System_Threading,
System_IO, Storage_FileSystem, Networking_WinSock, Security). Confirm the
`windows` crate `implement` support is available for the WinRT stream/async
interfaces (it is generated in-crate).

## Testing strategy

- **comio (standalone, via `tp::block_on`):**
  - named pipe echo: `NamedPipeListener` + `NamedPipeClient`, echo a payload
    both via raw `ReadAsync/WriteAsync` and via `DataReader`/`DataWriter`
    (interop proof).
  - AF_UNIX echo: `UnixStreamListener` + `UnixStreamSocket`.
  - cancel storm: start reads/accepts and drop the futures/ops in flight
    (≥1000×); a debug live-op counter must return to 0.
- **dq (standalone):** `run(async { 42 })`; `spawn_local` + join (value, panic,
  abort, detach); `sleep`/`interval` timing; **cross-thread waker** — a dq task
  awaits a comio op and must resume on the DQ thread (assert thread id).
- **integration:** dq runtime driving named-pipe and AF_UNIX echo end to end,
  wrapped in a timeout.
- **demo:** add a `dq` demo to `main.rs`, keeping the existing WinUI path and
  the `tp` demo intact. Verify clean teardown (no hang/AV at loop exit).

## Build order

1. `comio/buffer.rs` + `tpio.rs` (overlapped engine, `pcbe = NULL`).
2. `comio/asyncop.rs` — custom `IAsyncOperationWithProgress` driven by TP_IO;
   test by awaiting a trivial op with `tp::block_on`.
3. `comio/stream.rs` — `OverlappedDuplexStream`.
4. `comio/pipe.rs` — named pipe (simplest handle IO) + echo test.
5. `comio/unix.rs` — AF_UNIX + echo test.
6. `dq/waker.rs` + `task.rs` + `mod.rs` (`run`, `spawn_local`).
7. `dq/timer.rs`.
8. Integration + `main.rs` demo.

## Out of scope (YAGNI)

- Wrapping WinRT TCP/UDP/file/HTTP in bespoke types — awaited directly via
  `windows-future`; document usage only.
- Extracting a shared low-level overlapped engine between `tp::io` and
  `comio::tpio` — revisit only if duplication becomes a burden.
- AF_UNIX datagram; named pipe message-mode nuances beyond byte streams.
