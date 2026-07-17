# comio: COM async IO components (named pipe / AF_UNIX) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `src/comio/` — COM components exposing named pipe and AF_UNIX transports through the same WinRT stream interfaces (`IInputStream`/`IOutputStream`/`IAsyncOperationWithProgress`) as `StreamSocket`, backed by Win32 overlapped IO on the default thread-pool callback environment.

**Architecture:** A custom `#[implement(IAsyncOperationWithProgress<T,P>, IAsyncInfo)]` COM object is driven by a `TP_IO` (created with `pcbe = NULL`) overlapped completion. `OverlappedDuplexStream` implements `IInputStream + IOutputStream + IClosable` over a Win32 `HANDLE`/`SOCKET`, returning those async ops from `ReadAsync`/`WriteAsync`. Per-transport factories (`NamedPipe*`, `Unix*`) build the stream. Everything produces standard WinRT async interfaces, so `windows-future` turns them into Rust futures and `DataReader`/`DataWriter` consume them.

**Tech Stack:** Rust (edition 2024), `windows` 0.62.2 (Win32 + WinRT + `implement`), `windows-core`, `windows-future` 0.3.2, existing `src/tp/` (used only for `tp::block_on` in tests).

## Global Constraints

- Platform: Windows only. AF_UNIX stream sockets require Windows 10 1803+.
- Bindings: official `windows` 0.62.2 crate features only (no `windows-link` hand bindings).
- `pcbe = NULL` (default callback environment) for every `CreateThreadpoolIo` in comio — comio owns no `TP_POOL`.
- OVERLAPPED discipline (identical to `src/tp/io/mod.rs`): op block is `repr(C)` with `OVERLAPPED` first; `StartThreadpoolIo` before every issue, balanced 1:1 by a delivered completion or `CancelThreadpoolIo` on synchronous hard failure; `FILE_SKIP_COMPLETION_PORT_ON_SUCCESS` is never set (synchronous success still delivers a completion); a future/op dropped in flight calls `CancelIoEx` only and lets the completion callback free the block.
- No panic may cross a COM/FFI callback boundary.
- Buffers the kernel touches must be owned by the op block for its whole lifetime (the `IBuffer` for reads/writes).
- Commit after every task. Never use `--no-verify`.

## Reference facts (verified in crate sources)

- `IInputStream_Impl: IClosable_Impl { fn ReadAsync(&self, buffer: Ref<IBuffer>, count: u32, options: InputStreamOptions) -> Result<IAsyncOperationWithProgress<IBuffer, u32>> }` (Storage/Streams/mod.rs:2024).
- `IOutputStream_Impl: IClosable_Impl { fn WriteAsync(&self, buffer: Ref<IBuffer>) -> Result<IAsyncOperationWithProgress<u32, u32>>; fn FlushAsync(&self) -> Result<IAsyncOperation<bool>> }` (mod.rs:2136).
- `IClosable_Impl { fn Close(&self) -> Result<()> }`.
- `Buffer::Create(capacity: u32) -> Result<Buffer>`; `.Capacity()`, `.Length()`, `.SetLength(u32)`; cast `IBuffer -> IBufferByteAccess`, then `IBufferByteAccess::Buffer() -> Result<*mut u8>` (Win32/System/WinRT).
- `windows-future` `#[implement(IAsyncOperationWithProgress<T,P>, IAsyncInfo)]` pattern (async_spawn.rs:112); `Completed` handler is invoked on the completing thread and calls the Rust waker.
- `SOCKADDR_UN { sun_family: ADDRESS_FAMILY, sun_path: [i8; 108] }`; `AF_UNIX = 1`.
- `CreateNamedPipeW`, `ConnectNamedPipe`, `DisconnectNamedPipe` in `Win32::System::Pipes`.
- `TP_IO` API + `CancelIoEx` as already used in `src/tp/io/mod.rs`.

---

### Task 1: Cargo features + `comio` module skeleton

**Files:**
- Modify: `Cargo.toml` (add `windows` features)
- Create: `src/comio/mod.rs`
- Modify: `src/main.rs` (add `mod comio;`)

**Interfaces:**
- Produces: module `comio` compiles; `comio::check` helper `fn check<T>(r: windows_core::Result<T>) -> T`.

- [ ] **Step 1: Add windows features**

In `Cargo.toml`, extend the `[dependencies.windows]` `features` array with (append to the existing list added for `tp/`):

```toml
    "Win32_System_Pipes", "Win32_System_WinRT",
    "Storage_Streams", "Foundation",
```

(`Storage_Streams`/`Foundation` are already present via other features; duplicates are harmless. `Win32_System_Pipes` and `Win32_System_WinRT` are new.)

- [ ] **Step 2: Create `src/comio/mod.rs`**

```rust
//! COM components exposing named pipe and AF_UNIX transports through the same
//! WinRT stream interfaces (`IInputStream`/`IOutputStream`/
//! `IAsyncOperationWithProgress`) as `StreamSocket`, backed by Win32 overlapped
//! IO on the default thread-pool callback environment (`pcbe = NULL`).
#![allow(dead_code)]

pub mod tpio;

/// Thread pool / overlapped calls only fail on resource exhaustion; map those
/// to a panic, matching `tp`'s policy.
pub(crate) fn check<T>(r: windows_core::Result<T>) -> T {
    r.expect("comio overlapped allocation failed")
}
```

- [ ] **Step 3: Wire the module in `src/main.rs`**

Add near the existing `mod tp;`:

```rust
mod comio;
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check`
Expected: builds; new features resolve (only dead-code warnings from `check`).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/comio/mod.rs src/main.rs
git commit -m "feat(comio): module skeleton + windows features"
```

---

### Task 2: `tpio` — TP_IO overlapped engine with a caller-supplied completion action

The engine reuses the `tp::io` OVERLAPPED discipline but the completion action
is a boxed callback (later: "fill the IBuffer + fire the COM Completed
handler") rather than a Rust waker.

**Files:**
- Create: `src/comio/tpio.rs`
- Test: inline `#[cfg(test)]` in `src/comio/tpio.rs`

