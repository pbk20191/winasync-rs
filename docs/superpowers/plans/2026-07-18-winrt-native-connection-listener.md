# WinRT-native `ConnectionListener` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a WinRT-native, `StreamSocketListener`-shaped COM listener to `src/comio/`: a real `windows_core::Event`-backed `ConnectionReceived` event delivering accepted connections (named pipe + AF_UNIX) via a custom fixed-IID event-args interface, with proper `IClosable`/`RO_E_CLOSED` semantics.

**Architecture:** New `src/comio/listener.rs` layered on the existing transports. A custom `#[interface]` `IConnectionReceivedArgs` (fixed GUID) exposes the accepted `OverlappedDuplexStream`; a `#[implement(IClosable)]` `ConnectionListener` backs `ConnectionReceived` with `windows_core::Event<TypedEventHandler<IInspectable,IInspectable>>` and runs a self-driving accept loop on a crate-shared `tp::Runtime`. `Close` sets a `closed` flag; post-close member calls return `RO_E_CLOSED`. The existing `OverlappedDuplexStream::Close` is upgraded from a no-op to the same closed-flag semantics.

**Tech Stack:** Rust edition 2024, `windows` 0.62.2, `windows-core` 0.62.2 (`#[interface]`, `#[implement]`, `Event`, `TypedEventHandler`), `windows-future`, existing `src/comio/*` and `src/tp/` (same crate).

## Global Constraints

