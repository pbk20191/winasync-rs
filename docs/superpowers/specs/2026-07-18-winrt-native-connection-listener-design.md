# WinRT-native `ConnectionListener` (rich `StreamSocketListener`-shaped interface for comio)

**Date:** 2026-07-18
**Status:** Design approved, pending spec review

## Context

`src/comio/` already exposes named-pipe and AF_UNIX transports as WinRT stream
objects (`OverlappedDuplexStream : IInputStream/IOutputStream/IClosable`) plus
method-based factories (`NamedPipeListener::create`+`accept().await`,
`UnixStreamListener::bind`+`accept()`).

This project adds a **rich, WinRT-native listener interface mirroring
`Windows.Networking.Sockets.StreamSocketListener`**: a real COM event source
with a `ConnectionReceived` event, `BindAsync`, `Information`/`Control`
properties, and proper `IClosable` semantics — so consumers register a handler
and bind, and accepted connections arrive as events, exactly like
`StreamSocketListener`.

### Why not the literal `StreamSocketListener`

A literal implementation is impossible: `StreamSocketListenerConnectionReceivedEventArgs::Socket()`
is hard-typed to return the sealed concrete class `StreamSocket`
(`windows` 0.62.2 Networking/Sockets/mod.rs:3715), which cannot be fabricated
for a pipe/AF_UNIX handle; and the primary `IStreamSocketListener`/`IStreamSocket`
interfaces have no windows-rs `_Impl` traits (not user-implementable). We instead
build a **parallel interface of the same shape** over our own COM types.

### Feasibility (verified against crate sources)

- `windows_core::Event<T: Interface>` (windows-core 0.62.2 `event.rs`) is the
  WinRT event-source primitive: `add(&delegate) -> i64 token`, `remove(token)`,
  `call(|d| d.Invoke(..))`; thread-safe, agile-aware. Backs `ConnectionReceived`.
- `windows_core::interface` (`#[interface("GUID")]`) and `windows_core::implement`
  (`#[implement]`) are both available (re-exported in windows-core `lib.rs:58-59`),
  so we can define a custom fixed-IID COM interface and implement it.
- `TypedEventHandler<IInspectable, IInspectable>` is projectable (both type args
  are `RuntimeType`), usable as the event delegate.
- `IClosable_Impl` is implementable (used already by `OverlappedDuplexStream`).
- The existing `accept()` futures (`comio::pipe`, `comio::unix`) and their
  cancel-safe `await_io_owning` engine drive the accept loop; `tp::Runtime`
  (same crate) drives it to run without a caller-provided executor.

## Decisions (locked)

1. **Level:** WinRT-native COM event source — real `IInspectable`/`IClosable`
   objects, a `windows_core::Event`-backed `ConnectionReceived`, projectable
   `TypedEventHandler`. Rust consumes fully; C++/WinRT via known IIDs; full C#
   projection is out of scope (needs a `.winmd` we cannot emit — documented).
2. **Connection delivery:** event args expose the accepted stream via a custom
   fixed-IID interface method `IConnectionReceivedArgs::Stream()`.
3. **Close semantics:** `Close` actually closes; **all subsequent member calls
   return `RO_E_CLOSED` (0x80000013)**. Applies to the new listener AND upgrades
   the existing `OverlappedDuplexStream::Close` (was a no-op).
4. **Self-driving:** the listener runs its own accept loop on a crate-shared
   lazy `tp::Runtime`; `ConnectionReceived` fires on a pool thread (as WinRT
   `StreamSocketListener` does). comio gains a real (non-test) dep on `tp` — dq
   independence is unaffected (tp ≠ dq).
5. **Transports:** named pipe + AF_UNIX, sharing one listener implementation.

## Architecture

New file `src/comio/listener.rs` (+ small additions to `stream.rs`, `mod.rs`),
layered on the existing transports — nothing existing is removed.

### Custom interface (fixed IID)

```rust
// GUID generated once at implementation time and frozen as a constant.
#[interface("<one generated, stable GUID>")]
unsafe trait IConnectionReceivedArgs: IUnknown {
    fn Stream(&self, result: *mut *mut c_void) -> HRESULT;       // accepted OverlappedDuplexStream as IInspectable
    fn RemoteEndpoint(&self, result: *mut HSTRING) -> HRESULT;   // pipe name / AF_UNIX peer path (info)
}
```
(`#[interface]` generates an IUnknown-derived COM interface; the exact method
signature style follows windows-rs `#[interface]` conventions — raw ABI with
out-params, wrapped by generated safe callers.)