**Interfaces:**
- Consumes: `comio::check`.
- Produces:
  - `pub(crate) struct IoReg` with `pub(crate) fn register(handle: HANDLE, is_socket: bool) -> windows_core::Result<Arc<IoReg>>` and `pub(crate) handle: HANDLE`.
  - `pub(crate) enum Issue { Pending, Failed(windows_core::HRESULT) }`
  - `pub(crate) struct OpHandle` (opaque; identifies an in-flight op for cancellation) with `pub(crate) fn cancel(&self)`.
  - `pub(crate) unsafe fn submit(reg: &Arc<IoReg>, on_complete: Box<dyn FnOnce(u32 /*ioresult*/, usize /*bytes*/) + Send>, issue: impl FnOnce(*mut OVERLAPPED) -> Issue) -> OpHandle` — arms `StartThreadpoolIo`, runs `issue`, and on `Failed` calls `CancelThreadpoolIo` and invokes `on_complete(hr_as_win32, 0)` synchronously.
  - `#[cfg(debug_assertions)] pub(crate) static LIVE_OPS: AtomicUsize`.

**Design notes for the implementer (all mandatory):**
- Op block is `#[repr(C)]` with `overlapped: OVERLAPPED` as the FIRST field so the completion callback's `overlapped` pointer equals the block pointer.
- Refcount starts at 2 (future/caller side + kernel/completion side). `submit` returns an `OpHandle` holding the caller-side ref; `OpHandle::cancel` = `CancelIoEx(reg.handle, &overlapped)` (ignore `ERROR_NOT_FOUND`). Dropping `OpHandle` releases the caller-side ref.
- The single `io_complete` extern callback: read result under a lock, take the boxed `on_complete`, call it, then release the kernel-side ref (frees the block + possibly closes the reg when the last ref drops).
- `CreateThreadpoolIo(handle, Some(io_complete), None, None)` — the last `None` is `pcbe = NULL`.
- `IoReg::Drop`: `CloseThreadpoolIo` then `CloseHandle`/`closesocket`. Do NOT call `WaitForThreadpoolIoCallbacks` (would deadlock if Drop runs at a completion tail).
- Copy the mechanics from `src/tp/io/mod.rs` (`Op`, `release_op_ref`, `submit`, `OpFuture::Drop`), adapting: the "completion action" is `Box<dyn FnOnce(u32, usize) + Send>` instead of a waker, and there is no `OpFuture` (the async-op object owns cancellation via `OpHandle`).

- [ ] **Step 1: Write the failing test** (round-trips a byte through an anonymous pipe via overlapped IO)

Add to `src/comio/tpio.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, ERROR_IO_PENDING};
    use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile, CreateFileW, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows::Win32::System::Pipes::CreateNamedPipeW;
    use windows::Win32::System::Pipes::PIPE_ACCESS_DUPLEX;
    use windows::Win32::System::Pipes::{PIPE_TYPE_BYTE, PIPE_WAIT};

    // Minimal overlapped write-then-read over a named pipe pair.
    #[test]
    fn overlapped_pipe_roundtrip() {
        let name = format!(r"\\.\pipe\comio_tpio_{}", std::process::id());
        let wname = windows_core::HSTRING::from(&name);
        let server = unsafe {
            CreateNamedPipeW(&wname, windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0), PIPE_TYPE_BYTE | PIPE_WAIT, 1, 4096, 4096, 0, None)
        }.unwrap();
        // Client opens the pipe (synchronous connect is fine for the test).
        let client = unsafe {
            CreateFileW(&wname, FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, None)
        }.unwrap();

        let sreg = IoReg::register(server, false).unwrap();
        let creg = IoReg::register(client, false).unwrap();

        // Server writes "Z" via overlapped IO.
        let (tx, rx) = mpsc::channel();
        let wbuf = Box::new([b'Z']);
        let wptr = wbuf.as_ptr();
        let _wop = unsafe {
            submit(&sreg, Box::new(move |ioresult, bytes| { let _ = &wbuf; tx.send((ioresult, bytes)).unwrap(); }), |ovl| {
                match WriteFile(sreg.handle, Some(std::slice::from_raw_parts(wptr, 1)), None, Some(ovl)) {
                    Ok(()) => Issue::Pending,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        };
        let (wres, wbytes) = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!((wres, wbytes), (0, 1));

        // Client reads it back.
        let (tx2, rx2) = mpsc::channel();
        let rbuf = std::sync::Arc::new(std::sync::Mutex::new([0u8; 1]));
        let rbuf2 = rbuf.clone();
        let rptr = rbuf.lock().unwrap().as_mut_ptr();
        let _rop = unsafe {
            submit(&creg, Box::new(move |ioresult, bytes| { let _ = &rbuf2; tx2.send((ioresult, bytes)).unwrap(); }), |ovl| {
                match ReadFile(creg.handle, Some(std::slice::from_raw_parts_mut(rptr, 1)), None, Some(ovl)) {
                    Ok(()) => Issue::Pending,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        };
        let (rres, rbytes) = rx2.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!((rres, rbytes), (0, 1));
        assert_eq!(rbuf.lock().unwrap()[0], b'Z');

        drop(sreg); drop(creg);
        let _ = server; let _ = client; // handles closed by IoReg::Drop
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test comio::tpio::tests::overlapped_pipe_roundtrip`
Expected: FAIL to compile — `IoReg`, `submit`, `Issue` not defined.

- [ ] **Step 3: Implement `tpio.rs`**

Port `src/tp/io/mod.rs`'s op-block machinery with the completion action changed to a boxed `FnOnce(u32, usize)`. Concretely:

