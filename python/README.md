# Driving comio from Python

This is the acceptance test for the WinRT direction: a Python script binds a
listener and takes a connection over a named pipe served by Rust, through a
generated WinRT projection — no registry, no elevation, no C++ build.

```text
listening on \\.\pipe\comio_py_29588
connector transport = namedpipe
client connected
accepted from '\\.\pipe\comio_py_29588'
server read 'hello from python'
ROUND-TRIP-OK
```

## Running it

Nothing generated is checked in, so stage it first:

```bash
dotnet tool install --local PyWinRT
python -m pip install winrt-runtime winrt-Windows.Foundation \
                      winrt-Windows.Storage.Streams winrt-table-compiler
./stage.sh
python accept.py
```

`stage.sh` builds the cdylib, runs `cargo test --test activation` (which
writes the reg-free manifest out of `activation::manifest()`), generates the
projection from `metadata/rustwinui3loop.winmd`, compiles the projection
table, and copies the DLL and manifest into the package.

## How the component is reached

`HKLM\SOFTWARE\Microsoft\WindowsRuntime\ActivatableClassId` is owned by
`NT SERVICE\TrustedInstaller`, so a third-party component cannot register
there even elevated. [`actctx.py`](actctx.py) loads our manifest into the
process's activation context instead, which needs no elevation and no
registry. `python.exe`'s own manifest is not ours to edit, which is why this
is done at run time rather than by shipping one beside the interpreter.

## Two upstream gaps this ran into

**`windows-rdl` cannot emit a class constructor.** Its class encoder writes
the `TypeDef`, its `InterfaceImpl`s and attributes, and no `MethodDef` rows —
so there is no `.ctor` for PyWinRT to project, and the classes come out
without `__new__`. Instance members are unaffected: PyWinRT reads those from
the interfaces. [`activate_instance.py`](activate_instance.py) fills the gap
by calling `IActivationFactory::ActivateInstance` on the factory the
projection already knows how to find.

It is the *class* `.ctor` specifically, not the attribute. We checked that
first: matching the SDK's `Activatable(version, contract)` form and adding
`ContractVersion` made no difference, and `Windows.Storage.Streams.InMemoryRandomAccessStream`
— activatable with no factory interface — shows a bare `[activatable]` is
enough when the `.ctor` row exists. Those experiments were reverted, since
claiming membership in `Windows.Foundation.UniversalApiContract` would be
false.

**The Hyper-V members cannot be called.** PyWinRT warns that its ABI call
shape census has no callback for `(GUID, GUID) -> pointer`:

```text
warning: the shape census has no callback for slot 6 with the ABI call shape
  ({u4u2u2u1u1u1u1u1u1u1u1}{u4u2u2u1u1u1u1u1u1u1u1}p)
```

That is `IHyperVConnector::ConnectAsync` and
`IHyperVListenerBind::BindEndpointAsync`. They are projected but raise if
called. Named pipe and AF_UNIX are unaffected. The fix is upstream — a member
of that shape added to [testwinrt](https://github.com/pywinrt/testwinrt) — or
reshaping those two signatures.

## What this demonstrates

`accept.py` exercises the parts that only matter if the whole model is right:

* `await listener.bind_async(...)` — a real `IAsyncAction`, awaited by asyncio.
* `add_connection_received(...)` — a Python function invoked from a Rust
  thread-pool thread, hopping back to the loop with `call_soon_threadsafe`.
* `await connector.connect_async(...)` — a real
  `IAsyncOperation<TransportStream>`.
* `DataWriter`/`DataReader` — **Windows' own** stream classes reading and
  writing our stream, which only works because `TransportStream` really is an
  `IInputStream`/`IOutputStream`.