### COM objects

- **`ConnectionReceivedArgs`** — `#[implement(IConnectionReceivedArgs)]` holding
  the accepted `OverlappedDuplexStream` (as `IInspectable`) + the remote
  endpoint string. `Stream()` returns the stream; consumer casts it to
  `IInputStream`/`IOutputStream`. Being an `#[implement]` object it is also
  `IInspectable`, so it is a valid `TypedEventHandler` arg.
- **`ConnectionListener`** — `#[implement(IClosable)]`, the `StreamSocketListener`
  analog. Public methods (each checks `closed` first → `RO_E_CLOSED`):
  | Member | Behavior |
  |---|---|
  | `ConnectionReceived(handler: &TypedEventHandler<IInspectable, IInspectable>) -> Result<i64>` | `state.event.add(handler)` → token |
  | `RemoveConnectionReceived(token: i64) -> Result<()>` | `state.event.remove(token)` |
  | `BindAsync(endpoint: &HSTRING) -> Result<IAsyncAction>` | create listening handle/socket for the endpoint (pipe name / unix path), then spawn the accept loop on the shared `tp::Runtime`; the returned action completes once bound |
  | `Information() -> Result<ListenerInformation>` | local endpoint (pipe name / unix path) |
  | `Control() -> Result<ListenerControl>` | pre-bind options (backlog, pipe instances, buffer sizes) |
  | `Close()` (`IClosable`) | idempotent: `closed=true`; abort the accept-loop task (its drop `CancelIoEx`s the in-flight accept); close the listening handle; `event.clear()` |
  - Transport is chosen at construction: `ConnectionListener::named_pipe()` /
    `ConnectionListener::unix()` (or an enum), sharing all logic; only the
    listening-endpoint creation + `accept()` call differ (delegating to the
    existing `comio::pipe`/`comio::unix` internals).
- **`ListenerInformation` / `ListenerControl`** — small `#[implement]` objects
  (or plain structs behind a custom interface) carrying the info/options.

### Shared state & accept loop

```rust
struct ListenerState {
    closed: AtomicBool,
    event: windows_core::Event<TypedEventHandler<IInspectable, IInspectable>>,
    // listening endpoint + transport kind + control snapshot; accept-task abort handle
}
```
`Arc<ListenerState>` is held by the `ConnectionListener` COM object and by the
accept-loop task. The loop (spawned on the shared `tp::Runtime` at `BindAsync`):
1. `let stream = accept().await?` (existing cancel-safe transport accept);
2. wrap in `ConnectionReceivedArgs`;
3. `state.event.call(|h| h.Invoke(&sender_iinspectable, &args_iinspectable))`;
4. repeat until `closed` or the listening handle is closed.
`Close` sets `closed`, aborts the task (`tp::JoinHandle::abort` → the pending
`accept()` future drops → `CancelIoEx` per the existing cancel path), and closes
the listening handle. Firing on the pool thread matches WinRT behavior; UI
consumers marshal via `DispatcherQueue` inside their handler.

### Crate-shared runtime

`src/comio/mod.rs` gains a `pub(crate) fn shared_runtime() -> &'static tp::Runtime`
(lazy `OnceLock<tp::Runtime>`) used to drive listener accept loops. This is the
only non-test comio→tp dependency; keep it internal.

### `OverlappedDuplexStream` Close upgrade

`stream.rs` gains a `closed: AtomicBool` in the implement struct. `Close` sets it
(+ any cheap resource release; in-flight ops keep their own `Arc<IoReg>`).
`ReadAsync`/`WriteAsync` check it first → `RO_E_CLOSED`. This replaces the prior
no-op Close (and supersedes the "Close is advisory" note in the earlier spec).

## Key invariants & edge cases

- **COM identity is framework-generated — never hand-rolled.** `#[implement]`
  auto-generates the full `IUnknown` (`QueryInterface`/`AddRef`/`Release`) and
  `IInspectable`, backed by a thread-safe internal refcount (`ComObject`/
  `RefCount`); `#[interface]` supplies the custom vtable slots on top of that
  IUnknown base. We implement ONLY the custom methods (`Stream`, `ConnectionReceived`,
  …) — implementing `AddRef`/`Release`/`QueryInterface` ourselves is wrong and
  would conflict. (All existing comio COM objects already rely on this.)