```rust
use super::check;
use core::ffi::c_void;
use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Networking::WinSock::{closesocket, SOCKET};
use windows::Win32::System::Threading::{
    CancelThreadpoolIo, CloseThreadpoolIo, CreateThreadpoolIo, StartThreadpoolIo,
    PTP_CALLBACK_INSTANCE, PTP_IO,
};
use windows::Win32::System::IO::{CancelIoEx, OVERLAPPED};

#[cfg(debug_assertions)]
pub(crate) static LIVE_OPS: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct IoReg {
    pub(crate) handle: HANDLE,
    io: PTP_IO,
    is_socket: bool,
}
unsafe impl Send for IoReg {}
unsafe impl Sync for IoReg {}

impl IoReg {
    pub(crate) fn register(handle: HANDLE, is_socket: bool) -> windows_core::Result<Arc<Self>> {
        let io = unsafe { CreateThreadpoolIo(handle, Some(io_complete), None, None)? };
        Ok(Arc::new(Self { handle, io, is_socket }))
    }
    fn socket(&self) -> SOCKET { SOCKET(self.handle.0 as usize) }
}
impl Drop for IoReg {
    fn drop(&mut self) {
        unsafe {
            CloseThreadpoolIo(self.io);
            if self.is_socket { closesocket(self.socket()); } else { let _ = CloseHandle(self.handle); }
        }
    }
}

pub(crate) enum Issue { Pending, Failed(windows_core::HRESULT) }

type CompleteFn = Box<dyn FnOnce(u32, usize) + Send>;

struct OpShared { fired: bool, on_complete: Option<CompleteFn> }

struct OpHeader {
    refs: AtomicUsize,
    shared: Mutex<OpShared>,
    reg: Arc<IoReg>,
}

#[repr(C)]
struct Op {
    overlapped: OVERLAPPED,
    header: OpHeader,
}

unsafe fn release_ref(op: *mut c_void) {
    let header = &(*(op as *mut Op)).header;
    if header.refs.fetch_sub(1, Ordering::Release) == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        #[cfg(debug_assertions)]
        LIVE_OPS.fetch_sub(1, Ordering::Relaxed);
        drop(Box::from_raw(op as *mut Op));
    }
}

unsafe extern "system" fn io_complete(
    _: PTP_CALLBACK_INSTANCE, _ctx: *mut c_void, overlapped: *mut c_void,
    ioresult: u32, bytes: usize, _io: PTP_IO,
) {
    let header = &(*(overlapped as *mut Op)).header;
    let cb = { let mut s = header.shared.lock().unwrap(); s.fired = true; s.on_complete.take() };
    if let Some(cb) = cb {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || cb(ioresult, bytes)));
    }
    release_ref(overlapped);
}

pub(crate) struct OpHandle { op: *mut Op }
unsafe impl Send for OpHandle {}
impl OpHandle {
    pub(crate) fn cancel(&self) {
        let header = unsafe { &(*self.op).header };
        let in_flight = !header.shared.lock().unwrap().fired;
        if in_flight {
            unsafe { let _ = CancelIoEx(header.reg.handle, Some(&(*self.op).overlapped)); }
        }
    }
}
impl Drop for OpHandle {
    fn drop(&mut self) { unsafe { release_ref(self.op as *mut c_void) } }
}

pub(crate) unsafe fn submit(
    reg: &Arc<IoReg>, on_complete: CompleteFn,
    issue: impl FnOnce(*mut OVERLAPPED) -> Issue,
) -> OpHandle {
    let op = Box::into_raw(Box::new(Op {
        overlapped: mem::zeroed(),
        header: OpHeader { refs: AtomicUsize::new(2), shared: Mutex::new(OpShared { fired: false, on_complete: Some(on_complete) }), reg: reg.clone() },
    }));
    #[cfg(debug_assertions)]
    LIVE_OPS.fetch_add(1, Ordering::Relaxed);
    StartThreadpoolIo(reg.io);
    match issue(&raw mut (*op).overlapped) {
        Issue::Pending => {}
        Issue::Failed(hr) => {
            CancelThreadpoolIo(reg.io);
            let cb = { let mut s = (*op).header.shared.lock().unwrap(); s.fired = true; s.on_complete.take() };
            if let Some(cb) = cb {
                let win32 = windows::Win32::Foundation::WIN32_ERROR::from_error(&windows_core::Error::from_hresult(hr)).map(|e| e.0).unwrap_or(0x13);
                cb(win32, 0);
            }
            release_ref(op as *mut c_void);
        }
    }
    OpHandle { op }
}
```

Note: if `WIN32_ERROR::from_error` is unavailable, map `hr` to a non-zero error code however the `windows` crate allows (any nonzero signals failure to the async-op layer); the exact code only needs to be nonzero and roughly correct.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test comio::tpio::tests::overlapped_pipe_roundtrip`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/comio/tpio.rs src/comio/mod.rs
git commit -m "feat(comio): TP_IO overlapped engine with boxed completion action"
```

---

### Task 3: `buffer` — IBuffer helpers

**Files:**
- Create: `src/comio/buffer.rs`
- Modify: `src/comio/mod.rs` (add `pub mod buffer;`)
- Test: inline in `src/comio/buffer.rs`

