# Design: `tp` / `dq` / `comio` as independent cargo features over a unified overlapped-IO core

Date: 2026-11-08
Status: Draft (pending approval)

## Intent & why

The crate has grown three runtime pillars:

- **`tp`** — a from-scratch Rust async runtime over the Win32 thread pool (TP_*). Server-side.
- **`dq`** — a single-threaded async runtime whose executor is a WinRT `DispatcherQueue` (UI/WinUI 3).
- **`comio`** — WinRT-shaped COM components (`IInputStream`/`IOutputStream`/`ConnectionListener`/…) over overlapped IO.

The user wants **clean boundaries**: when doing server programming you use `tp` and it is *wrong* for WinUI 3 / WinRT dependencies (`winio-winui3`, `windows-future`) to be dragged in. Concretely: **building with only `tp` must pull zero `winio-winui3` / `windows-future`.** All three pillars should be **independently selectable cargo features**.

Investigation confirms the three are already independent at *runtime* (non-test) code:
- `tp`: only `windows` (Win32_*) + `windows-core`.
- `dq`: only `winio-winui3` (+ `windows-core`).
- `comio`: `windows-future` + `windows-threading` + `windows` (Storage_Streams, …); has its own overlapped core (`tpio`), does **not** use `tp`.
Only **test** code crosses boundaries (comio tests use `tp::block_on`; dq integration tests use comio+tp+windows-future).

## Goals

1. Three cargo features — `tp`, `dq`, `comio` — each independently buildable. `tp` alone links neither `winio-winui3` nor `windows-future`.
2. **Unify the overlapped-IO core (option B):** replace the near-duplicate `tp::io` `IoRes/submit/Op` and `comio::tpio` `IoReg/submit/Op/OpHandle` with **one shared core**, parameterized by (a) the TP callback environment and (b) reconciling the two cancellation models.
3. Factor the genuinely-neutral shared pieces into a `common` module: `oneshot` (all three), `block_on` + `check` (tp+comio).
4. Default build + `cargo run` + the demos keep working unchanged (default = all three features).
5. (Follow-on, enabled by this structure) `tp`-native named pipe + AF_UNIX stream, and UDP `WSARecvMsg`/`WSASendMsg` land cleanly afterward — **out of scope for this doc's implementation**, listed so the structure accommodates them.

## Non-goals

- **Not** a workspace / multi-crate split. Single crate + cargo features (the user asked for feature on/off).
- **No** public API/behavior changes to `tp`/`dq`/`comio` surfaces. Pure restructuring + internal core merge.
- **No** new transports/WinRT types in this doc (pipe/unix/WSARecvMsg are follow-on).
- comio stays non-IP (does not duplicate WinRT `DatagramSocket`/`StreamSocket`; see the pillar-3 gap criterion).

## Anti-goals (failure even if it compiles)

- Any of the 81 `tp` tests or the comio/dq tests regressing or becoming flaky.
- The `tp` feature transitively linking `winio-winui3` or `windows-future`.
- A unified unsafe core that is *less* safe/correct than the two it replaces — i.e. losing any current invariant:
  - op block refcount starts at 2 (future/handle side + kernel side); freed exactly once.
  - `StartThreadpoolIo` balanced 1:1 by a delivered completion or `CancelThreadpoolIo` on sync-failure.
  - no panic crosses the FFI completion callback.
  - tp's **Future-drop-cancels** semantics preserved; comio's **explicit `OpHandle::cancel`** preserved.
  - `#[cfg(debug_assertions)] LIVE_OPS` leak accounting preserved for the leak tests.

## Constraints

- Windows only; edition 2024; keep every existing test green throughout (verify after each step).
- The unified core is `unsafe`; it must preserve the invariants above exactly. TDD / incremental, re-running the leak + cancel-storm tests at each step.

## The how — decided

### Cargo features
- Features `tp`, `dq`, `comio`; `default = ["tp", "dq", "comio"]` so existing builds/demos are unchanged.
- Optional deps: `winio-winui3` (→ `dq`), `windows-future` (→ `comio`), `windows-threading` (→ `comio`).
- `windows` stays a required dep but its **feature set is split**: base list = the Win32_* that `tp` needs; `comio`/`dq` features add their extra `windows/...` features via the feature table (e.g. `comio = ["dep:windows-future", "windows/Storage_Streams", ...]`).
- `main.rs` demos cfg-gated: `tp_demo` under `feature="tp"`, `dq_demo` under `all(feature="dq", feature="comio", feature="tp")`; the WinUI section under `feature="dq"`.

### Module layout
- `src/common/` compiled under `any(feature="tp", feature="dq", feature="comio")`:
  - `oneshot.rs` (moved from `async_util`) — all three.
  - `block_on.rs` (moved from `tp`) + `check` — compiled under `any(feature="tp", feature="comio")`.
  - `overlapped.rs` — the unified core, compiled under `any(feature="tp", feature="comio")`.
- `src/tp/`, `src/dq/`, `src/comio/` under their own features; `tp`/`comio` depend on `common::overlapped`.

### Unified overlapped core (`common::overlapped`)
- Registration `IoReg { handle, io, is_socket, env_keepalive: Option<Arc<dyn Any + Send + Sync>> }`; `register(handle, is_socket, env: PTP_CALLBACK_ENVIRON | null, keepalive)`.
  - `tp` passes its runtime pool env + `Some(rt.clone())` keepalive; `comio` passes null + `None`.
- Primitive: `submit(reg, payload: P, issue, on_complete: Box<dyn FnOnce(u32,usize)+Send>) -> OpHandle`. Op block = `Op<P>` (OVERLAPPED-first `repr(C)`, header{refs, reg, release}, payload `UnsafeCell<Option<P>>`), single `io_complete` callback that runs `on_complete` then releases the kernel ref. `OpHandle::cancel` = `CancelIoEx`.
  - **comio** uses this directly (its current `OpHandle` + `CompleteFn` model *is* the primitive).
  - **tp**'s `OpFuture<P>` becomes a thin wrapper over an `OpHandle`: `on_complete` stores `(ioresult, bytes)` + wakes a shared waker slot; `poll` returns `(Result<usize>, P)` by taking the payload from the op block; `Drop` cancels if still in flight.
- `LIVE_OPS` accounting lives in the core (both leak tests keep working).

### Test re-homing
- comio tests switch `crate::tp::block_on` → `crate::common::block_on` (so they don't require the `tp` feature).
- Tests that genuinely span features are gated with `#[cfg(all(feature=…, …))]` (e.g. dq integration tests → `all(dq, comio, tp)`).

## Left to the builder

- Exact signature of the unified `register`/`submit` (how to type the callback env; `Option<Arc<dyn Any>>` keepalive vs a generic `R: Keepalive` param).
- Whether `block_on`/`check` live in `common` root or a `common::util`.
- Precise `#[cfg]` gating of each cross-feature test and of `main.rs` sections.
- Sequencing: (1) introduce `common` (oneshot/block_on/check, no behavior change) → (2) unify overlapped core, tp+comio migrate, all tests green → (3) add cargo features + optional-dep plumbing + demo/test gating → (4, separate) pipe/unix + WSARecvMsg.