- Windows only; official `windows`/`windows-core`/`windows-future` crates only.
- **Never hand-implement `IUnknown` (`QueryInterface`/`AddRef`/`Release`) or `IInspectable`** — `#[implement]` generates them (refcounted). Implement only custom methods.
- **Agility:** keep every field of every `#[implement]`/`#[interface]` object `Send + Sync` (`Arc`-wrapped handles, atomics, `windows_core::Event`); NO `Rc`/`!Send` fields; never pass `Agile(false)`. `#[implement]` is agile by default.
- **`RO_E_CLOSED` = `HRESULT(0x80000013u32 as i32)`**: every public member of a closed listener, and `ReadAsync`/`WriteAsync` of a closed stream, returns `Err(windows_core::Error::from_hresult(RO_E_CLOSED))`. `Close` is idempotent.
- No panic may cross a COM/FFI boundary (`#[implement]`/`#[interface]` methods, the event fire, the accept loop's event callback).
- The accept loop fires `ConnectionReceived` on a `tp` pool thread (matching WinRT `StreamSocketListener`). It must be cancel-safe: aborting it drops the in-flight `accept()` future, which `CancelIoEx`s the overlapped op (existing behavior).
- Commit after every task. Never use `--no-verify`.

## Reference facts (verified in crate sources)

- Custom interface pattern (windows-interface `lib.rs` example):
  ```rust
  #[interface("094d70d6-5202-44b8-abb8-43860da5aca2")]
  unsafe trait IValue: IUnknown { fn GetValue(&self, value: *mut i32) -> HRESULT; }
  #[implement(IValue)] struct Value(i32);
  impl IValue_Impl for Value_Impl { unsafe fn GetValue(&self, value: *mut i32) -> HRESULT { *value = self.0; HRESULT(0) } }
  let object: IValue = Value(123).into();
  ```
- `TypedEventHandler::new(F)` where `F: Fn(Ref<TSender>, Ref<TResult>) -> Result<()> + Send + 'static` (Foundation/mod.rs:2502). Caller side: `handler.Invoke(sender, args)` (`P: Param<..>`, accepts `None` for a null sender).
- `windows_core::Event<T: Interface>`: `new()`, `add(&T) -> Result<i64>`, `remove(i64)`, `clear()`, `call(|d: &T| -> Result<()> {..})` (`event.rs`). Send+Sync, agile-aware.
- Existing to extend/reuse:
  - `src/comio/stream.rs`: `#[implement(IInputStream, IOutputStream, windows::Foundation::IClosable)] pub struct OverlappedDuplexStream { reg: Arc<IoReg> }`; `pub(crate) fn from_reg(reg: Arc<IoReg>) -> windows_core::IInspectable`; `IClosable_Impl::Close`, `IInputStream_Impl::ReadAsync`, `IOutputStream_Impl::WriteAsync` on `OverlappedDuplexStream_Impl`.
  - `src/comio/pipe.rs`: `NamedPipeListener::create(name: &str) -> Self`, `async fn accept(&self) -> Result<IInspectable>`; `NamedPipeClient::connect(name: &str) -> Result<IInspectable>` (async).
  - `src/comio/unix.rs`: `UnixStreamListener::bind(path: &str) -> Result<Self>`, `async fn accept(&self) -> Result<IInspectable>`; `UnixStreamSocket::connect(path) -> Result<IInspectable>`.
  - `src/comio/tpio.rs`: `pub(crate) static LIVE_OPS: AtomicUsize` (debug).
  - `src/tp/`: `tp::Runtime` (`spawn<F: Future + Send + 'static>(fut) -> JoinHandle<F::Output>` where `Output: Send + 'static`; `JoinHandle::abort()`), `tp::block_on`.
  - `windows::Storage::Streams::{IInputStream, IOutputStream, IBuffer, InputStreamOptions}`, `windows_core::Interface` (`.cast()`).

---

### Task 1: `IConnectionReceivedArgs` custom interface + `ConnectionReceivedArgs` object

**Files:**
- Create: `src/comio/listener.rs`
- Modify: `src/comio/mod.rs` (add `pub mod listener;`)
- Test: inline in `src/comio/listener.rs`

**Interfaces:**
- Produces:
  - `#[interface("<GUID1>")] pub(crate) unsafe trait IConnectionReceivedArgs: IUnknown { fn Stream(&self, result: *mut *mut c_void) -> HRESULT; fn RemoteEndpoint(&self, result: *mut core::mem::ManuallyDrop<HSTRING>) -> HRESULT; }`
  - `pub(crate) fn new_args(stream: IInspectable, endpoint: HSTRING) -> IConnectionReceivedArgs`
  - `RO_E_CLOSED: windows_core::HRESULT` const.

**Design notes:**
- Generate ONE fresh GUID for `<GUID1>` (any UUID; freeze it as the literal in the `#[interface]` attribute). Do not reuse an existing IID.
- `ConnectionReceivedArgs` is `#[implement(IConnectionReceivedArgs)]` holding `{ stream: IInspectable, endpoint: HSTRING }` (both agile/Send+Sync). Its `Stream()` writes a cloned `IInspectable` out-pointer; `RemoteEndpoint()` writes a cloned `HSTRING`. Follow the raw-ABI out-param style from the reference example (write via the out pointer, return `HRESULT(0)`; guard against panics — the bodies here are infallible clones so just don't `unwrap`).
- Constructing: `let obj: IConnectionReceivedArgs = ConnectionReceivedArgs { stream, endpoint }.into();`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::buffer::new_buffer;
    use windows_core::Interface;

    #[test]
    fn args_exposes_stream_and_endpoint() {
        // Any IInspectable stands in for the accepted stream here.
        let stream_stub: windows_core::IInspectable = new_buffer(4).unwrap().into();
        let args = new_args(stream_stub.clone(), windows_core::HSTRING::from("pipe://x"));
        // Stream() returns the same object identity.
        let got = unsafe {
            let mut raw = core::ptr::null_mut();
            assert!(args.Stream(&mut raw).is_ok());
            windows_core::IInspectable::from_raw(raw)
        };
        assert_eq!(got, stream_stub);
        let ep = unsafe {
            let mut s = core::mem::ManuallyDrop::new(windows_core::HSTRING::new());
            assert!(args.RemoteEndpoint(&mut s).is_ok());
            core::mem::ManuallyDrop::into_inner(s)
        };
        assert_eq!(ep.to_string(), "pipe://x");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test comio::listener::tests::args_exposes_stream_and_endpoint`
Expected: FAIL — `new_args`/`IConnectionReceivedArgs` not defined.

- [ ] **Step 3: Implement the interface + args object**

```rust
//! WinRT-native StreamSocketListener-shaped listener over comio transports.
#![allow(dead_code)]

use core::ffi::c_void;
use windows_core::{implement, interface, HSTRING, HRESULT, IInspectable, IUnknown, Interface};

/// RO_E_CLOSED — returned by members of a closed listener/stream.
pub(crate) const RO_E_CLOSED: HRESULT = HRESULT(0x80000013u32 as i32);

#[interface("<GENERATE-A-FRESH-GUID>")]
pub(crate) unsafe trait IConnectionReceivedArgs: IUnknown {
    fn Stream(&self, result: *mut *mut c_void) -> HRESULT;
    fn RemoteEndpoint(&self, result: *mut core::mem::ManuallyDrop<HSTRING>) -> HRESULT;
}

#[implement(IConnectionReceivedArgs)]
struct ConnectionReceivedArgs {
    stream: IInspectable,
    endpoint: HSTRING,
}

impl IConnectionReceivedArgs_Impl for ConnectionReceivedArgs_Impl {
    unsafe fn Stream(&self, result: *mut *mut c_void) -> HRESULT {
        if result.is_null() {
            return HRESULT(-2147467261i32); // E_POINTER
        }
        unsafe { *result = self.stream.clone().into_raw() };
        HRESULT(0)
    }
    unsafe fn RemoteEndpoint(&self, result: *mut core::mem::ManuallyDrop<HSTRING>) -> HRESULT {
        if result.is_null() {
            return HRESULT(-2147467261i32);
        }
        unsafe { *result = core::mem::ManuallyDrop::new(self.endpoint.clone()) };
        HRESULT(0)
    }
}

pub(crate) fn new_args(stream: IInspectable, endpoint: HSTRING) -> IConnectionReceivedArgs {
    ConnectionReceivedArgs { stream, endpoint }.into()
}
```
Add `pub mod listener;` to `src/comio/mod.rs`. (Adjust the exact out-param types if the `#[interface]` macro expects a different `HSTRING` ABI shape — match whatever compiles against windows-core 0.62.2; `*mut *mut c_void` for the interface out and cloning `into_raw()` is the canonical pattern.)

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test comio::listener::tests::args_exposes_stream_and_endpoint`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/comio/listener.rs src/comio/mod.rs
git commit -m "feat(comio): IConnectionReceivedArgs custom interface + args object"
```

---

### Task 2: `OverlappedDuplexStream` Close upgrade → `RO_E_CLOSED`

**Files:**
- Modify: `src/comio/stream.rs`
- Test: inline in `src/comio/stream.rs`

**Interfaces:**
- Consumes: `crate::comio::listener::RO_E_CLOSED`.
- Produces: `OverlappedDuplexStream` gains a `closed: AtomicBool`; `Close` sets it; `ReadAsync`/`WriteAsync` return `RO_E_CLOSED` after close.

**Design notes:**
- Add `closed: std::sync::atomic::AtomicBool` to the `#[implement]` struct (initialize `false` in `from_reg`). `AtomicBool` is Send+Sync — agility preserved.
- `IClosable_Impl::Close`: `self.closed.store(true, Release); Ok(())` (idempotent). Keep the existing doc note that in-flight ops keep their own `Arc<IoReg>` and complete; only NEW ops are refused.
- At the TOP of `ReadAsync` and `WriteAsync`: `if self.closed.load(Acquire) { return Err(windows_core::Error::from_hresult(RO_E_CLOSED)); }`.

- [ ] **Step 1: Write the failing test** (append to `stream.rs` tests)

```rust
#[test]
fn read_write_after_close_returns_ro_e_closed() {
    use crate::comio::listener::RO_E_CLOSED;
    use crate::comio::tpio::IoReg;
    use crate::comio::buffer::new_buffer;
    use windows::Storage::Streams::{IInputStream, IOutputStream, IBuffer, InputStreamOptions};
    use windows::Foundation::IClosable;
    use windows_core::Interface;
    // Build a stream over one end of a pipe pair (reuse the existing test setup helper pattern).
    let (a, _b) = super::tests::pipe_pair(); // a: IInspectable stream over the server end
    IClosable::from(&a).unwrap_or_else(|_| a.cast().unwrap()); // ensure it exposes IClosable
    let closable: IClosable = a.cast().unwrap();
    closable.Close().unwrap();
    let inp: IInputStream = a.cast().unwrap();
    let outp: IOutputStream = a.cast().unwrap();
    let rb = new_buffer(4).unwrap();
    let rerr = inp.ReadAsync(&rb.cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0)).unwrap_err();
    assert_eq!(rerr.code(), RO_E_CLOSED);
    let werr = outp.WriteAsync(&rb.cast::<IBuffer>().unwrap()).unwrap_err();
    assert_eq!(werr.code(), RO_E_CLOSED);
}
```
If `stream.rs`'s existing tests have no reusable `pipe_pair()` helper, add a small one in the `tests` module that returns two `from_reg`-wrapped `IInspectable` streams over a `CreateNamedPipeW`/`CreateFileW` overlapped pair (mirror the setup already in `stream_read_write_roundtrip`). Also keep a positive test that a read issued BEFORE close still resolves (reuse/adapt the existing roundtrip test — do not delete it).

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test comio::stream::tests::read_write_after_close_returns_ro_e_closed`
Expected: FAIL — reads/writes currently succeed (Close is a no-op).

- [ ] **Step 3: Implement the closed flag + checks**

In `src/comio/stream.rs`:
- struct: add `closed: std::sync::atomic::AtomicBool,`
- `from_reg`: construct with `closed: std::sync::atomic::AtomicBool::new(false)`.
- `Close`: `self.closed.store(true, std::sync::atomic::Ordering::Release); Ok(())`.
- first line of `ReadAsync` and of `WriteAsync`:
  ```rust
  if self.closed.load(std::sync::atomic::Ordering::Acquire) {
      return Err(windows_core::Error::from_hresult(crate::comio::listener::RO_E_CLOSED));
  }
  ```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test comio::stream -- --nocapture`
Expected: PASS (new close test + the preserved roundtrip test).

- [ ] **Step 5: Commit**

```bash
git add src/comio/stream.rs
git commit -m "feat(comio): OverlappedDuplexStream Close now refuses new ops with RO_E_CLOSED"
```

---

### Task 3: `ConnectionListener` (named pipe) — event source, accept loop, Close

**Files:**
- Modify: `src/comio/listener.rs`, `src/comio/mod.rs` (shared runtime)
- Test: inline in `src/comio/listener.rs`

**Interfaces:**
- Consumes: `IConnectionReceivedArgs`/`new_args`/`RO_E_CLOSED` (Task 1), `NamedPipeListener` (pipe.rs), `tp::Runtime`.
- Produces:
  - `pub(crate) fn shared_runtime() -> &'static crate::tp::Runtime` in `src/comio/mod.rs` (lazy `OnceLock`).
  - `#[implement(windows::Foundation::IClosable)] pub struct ConnectionListener` with:
    - `pub fn named_pipe() -> ConnectionListener` (constructor returning the projected type; store transport = pipe)
    - `pub fn ConnectionReceived(&self, handler: &TypedEventHandler<IInspectable, IInspectable>) -> Result<i64>`
    - `pub fn RemoveConnectionReceived(&self, token: i64) -> Result<()>`
    - `pub fn BindAsync(&self, endpoint: &HSTRING) -> Result<windows_future::IAsyncAction>`
    - `IClosable::Close`
  - all guarded by `RO_E_CLOSED` when closed.

**Design notes:**
- `ListenerState` (in an `Arc`, held by the object and the loop):
  ```rust
  struct ListenerState {
      closed: AtomicBool,
      event: windows_core::Event<TypedEventHandler<IInspectable, IInspectable>>,
      endpoint: Mutex<Option<HSTRING>>,      // set at BindAsync (pipe name)
      task: Mutex<Option<crate::tp::JoinHandle<()>>>, // accept-loop handle; taken/aborted on Close
  }
  ```
  `ConnectionListener` holds `{ state: Arc<ListenerState>, transport: Transport }` where `enum Transport { NamedPipe }` (AF_UNIX added in Task 4). All fields Send+Sync (agility ok).
- `ConnectionReceived`/`RemoveConnectionReceived`/`BindAsync`: first check `state.closed` → `RO_E_CLOSED`. `ConnectionReceived` = `state.event.add(handler.into())`... use `state.event.add(&handler)` (handler is `&TypedEventHandler<..>`; `Event::add` takes `&T`).
- `BindAsync`: store `endpoint`; spawn the accept loop on `shared_runtime()`; return an **already-completed** `IAsyncAction` (`windows_future::IAsyncAction::spawn(|| Ok(()))`) — binding for a pipe is just recording the name (the loop creates instances). Store the returned `JoinHandle` in `state.task`.
- **Accept loop** (a `Send + 'static` async block spawned on `shared_runtime()`):
  ```
  loop {
      if state.closed.load(Acquire) { break; }
      let listener = NamedPipeListener::create(&name);   // instance-per-accept
      match listener.accept().await {
          Ok(stream) => {
              if state.closed.load(Acquire) { break; }
              let args: IInspectable = new_args(stream, HSTRING::from(&name)).cast().unwrap();
              // fire on this pool thread; null sender (connection is in args)
              state.event.call(|h| unsafe { h.Invoke(None, &args) });
          }
          Err(_) => { if state.closed.load(Acquire) { break; } /* transient; continue */ }
      }
  }
  ```
  Wrap the `event.call` body so a panicking consumer handler cannot unwind across the pool-thread boundary: `let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state.event.call(...)));`. (Each iteration creates a fresh instance so multiple sequential clients connect; a brief window exists between instances — the test's client uses a short connect retry.)
- `Close`: idempotent — `state.closed.store(true, Release)`; `if let Some(h) = state.task.lock().unwrap().take() { h.abort(); }` (aborting drops the in-flight `accept()`, cancelling the overlapped op); `state.event.clear()`; `Ok(())`.
- Note the `new_args` return is `IConnectionReceivedArgs`; cast to `IInspectable` for the event arg (the object is IInspectable). The consumer's handler `.cast::<IConnectionReceivedArgs>()` then calls `Stream()`.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod listener_tests {
    use super::*;
    use crate::comio::buffer::{new_buffer, buffer_ptr};
    use crate::tp::block_on;
    use std::sync::mpsc;
    use windows::Foundation::{IClosable, TypedEventHandler};
    use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
    use windows_core::{IInspectable, Interface, HSTRING};

    #[test]
    fn connection_received_fires_with_working_stream() {
        let name = format!(r"\\.\pipe\comio_listener_{}", std::process::id());
        let listener = ConnectionListener::named_pipe();

        let (tx, rx) = mpsc::channel::<IInspectable>();
        let handler = TypedEventHandler::<IInspectable, IInspectable>::new(move |_sender, args| {
            // args -> IConnectionReceivedArgs -> Stream()
            let a = args.as_ref().unwrap();
            let cra: IConnectionReceivedArgs = a.cast().unwrap();
            let stream = unsafe {
                let mut raw = core::ptr::null_mut();
                cra.Stream(&mut raw).ok()?;
                IInspectable::from_raw(raw)
            };
            let _ = tx.send(stream);
            Ok(())
        });
        listener.ConnectionReceived(&handler).unwrap();
        block_on(listener.BindAsync(&HSTRING::from(&name))).unwrap();

        // Client connects (short retry to cover the inter-instance window).
        let client = {
            let mut last = None;
            for _ in 0..50 {
                match block_on(crate::comio::NamedPipeClient::connect(&name)) {
                    Ok(c) => { last = Some(c); break; }
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
                }
            }
            last.expect("client failed to connect")
        };

        // Server-side stream delivered via the event; echo a payload client->server->client.
        let server = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("no ConnectionReceived");
        let c_out: IOutputStream = client.cast().unwrap();
        let c_in: IInputStream = client.cast().unwrap();
        let s_in: IInputStream = server.cast().unwrap();
        let s_out: IOutputStream = server.cast().unwrap();

        let wb = new_buffer(4).unwrap();
        unsafe { buffer_ptr(&wb.clone().into()).unwrap().copy_from_nonoverlapping(b"echo".as_ptr(), 4); }
        wb.SetLength(4).unwrap();
        block_on(c_out.WriteAsync(&wb.cast::<IBuffer>().unwrap()).unwrap()).unwrap();
        let got: IBuffer = block_on(s_in.ReadAsync(&new_buffer(4).unwrap().cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0)).unwrap()).unwrap();
        block_on(s_out.WriteAsync(&got).unwrap()).unwrap();
        let echoed: IBuffer = block_on(c_in.ReadAsync(&new_buffer(4).unwrap().cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0)).unwrap()).unwrap();
        unsafe { assert_eq!(std::slice::from_raw_parts(buffer_ptr(&echoed).unwrap(), 4), b"echo"); }

        IClosable::from(&listener).Close().unwrap();
    }

    #[test]
    fn members_after_close_return_ro_e_closed() {
        let listener = ConnectionListener::named_pipe();
        IClosable::from(&listener).Close().unwrap();
        let handler = TypedEventHandler::<IInspectable, IInspectable>::new(|_, _| Ok(()));
        assert_eq!(listener.ConnectionReceived(&handler).unwrap_err().code(), RO_E_CLOSED);
        assert_eq!(listener.BindAsync(&HSTRING::from("x")).unwrap_err().code(), RO_E_CLOSED);
        // Second Close is a no-op.
        IClosable::from(&listener).Close().unwrap();
    }
}
```
Note: `IClosable::from(&listener)` requires `ConnectionListener::named_pipe()` to return the projected `ConnectionListener` interface object (from `.into()`), so `IClosable`/the inherent methods are both reachable. If calling inherent methods (`ConnectionReceived`) on the projected type is awkward, expose them as methods on a thin wrapper OR keep `named_pipe()` returning the concrete projected type and cast to `IClosable` for `Close`. Make the ergonomics compile; the behavior above is what matters.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test comio::listener::listener_tests`
Expected: FAIL — `ConnectionListener` not defined.

- [ ] **Step 3: Implement `shared_runtime` + `ConnectionListener`**

Add to `src/comio/mod.rs`:
```rust
pub(crate) fn shared_runtime() -> &'static crate::tp::Runtime {
    use std::sync::OnceLock;
    static RT: OnceLock<crate::tp::Runtime> = OnceLock::new();
    RT.get_or_init(crate::tp::Runtime::new)
}
```
Implement `ListenerState`, `Transport`, `ConnectionListener` (`#[implement(windows::Foundation::IClosable)]`), its inherent methods (`named_pipe`, `ConnectionReceived`, `RemoveConnectionReceived`, `BindAsync`), and `IClosable_Impl::Close`, per the design notes. Spawn the accept loop via `crate::comio::shared_runtime().spawn(async move { ... })`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test comio::listener -- --nocapture`
Expected: PASS. If the projected-type ergonomics fight you, adjust construction (return the concrete `ConnectionListener` projected object and route `Close` through an `IClosable` cast) until both tests compile and pass.

- [ ] **Step 5: Commit**

```bash
git add src/comio/listener.rs src/comio/mod.rs
git commit -m "feat(comio): ConnectionListener (named pipe) with ConnectionReceived event + RO_E_CLOSED Close"
```

---

### Task 4: AF_UNIX transport in `ConnectionListener`

**Files:**
- Modify: `src/comio/listener.rs`
- Test: inline

**Interfaces:**
- Consumes: `UnixStreamListener` (unix.rs).
- Produces: `pub fn unix() -> ConnectionListener`; the accept loop handles `Transport::Unix`.

**Design notes:**
- Extend `enum Transport { NamedPipe, Unix }`. For `Unix`, `BindAsync` must `UnixStreamListener::bind(path)?` ONCE (persistent listen socket) and move that listener into the loop (store it in `ListenerState`, e.g. `unix_listener: Mutex<Option<Arc<UnixStreamListener>>>` — but `UnixStreamListener` isn't `Arc`/Clone; simplest: create it in `BindAsync` and `move` it into the spawned loop closure rather than into shared state). The loop calls `unix_listener.accept().await` in a loop on the SAME socket.
- Because the unix listen socket is created in `BindAsync` and moved into the loop, `BindAsync` for unix returns its completed action only after `bind` succeeds (so bind errors surface). For the pipe transport keep Task 3's behavior.
- Same event fire + close/abort semantics.
- `UnixStreamListener::bind` requires the socket file not pre-exist; the test removes it first.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn unix_connection_received_echo() {
    use crate::comio::buffer::{new_buffer, buffer_ptr};
    use crate::tp::block_on;
    use std::sync::mpsc;
    use windows::Foundation::{IClosable, TypedEventHandler};
    use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
    use windows_core::{IInspectable, Interface, HSTRING};

    let path = std::env::temp_dir().join(format!("comio_listener_unix_{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let p = path.to_string_lossy().into_owned();

    let listener = ConnectionListener::unix();
    let (tx, rx) = mpsc::channel::<IInspectable>();
    let handler = TypedEventHandler::<IInspectable, IInspectable>::new(move |_s, args| {
        let cra: IConnectionReceivedArgs = args.as_ref().unwrap().cast().unwrap();
        let stream = unsafe { let mut raw = core::ptr::null_mut(); cra.Stream(&mut raw).ok()?; IInspectable::from_raw(raw) };
        let _ = tx.send(stream);
        Ok(())
    });
    listener.ConnectionReceived(&handler).unwrap();
    block_on(listener.BindAsync(&HSTRING::from(&p))).unwrap();

    let client = block_on(crate::comio::UnixStreamSocket::connect(&p)).unwrap();
    let server = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("no ConnectionReceived");

    let c_out: IOutputStream = client.cast().unwrap();
    let s_in: IInputStream = server.cast().unwrap();
    let s_out: IOutputStream = server.cast().unwrap();
    let c_in: IInputStream = client.cast().unwrap();
    let wb = new_buffer(4).unwrap();
    unsafe { buffer_ptr(&wb.clone().into()).unwrap().copy_from_nonoverlapping(b"unix".as_ptr(), 4); }
    wb.SetLength(4).unwrap();
    block_on(c_out.WriteAsync(&wb.cast::<IBuffer>().unwrap()).unwrap()).unwrap();
    let got: IBuffer = block_on(s_in.ReadAsync(&new_buffer(4).unwrap().cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0)).unwrap()).unwrap();
    block_on(s_out.WriteAsync(&got).unwrap()).unwrap();
    let echoed: IBuffer = block_on(c_in.ReadAsync(&new_buffer(4).unwrap().cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0)).unwrap()).unwrap();
    unsafe { assert_eq!(std::slice::from_raw_parts(buffer_ptr(&echoed).unwrap(), 4), b"unix"); }

    IClosable::from(&listener).Close().unwrap();
    let _ = std::fs::remove_file(&path);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test comio::listener::listener_tests::unix_connection_received_echo`
Expected: FAIL — `ConnectionListener::unix` not defined.

- [ ] **Step 3: Implement the unix transport branch**

Add `unix()` constructor (transport = Unix) and, in `BindAsync`, branch on transport: for Unix, `let l = UnixStreamListener::bind(&path)?;` then spawn a loop that owns `l` and calls `l.accept().await` repeatedly, firing the event as in Task 3. Same close/abort.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test comio::listener -- --nocapture`
Expected: PASS (pipe + unix + close tests).

- [ ] **Step 5: Commit**

```bash
git add src/comio/listener.rs
git commit -m "feat(comio): AF_UNIX transport for ConnectionListener"
```

---

### Task 5: Close cancels in-flight accept (leak check) + `Information`/`Control` + re-exports

**Files:**
- Modify: `src/comio/listener.rs`, `src/comio/mod.rs`
- Test: inline

**Interfaces:**
- Produces:
  - `ConnectionListener::Information(&self) -> Result<HSTRING>` (the bound endpoint; `RO_E_CLOSED` if closed) — a minimal `Information` (return the endpoint string; a full info object is YAGNI).
  - `ConnectionListener::Control(&self) -> Result<ListenerControl>` where `ListenerControl` is a small `#[implement]`-free plain struct OR omit if not needed — see note.
  - `pub use listener::{ConnectionListener, IConnectionReceivedArgs};` in `src/comio/mod.rs`.

**Design notes:**
- Keep `Control` minimal: expose `set_pipe_instances(u32)`/`set_buffer_size(u32)` on the listener BEFORE bind, stored in `ListenerState` and read by the pipe loop's `CreateNamedPipeW`. If wiring `Control` cleanly is more than a couple of methods, ship just `Information` + the setters and note `Control` as a follow-up (YAGNI) — do NOT build an elaborate control object.
- `Information()` returns the stored endpoint (`state.endpoint`), `RO_E_CLOSED` when closed.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn close_cancels_inflight_accept_no_leak() {
    use crate::tp::block_on;
    use windows::Foundation::{IClosable, TypedEventHandler};
    use windows_core::{IInspectable, HSTRING};
    let name = format!(r"\\.\pipe\comio_listener_leak_{}", std::process::id());
    let listener = ConnectionListener::named_pipe();
    let handler = TypedEventHandler::<IInspectable, IInspectable>::new(|_, _| Ok(()));
    listener.ConnectionReceived(&handler).unwrap();
    block_on(listener.BindAsync(&HSTRING::from(&name))).unwrap();
    // No client connects; the loop is parked in an in-flight ConnectNamedPipe.
    std::thread::sleep(std::time::Duration::from_millis(50));
    // Information returns the endpoint; then close.
    assert_eq!(listener.Information().unwrap().to_string(), name);
    IClosable::from(&listener).Close().unwrap();
    assert_eq!(listener.Information().unwrap_err().code(), RO_E_CLOSED);
    #[cfg(debug_assertions)]
    {
        for _ in 0..500 {
            if crate::comio::tpio::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed) == 0 { break; }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(crate::comio::tpio::LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed), 0, "listener accept leaked an op");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test comio::listener::listener_tests::close_cancels_inflight_accept_no_leak`
Expected: FAIL — `Information` not defined (and possibly a leak if abort path is wrong).

- [ ] **Step 3: Implement `Information` (+ optional `Control` setters) + re-exports**

Add `Information(&self) -> Result<HSTRING>` (closed-check → `RO_E_CLOSED`, else clone `state.endpoint`). Add `pub use listener::{ConnectionListener, IConnectionReceivedArgs};` to `mod.rs`. If adding `Control` setters, store them in `ListenerState` and read in the pipe loop; otherwise document their omission.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test comio::listener -- --nocapture` then full `cargo test -- --test-threads=1`.
Expected: all PASS; `LIVE_OPS` returns to 0 after close (no leaked accept op).

- [ ] **Step 5: Commit**

```bash
git add src/comio/listener.rs src/comio/mod.rs
git commit -m "feat(comio): ConnectionListener Information + close-cancel leak test + re-exports"
```

---

## Self-Review

- **Spec coverage:** `IConnectionReceivedArgs`+args (Task 1), Close/`RO_E_CLOSED` on stream (Task 2), `ConnectionListener` event source + `BindAsync` + `Close`/`RO_E_CLOSED` + accept loop on shared `tp::Runtime` + pool-thread event fire (Task 3), AF_UNIX (Task 4), `Information` + close-cancel leak-freedom + re-exports (Task 5). `Control` intentionally minimal/optional per YAGNI (Task 5 note). Agility & no-hand-rolled-IUnknown are Global Constraints enforced across all `#[implement]`/`#[interface]` tasks. C# `.winmd` projection explicitly out of scope. ✔
- **Type consistency:** `IConnectionReceivedArgs`/`new_args`/`RO_E_CLOSED` (T1) consumed unchanged in T2–T5; `ConnectionListener`/`named_pipe`/`unix`/`ConnectionReceived`/`BindAsync`/`Information`/`Close` names stable T3→T5; `shared_runtime()` (T3) reused by T4. ✔
- **Risks flagged inline:** the `#[interface]` out-param ABI shape (T1 Step 3 note — match what compiles); projected-vs-inherent method ergonomics on `ConnectionListener` (T3 Step 1/4 notes); the named-pipe inter-instance connect window (T3 test uses a bounded retry). Each is validated by its task's test.
- **Placeholders:** the `<GUID1>`/`<GENERATE-A-FRESH-GUID>` is an instruction to mint one fresh UUID at implementation time (not a dangling TODO) — the only value that cannot be pre-baked. Everything else is concrete.