**Interfaces:**
- Produces:
  - `pub(crate) fn new_buffer(capacity: u32) -> windows_core::Result<Buffer>`
  - `pub(crate) fn buffer_ptr(buf: &IBuffer) -> windows_core::Result<*mut u8>` (via `IBufferByteAccess`)
  - `pub(crate) fn buffer_capacity(buf: &IBuffer) -> windows_core::Result<u32>`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn buffer_ptr_writable() {
        let buf = new_buffer(8).unwrap();
        let ib: windows::Storage::Streams::IBuffer = buf.clone().into();
        let p = buffer_ptr(&ib).unwrap();
        unsafe { *p = 0xAB; }
        assert_eq!(buffer_capacity(&ib).unwrap(), 8);
        assert!(!p.is_null());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test comio::buffer::tests::buffer_ptr_writable`
Expected: FAIL — `new_buffer` not defined.

- [ ] **Step 3: Implement `buffer.rs`**

```rust
use windows::Storage::Streams::{Buffer, IBuffer};
use windows::Win32::System::WinRT::IBufferByteAccess;
use windows_core::Interface;

pub(crate) fn new_buffer(capacity: u32) -> windows_core::Result<Buffer> {
    Buffer::Create(capacity)
}

pub(crate) fn buffer_ptr(buf: &IBuffer) -> windows_core::Result<*mut u8> {
    let access: IBufferByteAccess = buf.cast()?;
    unsafe { access.Buffer() }
}

pub(crate) fn buffer_capacity(buf: &IBuffer) -> windows_core::Result<u32> {
    buf.Capacity()
}
```

Add `pub mod buffer;` to `src/comio/mod.rs`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test comio::buffer::tests::buffer_ptr_writable`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/comio/buffer.rs src/comio/mod.rs
git commit -m "feat(comio): IBuffer helpers via IBufferByteAccess"
```

---

### Task 4: `asyncop` — custom `IAsyncOperationWithProgress` driven by TP_IO (THE keystone)

A COM object implementing `IAsyncOperationWithProgress<T, P> + IAsyncInfo`
whose result is filled by a TP_IO completion. This validates the entire
"custom WinRT async op" approach; build and test it in isolation before any
stream/transport work.

**Files:**
- Create: `src/comio/asyncop.rs`
- Modify: `src/comio/mod.rs` (add `pub mod asyncop;`)
- Test: inline in `src/comio/asyncop.rs`

**Interfaces:**
- Consumes: `tpio::{IoReg, submit, Issue, OpHandle}`, `windows-future` async interfaces.
- Produces:
  - `pub(crate) struct IoAsyncOp<T: RuntimeType + 'static>` COM object with a constructor
    `pub(crate) fn start_read(reg: &Arc<IoReg>, out: IBuffer, issue: impl FnOnce(*mut OVERLAPPED) -> Issue + Send + 'static) -> IAsyncOperationWithProgress<IBuffer, u32>` and
    `pub(crate) fn start_write(reg: &Arc<IoReg>, count_hint: u32, issue: impl FnOnce(*mut OVERLAPPED) -> Issue + Send + 'static) -> IAsyncOperationWithProgress<u32, u32>`.
  - The op's `IAsyncInfo::Cancel` calls the stored `OpHandle::cancel`.

**Design notes (mandatory):**
- Model the state container on `windows-future`'s `SyncState` (async_spawn.rs): a `Mutex<{ result: Option<Result<Output>>, completed: Option<CompletedHandler>, completed_assigned, op: Option<OpHandle> }>`.
- Implement with `#[implement(IAsyncOperationWithProgress<Out, u32>, IAsyncInfo)]`. Provide `IAsyncInfo_Impl` (Status from result, ErrorCode, `Cancel` → `op.cancel()`, `Close`) and `IAsyncOperationWithProgress_Impl` (`SetCompleted`/`Completed`/`GetResults`/`SetProgress`/`Progress`). Mirror async_spawn.rs exactly for the parts that aren't IO-specific.
- Construction: create the `ComObject`, capture a clone of its interface into the TP_IO completion closure. In `submit`'s `on_complete(ioresult, bytes)`: under the state lock set `result = Some(Ok(output))` where for reads `output` is the passed-in `IBuffer` after `SetLength(bytes)` (mapping `ERROR_HANDLE_EOF`→len 0) and for writes `output = bytes as u32`; a nonzero `ioresult` sets `Err`. Then take + invoke the Completed handler (outside the lock).
- Because the completion may arrive before `SetCompleted`, `SetCompleted` must, if `result` is already present, invoke the handler immediately (again mirror `SyncState::set_completed`).
- Store the `OpHandle` returned by `tpio::submit` in the state so `Cancel` can reach it.
- The object must be agile-safe: it holds only `Send` state behind a `Mutex` and the `IBuffer`/interfaces (agile). Do not store thread-affine data.

- [ ] **Step 1: Write the failing test** (await a real read op end-to-end using `tp::block_on`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::buffer::{new_buffer, buffer_ptr};
    use crate::comio::tpio::{IoReg, Issue};
    use crate::tp::block_on;
    use windows::Storage::Streams::IBuffer;
    use windows::Win32::Foundation::{ERROR_IO_PENDING, FILE_FLAGS_AND_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::{CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_ACCESS_DUPLEX, PIPE_TYPE_BYTE, PIPE_WAIT};

    #[test]
    fn async_read_completes() {
        let name = format!(r"\\.\pipe\comio_asyncop_{}", std::process::id());
        let wname = windows_core::HSTRING::from(&name);
        let server = unsafe { CreateNamedPipeW(&wname, FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0), PIPE_TYPE_BYTE | PIPE_WAIT, 1, 4096, 4096, 0, None) }.unwrap();
        let client = unsafe { CreateFileW(&wname, FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, None) }.unwrap();
        let sreg = IoReg::register(server, false).unwrap();
        let creg = IoReg::register(client, false).unwrap();

        // Server: async write "hi".
        let wbuf = new_buffer(2).unwrap();
        let wib: IBuffer = wbuf.clone().into();
        unsafe { let p = buffer_ptr(&wib).unwrap(); p.copy_from_nonoverlapping(b"hi".as_ptr(), 2); }
        wbuf.SetLength(2).unwrap();
        let wp = buffer_ptr(&wib).unwrap();
        let write_op = IoAsyncOp::<u32>::start_write(&sreg, 2, move |ovl| {
            match unsafe { WriteFile(sreg.handle, Some(std::slice::from_raw_parts(wp, 2)), None, Some(ovl)) } {
                Ok(()) => Issue::Pending,
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                Err(e) => Issue::Failed(e.code()),
            }
        });

        // Client: async read 2 bytes.
        let rbuf = new_buffer(2).unwrap();
        let rib: IBuffer = rbuf.clone().into();
        let rp = buffer_ptr(&rib).unwrap();
        let read_op = IoAsyncOp::start_read(&creg, rib.clone(), move |ovl| {
            match unsafe { ReadFile(creg.handle, Some(std::slice::from_raw_parts_mut(rp, 2)), None, Some(ovl)) } {
                Ok(()) => Issue::Pending,
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                Err(e) => Issue::Failed(e.code()),
            }
        });

        let wrote = block_on(write_op).unwrap();
        assert_eq!(wrote, 2);
        let got = block_on(read_op).unwrap();
        assert_eq!(got.Length().unwrap(), 2);
        unsafe { assert_eq!(std::slice::from_raw_parts(buffer_ptr(&got).unwrap(), 2), b"hi"); }
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test comio::asyncop::tests::async_read_completes`
Expected: FAIL — `IoAsyncOp` not defined.

- [ ] **Step 3: Implement `asyncop.rs`**

Follow the design notes. Structure (fill the bodies by mirroring `windows-future/src/async_spawn.rs` for the `IAsyncInfo`/`IAsyncOperationWithProgress` methods and `set_completed` logic):

```rust
use super::check;
use super::tpio::{submit, IoReg, Issue, OpHandle};
use std::sync::{Arc, Mutex};
use windows::Storage::Streams::IBuffer;
use windows::Win32::System::IO::OVERLAPPED;
use windows_core::{implement, ComObject, IInspectableImpl, Interface, Ref, Result, HRESULT, RuntimeType};
use windows_future::{
    AsyncOperationWithProgressCompletedHandler, AsyncOperationProgressHandler,
    IAsyncOperationWithProgress, IAsyncOperationWithProgress_Impl,
};
use windows_future::{AsyncStatus, IAsyncInfo_Impl};

struct OpState<T: RuntimeType + 'static> {
    result: Option<Result<T>>,
    completed: Option<AsyncOperationWithProgressCompletedHandler<T, u32>>,
    completed_assigned: bool,
    op: Option<OpHandle>,
}

#[implement(IAsyncOperationWithProgress<T, u32>, windows_future::IAsyncInfo)]
pub(crate) struct IoAsyncOp<T: RuntimeType + 'static>(Mutex<OpState<T>>);

// IAsyncInfo_Impl: Status/ErrorCode from result; Cancel -> op.cancel(); Close ok.
// IAsyncOperationWithProgress_Impl: SetCompleted (mirror SyncState::set_completed,
//   invoking immediately if result present), GetResults (clone), SetProgress/Progress no-op.
// Provide two associated constructors that call tpio::submit with an on_complete
// closure that fills `result` and fires the completed handler, then store the
// returned OpHandle into state.op.
```

The two constructors:

```rust
impl IoAsyncOp<IBuffer> {
    pub(crate) fn start_read(
        reg: &Arc<IoReg>, out: IBuffer,
        issue: impl FnOnce(*mut OVERLAPPED) -> Issue + Send + 'static,
    ) -> IAsyncOperationWithProgress<IBuffer, u32> {
        let object = ComObject::new(IoAsyncOp(Mutex::new(OpState { result: None, completed: None, completed_assigned: false, op: None })));
        let iface: IAsyncOperationWithProgress<IBuffer, u32> = object.to_interface();
        let sender = iface.clone();
        let out_for_cb = out.clone();
        let handle = unsafe {
            submit(reg, Box::new(move |ioresult, bytes| {
                // EOF -> length 0; other nonzero -> error.
                let res = if ioresult == 0 || ioresult == windows::Win32::Foundation::ERROR_HANDLE_EOF.0 {
                    let _ = out_for_cb.SetLength(if ioresult == 0 { bytes as u32 } else { 0 });
                    Ok(out_for_cb.clone())
                } else {
                    Err(windows_core::Error::from_hresult(windows::Win32::Foundation::WIN32_ERROR(ioresult).to_hresult()))
                };
                complete::<IBuffer>(&sender, res);
            }), issue)
        };
        object.0.lock().unwrap().op = Some(handle);
        iface
    }
}

impl IoAsyncOp<u32> {
    pub(crate) fn start_write(
        reg: &Arc<IoReg>, _count_hint: u32,
        issue: impl FnOnce(*mut OVERLAPPED) -> Issue + Send + 'static,
    ) -> IAsyncOperationWithProgress<u32, u32> {
        let object = ComObject::new(IoAsyncOp(Mutex::new(OpState { result: None, completed: None, completed_assigned: false, op: None })));
        let iface: IAsyncOperationWithProgress<u32, u32> = object.to_interface();
        let sender = iface.clone();
        let handle = unsafe {
            submit(reg, Box::new(move |ioresult, bytes| {
                let res = if ioresult == 0 { Ok(bytes as u32) } else { Err(windows_core::Error::from_hresult(windows::Win32::Foundation::WIN32_ERROR(ioresult).to_hresult())) };
                complete::<u32>(&sender, res);
            }), issue)
        };
        object.0.lock().unwrap().op = Some(handle);
        iface
    }
}
```

Where `complete::<T>` downcasts the sender interface back to the `IoAsyncOp<T>` impl (via `IAsyncInfo` cast or by keeping a typed handle), stores the result, and invokes the completed handler. If downcasting the interface back to the concrete impl proves awkward, instead capture `object.clone()` (the `ComObject`) — not just the interface — into the closure and call a method on it directly; `ComObject` derefs to the impl. Prefer capturing the `ComObject` clone; adjust `sender` to `object.clone()` and call `object.complete(res)` where `complete` is an inherent method that locks the state, stores the result, and invokes the completed handler using `object.to_interface()` as the sender argument.

Add `pub mod asyncop;` to `src/comio/mod.rs`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test comio::asyncop::tests::async_read_completes -- --nocapture`
Expected: PASS. If `#[implement]` on the generic interface fails to resolve the parameterized IID, confirm `windows-future` is the source of the `_Impl` traits and import them from there (as async_spawn.rs does).

- [ ] **Step 5: Commit**

```bash
git add src/comio/asyncop.rs src/comio/mod.rs
git commit -m "feat(comio): custom IAsyncOperationWithProgress driven by TP_IO"
```

---

### Task 5: `stream` — `OverlappedDuplexStream` implementing IInputStream/IOutputStream/IClosable

**Files:**
- Create: `src/comio/stream.rs`
- Modify: `src/comio/mod.rs` (add `pub mod stream;`)
- Test: inline in `src/comio/stream.rs`

**Interfaces:**
- Consumes: `tpio::IoReg`, `asyncop::IoAsyncOp`, `buffer::buffer_ptr`.
- Produces:
  - `pub struct OverlappedDuplexStream` (COM object) with `pub(crate) fn wrap(reg: Arc<IoReg>) -> IInspectable`-style factory returning the object; expose `pub fn input(&self) -> IInputStream` and `pub fn output(&self) -> IOutputStream` (both return `self`-cast interfaces) — or return the object typed so callers can `.cast()`.
  - Public constructor used by transports: `pub(crate) fn from_reg(reg: Arc<IoReg>) -> OverlappedDuplexStream_wrapper` (the `ComObject`'s interface set).

**Design notes:**
- `#[implement(IInputStream, IOutputStream, windows::Foundation::IClosable)]` over `Arc<IoReg>`.
- `ReadAsync(buffer: Ref<IBuffer>, count, _options)`: get `*mut u8` via `buffer_ptr`, min(count, capacity); call `IoAsyncOp::start_read(&reg, buffer.clone(), |ovl| ReadFile/WSARecv...)`. For a byte-stream handle use `ReadFile`; for a socket reg use `WSARecv`. Decide by `reg.is_socket` — expose an `is_socket` getter on `IoReg`.
- `WriteAsync(buffer: Ref<IBuffer>)`: length = `buffer.Length()`; `IoAsyncOp::start_write(&reg, len, |ovl| WriteFile/WSASend...)`.
- `FlushAsync`: return an already-completed `IAsyncOperation<bool>` (`IAsyncOperation::<bool>::spawn(|| Ok(true))` from `windows-future`).
- `Close`: nothing extra needed (dropping the last ref drops `IoReg` which closes the handle); return `Ok(())`.

- [ ] **Step 1: Write the failing test** (build a stream over a pipe pair and echo through the WinRT `ReadAsync`/`WriteAsync` surface)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::buffer::{new_buffer, buffer_ptr};
    use crate::comio::tpio::IoReg;
    use crate::tp::block_on;
    use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
    use windows::Win32::Foundation::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_ACCESS_DUPLEX, PIPE_TYPE_BYTE, PIPE_WAIT};
    use windows_core::Interface;

    #[test]
    fn stream_read_write_roundtrip() {
        let name = format!(r"\\.\pipe\comio_stream_{}", std::process::id());
        let wname = windows_core::HSTRING::from(&name);
        let server = unsafe { CreateNamedPipeW(&wname, FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0), PIPE_TYPE_BYTE | PIPE_WAIT, 1, 4096, 4096, 0, None) }.unwrap();
        let client = unsafe { CreateFileW(&wname, FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, None) }.unwrap();
        let s = from_reg(IoReg::register(server, false).unwrap());
        let c = from_reg(IoReg::register(client, false).unwrap());
        let s_out: IOutputStream = s.cast().unwrap();
        let c_in: IInputStream = c.cast().unwrap();

        let wbuf = new_buffer(3).unwrap();
        unsafe { buffer_ptr(&wbuf.clone().into()).unwrap().copy_from_nonoverlapping(b"abc".as_ptr(), 3); }
        wbuf.SetLength(3).unwrap();
        let wrote = block_on(s_out.WriteAsync(&wbuf.cast::<IBuffer>().unwrap()).unwrap()).unwrap();
        assert_eq!(wrote, 3);

        let rbuf = new_buffer(3).unwrap();
        let got: IBuffer = block_on(c_in.ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), 3, InputStreamOptions(0)).unwrap()).unwrap();
        assert_eq!(got.Length().unwrap(), 3);
        unsafe { assert_eq!(std::slice::from_raw_parts(buffer_ptr(&got).unwrap(), 3), b"abc"); }
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test comio::stream::tests::stream_read_write_roundtrip`
Expected: FAIL — `from_reg` not defined.

- [ ] **Step 3: Implement `stream.rs`**

```rust
use super::asyncop::IoAsyncOp;
use super::buffer::buffer_ptr;
use super::tpio::{IoReg, Issue};
use std::sync::Arc;
use windows::Foundation::IClosable_Impl;
use windows::Storage::Streams::{
    IBuffer, IInputStream, IInputStream_Impl, IOutputStream, IOutputStream_Impl, InputStreamOptions,
};
use windows::Win32::Foundation::ERROR_IO_PENDING;
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::Networking::WinSock::{WSARecv, WSASend, WSABUF, SOCKET};
use windows_core::{implement, ComObject, Interface, Ref, Result};
use windows_future::{IAsyncOperation, IAsyncOperationWithProgress};