- **Agility (free-threaded marshaling).** `#[implement]` implements
  `IAgileObject`/`IMarshal` by default (`windows-implement` `agile = true`), so
  `ConnectionReceivedArgs`/`ConnectionListener`/the stream can be safely marshaled
  by a handler onto another thread (e.g. a consumer forwarding the connection to
  the UI thread via `DispatcherQueue`). This holds only while every field is
  `Send + Sync`; keep these objects' state to `Arc`-wrapped, thread-safe handles
  (as the existing objects do) — do NOT introduce `Rc`/`!Send` fields, and do not
  pass `Agile(false)`.
- **`RO_E_CLOSED` everywhere post-Close:** every public member of the listener
  and (for reads/writes) the stream checks the `closed` flag before acting;
  return `Err(windows_core::Error::from_hresult(HRESULT(0x80000013u32 as i32)))`
  (use a named `RO_E_CLOSED` const if available). `Close` itself is idempotent.
- **Accept-loop cancel safety:** aborting the loop drops the in-flight `accept()`
  future, which `CancelIoEx`s the overlapped `AcceptEx`/`ConnectNamedPipe` and
  (via the kernel-gated op block from the existing `await_io_owning`) frees the
  address buffer only on the delivered completion — no UAF, no leak.
- **Event thread-safety:** `windows_core::Event` is Send+Sync and agile-wraps
  non-agile delegates; `call` invokes on the accept-completion (pool) thread.
  A handler that panics must not unwind across the COM boundary — wrap the
  `event.call` body / each `Invoke` defensively (`Event::call` already isolates
  a failing delegate; a panicking Rust handler closure is caught with
  `catch_unwind` at the fire site).
- **Send/'static for the loop task:** `ListenerState` (AtomicBool + `Event` +
  Arc'd handles) is Send+Sync; the accepted `OverlappedDuplexStream`/args are
  agile `#[implement]` objects — the task closure is `Send + 'static`.
- **No panic across FFI** in any `#[implement]`/`#[interface]` method or the
  event fire.
- **Bind before Control changes take effect:** `Control` options are snapshotted
  at `BindAsync`; changing them after bind has no effect on the running listener
  (documented — no error, matching WinRT's mutate-before-bind convention).

## Testing strategy

- **Event delivery (pipe + unix):** register a `TypedEventHandler` that captures
  the accepted stream (via `args.cast::<IConnectionReceivedArgs>()?.Stream()`),
  `BindAsync`, connect a client with `NamedPipeClient`/`UnixStreamSocket`, and
  echo a payload through the delivered stream (cast to `IInputStream`/
  `IOutputStream`), driven by `tp::block_on`. Assert bytes round-trip. Test ≥2
  sequential connections firing the event twice.
- **`args.Stream()` identity:** the delivered args QIs to `IConnectionReceivedArgs`
  and its `Stream()` casts to `IInputStream`.
- **Close semantics:** after `Close()`, assert `BindAsync`/`ConnectionReceived`/
  `Information`/`Control` all return `RO_E_CLOSED`; assert no further events fire;
  assert the in-flight accept was cancelled with `comio::tpio::LIVE_OPS == 0`
  (debug, bounded wait). `Close` twice is Ok/no-op.
- **Stream Close:** after `OverlappedDuplexStream::Close`, `ReadAsync`/`WriteAsync`
  return `RO_E_CLOSED`; a read/write issued *before* Close still completes.
- **Demo (optional):** extend `main.rs` `dq_demo` to register a
  `ConnectionReceived` handler that marshals to the DQ thread — showing the
  UI-affine consumption pattern. (Only if it stays short and deterministic.)

## Build order

1. `IConnectionReceivedArgs` (`#[interface]`, fixed GUID) + `ConnectionReceivedArgs`
   (`#[implement]`) — unit test QI + `Stream()`.
2. `RO_E_CLOSED` + `closed` flag in `OverlappedDuplexStream` (Close upgrade +
   post-close read/write error) — with tests.
3. `comio::shared_runtime()` lazy `tp::Runtime`.
4. `ListenerState` + accept loop + `ConnectionListener` (`ConnectionReceived`/
   `BindAsync`/`Close`) for named pipe — event-delivery + close tests.
5. AF_UNIX transport in the same `ConnectionListener`.
6. `Information`/`Control` + re-exports (+ optional demo).

## Out of scope (YAGNI)

- Full C# metadata projection (no `.winmd`); document the limitation.
- TCP-style socket `Control` (NoDelay/KeepAlive) on pipe/unix connections.
- Replacing the existing method-based `accept().await` API — it remains as the
  low-level engine the event loop is built on.