#[implement(IInputStream, IOutputStream, windows::Foundation::IClosable)]
pub struct OverlappedDuplexStream {
    reg: Arc<IoReg>,
}

pub(crate) fn from_reg(reg: Arc<IoReg>) -> windows_core::IInspectable {
    ComObject::new(OverlappedDuplexStream { reg }).to_interface()
}

impl IClosable_Impl for OverlappedDuplexStream_Impl {
    fn Close(&self) -> Result<()> { Ok(()) }
}

impl IInputStream_Impl for OverlappedDuplexStream_Impl {
    fn ReadAsync(&self, buffer: Ref<IBuffer>, count: u32, _options: InputStreamOptions)
        -> Result<IAsyncOperationWithProgress<IBuffer, u32>>
    {
        let ib = buffer.ok()?.clone();
        let ptr = buffer_ptr(&ib)?;
        let n = count.min(ib.Capacity()?);
        let reg = self.reg.clone();
        let is_socket = reg.is_socket();
        let sock = SOCKET(reg.handle.0 as usize);
        let handle = reg.handle;
        Ok(IoAsyncOp::start_read(&reg, ib, move |ovl| unsafe {
            if is_socket {
                let mut flags = 0u32;
                let wsabuf = WSABUF { len: n, buf: windows_core::PSTR(ptr) };
                let r = WSARecv(sock, &[wsabuf], None, &mut flags, Some(ovl), None);
                if r == 0 { Issue::Pending }
                else if windows::Win32::Networking::WinSock::WSAGetLastError() == windows::Win32::Networking::WinSock::WSA_IO_PENDING { Issue::Pending }
                else { Issue::Failed(windows::Win32::Foundation::WIN32_ERROR(windows::Win32::Networking::WinSock::WSAGetLastError().0 as u32).to_hresult()) }
            } else {
                match ReadFile(handle, Some(std::slice::from_raw_parts_mut(ptr, n as usize)), None, Some(ovl)) {
                    Ok(()) => Issue::Pending,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            }
        }))
    }
}

impl IOutputStream_Impl for OverlappedDuplexStream_Impl {
    fn WriteAsync(&self, buffer: Ref<IBuffer>) -> Result<IAsyncOperationWithProgress<u32, u32>> {
        let ib = buffer.ok()?.clone();
        let ptr = buffer_ptr(&ib)?;
        let len = ib.Length()?;
        let reg = self.reg.clone();
        let is_socket = reg.is_socket();
        let sock = SOCKET(reg.handle.0 as usize);
        let handle = reg.handle;
        Ok(IoAsyncOp::start_write(&reg, len, move |ovl| unsafe {
            if is_socket {
                let wsabuf = WSABUF { len, buf: windows_core::PSTR(ptr) };
                let r = WSASend(sock, &[wsabuf], None, 0, Some(ovl), None);
                if r == 0 { Issue::Pending }
                else if windows::Win32::Networking::WinSock::WSAGetLastError() == windows::Win32::Networking::WinSock::WSA_IO_PENDING { Issue::Pending }
                else { Issue::Failed(windows::Win32::Foundation::WIN32_ERROR(windows::Win32::Networking::WinSock::WSAGetLastError().0 as u32).to_hresult()) }
            } else {
                match WriteFile(handle, Some(std::slice::from_raw_parts(ptr, len as usize)), None, Some(ovl)) {
                    Ok(()) => Issue::Pending,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            }
        }))
    }

    fn FlushAsync(&self) -> Result<IAsyncOperation<bool>> {
        Ok(IAsyncOperation::<bool>::spawn(|| Ok(true)))
    }
}
```

Add `is_socket(&self) -> bool` getter to `IoReg` (returns `self.is_socket`), and `pub mod stream;` to `mod.rs`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test comio::stream::tests::stream_read_write_roundtrip`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/comio/stream.rs src/comio/tpio.rs src/comio/mod.rs
git commit -m "feat(comio): OverlappedDuplexStream (IInputStream/IOutputStream/IClosable)"
```

---

### Task 6: `pipe` — NamedPipeListener / NamedPipeClient

**Files:**
- Create: `src/comio/pipe.rs`
- Modify: `src/comio/mod.rs` (add `pub mod pipe;`)
- Test: inline in `src/comio/pipe.rs`

**Interfaces:**
- Produces:
  - `pub struct NamedPipeListener { name: HSTRING }` with `pub fn create(name: &str) -> Self` and `pub async fn accept(&self) -> Result<IInspectable /*stream*/>` (creates an overlapped pipe instance, `ConnectNamedPipe` overlapped via `tpio::submit`, returns `stream::from_reg`).
  - `pub struct NamedPipeClient;` with `pub async fn connect(name: &str) -> Result<IInspectable>` (`CreateFileW` with `FILE_FLAG_OVERLAPPED`, wrap in `from_reg`).

**Design notes:**
- `accept`: `CreateNamedPipeW(name, PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED, PIPE_TYPE_BYTE | PIPE_WAIT, PIPE_UNLIMITED_INSTANCES, 4096, 4096, 0, None)`, register with `IoReg`, then a small helper future that `tpio::submit`s `ConnectNamedPipe(handle, ovl)`. `ConnectNamedPipe` returning `ERROR_PIPE_CONNECTED` synchronously = already connected (treat as immediate success); `ERROR_IO_PENDING` = pending. Wrap the connect as an `IoAsyncOp`-style op OR a one-shot channel future; simplest: expose a tiny `connect_future` returning `IAsyncAction`-like — but to keep it uniform, reuse `IoAsyncOp::<u32>::start_write` semantics is wrong; instead add a minimal `tpio`-level `await_op(reg, issue) -> impl Future` in `pipe.rs` using a std oneshot + the boxed completion. Provide it inline.
- `connect`: `CreateFileW` OPEN_EXISTING with overlapped flag; if `ERROR_PIPE_BUSY`, call `WaitNamedPipeW` then retry (document; a single retry is enough for tests).

- [ ] **Step 1: Write the failing test** (named pipe echo through the stream API, driven by `tp::block_on` + a spawned thread for the server)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::buffer::{new_buffer, buffer_ptr};
    use crate::tp::block_on;
    use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
    use windows_core::Interface;

    #[test]
    fn named_pipe_echo() {
        let name = format!(r"\\.\pipe\comio_pipe_echo_{}", std::process::id());
        let sname = name.clone();
        let server = std::thread::spawn(move || {
            let listener = NamedPipeListener::create(&sname);
            let stream = block_on(listener.accept()).unwrap();
            let input: IInputStream = stream.cast().unwrap();
            let output: IOutputStream = stream.cast().unwrap();
            let rbuf = new_buffer(16).unwrap();
            let got: IBuffer = block_on(input.ReadAsync(&rbuf.cast().unwrap(), 16, InputStreamOptions(0))).unwrap()).unwrap();
            let n = got.Length().unwrap();
            block_on(output.WriteAsync(&got).unwrap()).unwrap();
            n
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let client = block_on(NamedPipeClient::connect(&name)).unwrap();
        let c_out: IOutputStream = client.cast().unwrap();
        let c_in: IInputStream = client.cast().unwrap();
        let wbuf = new_buffer(5).unwrap();
        unsafe { buffer_ptr(&wbuf.clone().into()).unwrap().copy_from_nonoverlapping(b"hello".as_ptr(), 5); }
        wbuf.SetLength(5).unwrap();
        block_on(c_out.WriteAsync(&wbuf.cast().unwrap()).unwrap()).unwrap();
        let rbuf = new_buffer(5).unwrap();
        let echoed: IBuffer = block_on(c_in.ReadAsync(&rbuf.cast().unwrap(), 5, InputStreamOptions(0)).unwrap()).unwrap();
        unsafe { assert_eq!(std::slice::from_raw_parts(buffer_ptr(&echoed).unwrap(), 5), b"hello"); }
        assert_eq!(server.join().unwrap(), 5);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test comio::pipe::tests::named_pipe_echo`
Expected: FAIL — `NamedPipeListener` not defined.

- [ ] **Step 3: Implement `pipe.rs`**

Include the inline `await_op` helper (oneshot + boxed completion), `NamedPipeListener::{create, accept}`, `NamedPipeClient::connect`. Use `CreateNamedPipeW`/`ConnectNamedPipe`/`CreateFileW`/`WaitNamedPipeW` with the exact flags in the design notes. `accept` handles `ERROR_PIPE_CONNECTED` as immediate success (skip the op) and returns `stream::from_reg(reg)`.

(Write the full bodies here during implementation; the interfaces above and the design notes fully determine them. Key literal: `await_op` builds a `futures`-free oneshot via `std::sync::mpsc` is not `Future`; instead implement a tiny `struct ConnectOp(Arc<Mutex<(Option<...>, Option<Waker>)>>)` future whose completion closure fills the slot and wakes — same shape as `asyncop` but returning `()`/`Result<()>`.)

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test comio::pipe::tests::named_pipe_echo`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/comio/pipe.rs src/comio/mod.rs
git commit -m "feat(comio): NamedPipeListener/NamedPipeClient over overlapped streams"
```

---

### Task 7: `unix` — UnixStreamListener / UnixStreamSocket (AF_UNIX)

**Files:**
- Create: `src/comio/unix.rs`
- Modify: `src/comio/mod.rs` (add `pub mod unix;`)
- Test: inline in `src/comio/unix.rs`

**Interfaces:**
- Produces:
  - `pub struct UnixStreamListener { sock: SOCKET }` with `pub fn bind(path: &str) -> Result<Self>` (socket AF_UNIX + `bind` SOCKADDR_UN + `listen`) and `pub async fn accept(&self) -> Result<IInspectable>` (`AcceptEx` overlapped, `SO_UPDATE_ACCEPT_CONTEXT`, `stream::from_reg` with `is_socket = true`).
  - `pub struct UnixStreamSocket;` with `pub async fn connect(path: &str) -> Result<IInspectable>` (bind to autobind, `ConnectEx`, `SO_UPDATE_CONNECT_CONTEXT`).

**Design notes:**
- Reuse `src/tp/io/net.rs` patterns for `WSASocketW`, `AcceptEx`, `ConnectEx` (WSAIoctl fetch), `SO_UPDATE_*_CONTEXT`, but with `AF_UNIX` and `SOCKADDR_UN { sun_family: ADDRESS_FAMILY(AF_UNIX), sun_path }` where `sun_path` is the NUL-terminated path bytes (as `i8`). Address length passed to `bind`/`AcceptEx` is `offsetof(sun_path) + path_len + 1`.
- Reuse the `wsa_init()` Once from a shared spot; simplest is to call `crate::tp::io::net`'s init — but that's private. Add a local `wsa_init()` in `unix.rs` (dup of the tiny `WSAStartup` Once).
- `accept`: create AF_UNIX accept socket, `AcceptEx` via the same `await_op` helper shape as `pipe.rs` (factor it into `tpio.rs` as `pub(crate) async fn await_op`? No — keep the future type in a shared `comio::op` module). **Refactor:** move the `ConnectOp`-style oneshot future from Task 6 into `src/comio/waitop.rs` as `pub(crate) fn await_io(reg: &Arc<IoReg>, issue) -> impl Future<Output = Result<usize>>`, and have both `pipe.rs` and `unix.rs` use it. Do this refactor as Step 3 here (move + adjust `pipe.rs` import) so there is exactly one implementation.

- [ ] **Step 1: Write the failing test** (AF_UNIX echo)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::buffer::{new_buffer, buffer_ptr};
    use crate::tp::block_on;
    use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
    use windows_core::Interface;

    #[test]
    fn af_unix_echo() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("comio_unix_{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let p = path.to_string_lossy().into_owned();
        let sp = p.clone();
        let server = std::thread::spawn(move || {
            let l = UnixStreamListener::bind(&sp).unwrap();
            let s = block_on(l.accept()).unwrap();
            let input: IInputStream = s.cast().unwrap();
            let output: IOutputStream = s.cast().unwrap();
            let rbuf = new_buffer(16).unwrap();
            let got: IBuffer = block_on(input.ReadAsync(&rbuf.cast().unwrap(), 16, InputStreamOptions(0)).unwrap()).unwrap();
            block_on(output.WriteAsync(&got).unwrap()).unwrap();
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let c = block_on(UnixStreamSocket::connect(&p)).unwrap();
        let c_out: IOutputStream = c.cast().unwrap();
        let c_in: IInputStream = c.cast().unwrap();
        let wbuf = new_buffer(4).unwrap();
        unsafe { buffer_ptr(&wbuf.clone().into()).unwrap().copy_from_nonoverlapping(b"ping".as_ptr(), 4); }
        wbuf.SetLength(4).unwrap();
        block_on(c_out.WriteAsync(&wbuf.cast().unwrap()).unwrap()).unwrap();
        let rbuf = new_buffer(4).unwrap();
        let echoed: IBuffer = block_on(c_in.ReadAsync(&rbuf.cast().unwrap(), 4, InputStreamOptions(0)).unwrap()).unwrap();
        unsafe { assert_eq!(std::slice::from_raw_parts(buffer_ptr(&echoed).unwrap(), 4), b"ping"); }
        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test comio::unix::tests::af_unix_echo`
Expected: FAIL — `UnixStreamListener` not defined.

- [ ] **Step 3: Refactor the wait-op future into `waitop.rs`, then implement `unix.rs`**

Create `src/comio/waitop.rs` with `pub(crate) fn await_io(reg: &Arc<IoReg>, issue: impl FnOnce(*mut OVERLAPPED) -> Issue + Send + 'static) -> impl Future<Output = windows_core::Result<usize>>` (a `Mutex<(Option<Result<usize>>, Option<Waker>)>` shared block; the boxed completion fills it and wakes; poll returns Ready when present). Update `pipe.rs` to import `await_io` from `waitop` instead of its local copy. Add `pub mod waitop;`. Then implement `unix.rs` using `await_io` for `AcceptEx`/`ConnectEx`, following `tp/io/net.rs`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test comio::unix::tests::af_unix_echo` and `cargo test comio::pipe::tests::named_pipe_echo`
Expected: both PASS.

- [ ] **Step 5: Commit**

```bash
git add src/comio/unix.rs src/comio/waitop.rs src/comio/pipe.rs src/comio/mod.rs
git commit -m "feat(comio): AF_UNIX listener/socket + shared await_io helper"
```

---

### Task 8: Interop test + cancel-storm + public re-exports

**Files:**
- Modify: `src/comio/mod.rs` (re-exports; interop + cancel tests)

**Interfaces:**
- Produces: `pub use pipe::{NamedPipeClient, NamedPipeListener}; pub use unix::{UnixStreamListener, UnixStreamSocket}; pub use stream::from_reg;` (and any types demos need).

- [ ] **Step 1: Write the interop + cancel tests**

Add to `src/comio/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use crate::tp::block_on;
    // (a) DataReader/DataWriter interop over our stream.
    #[test]
    fn datareader_interop() {
        // Build a pipe pair, wrap client with DataWriter and server with DataReader,
        // WriteString / LoadAsync+ReadString a short string, assert equality.
        // Uses windows::Storage::Streams::{DataReader, DataWriter}.
    }
    // (b) Cancel storm: start many reads with no writer and drop the futures/ops;
    // assert comio::tpio::LIVE_OPS returns to 0 (debug).
    #[test]
    fn cancel_storm() {
        // Open a pipe pair; loop 1000x: create a read op future and drop it immediately.
        // Then drain and assert LIVE_OPS == 0.
    }
}
```

Fill both bodies with concrete code following the same construction patterns as Tasks 6–7 (DataReader via `DataReader::CreateDataReader(input_stream)`; DataWriter via `DataWriter::CreateDataWriter(output_stream)`, `WriteString`, `StoreAsync().await`).

- [ ] **Step 2: Run to verify they fail (then implement/adjust)**

Run: `cargo test comio::tests`
Expected: initially FAIL (unwritten bodies), then PASS after filling in.

- [ ] **Step 3: Add re-exports**

Add the `pub use` lines to `src/comio/mod.rs`.

- [ ] **Step 4: Full suite**

Run: `cargo test comio::`
Expected: all comio tests PASS. Then `cargo test` (whole crate incl. `tp`) PASS.

- [ ] **Step 5: Commit**

```bash
git add src/comio/mod.rs
git commit -m "feat(comio): DataReader interop + cancel-storm tests + re-exports"
```

---

## Self-Review

- **Spec coverage:** OverlappedDuplexStream (Task 5), custom IAsyncOperationWithProgress via TP_IO pcbe=NULL (Tasks 2,4), IBuffer/IBufferByteAccess (Task 3), NamedPipe (Task 6), AF_UNIX (Task 7), DataReader interop + cancel storm (Task 8), Cargo features (Task 1). WinRT TCP/UDP/file/HTTP are out of scope by spec (awaited directly). ✔
- **Type consistency:** `IoReg`, `submit`, `Issue`, `OpHandle` (Task 2) consumed unchanged in Tasks 4–7; `IoAsyncOp::start_read/start_write` (Task 4) consumed in Task 5; `from_reg` (Task 5) consumed in Tasks 6–7; `await_io` centralized in Task 7. ✔
- **Known risk flagged inline:** whether `#[implement]` resolves the parameterized async IID (Task 4 Step 4 note) — validated by that task's test before anything builds on it. `WIN32_ERROR::from_error` fallback noted (Task 2 Step 3).
- **Placeholders:** Tasks 6–8 leave some full bodies to implementation but pin exact APIs, flags, and shapes; acceptable given the determinism of the interfaces. If executing via subagents, the interface blocks + design notes are sufficient.
