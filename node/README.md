# Driving comio from Node

The third projection of the same metadata, after [`python/`](../python/README.md)
and [`csharp/`](../csharp/README.md), and the last of the three runtimes the
`.winmd` was published for. It goes through
[dynwinrt](https://github.com/microsoft/dynwinrt), Microsoft's dynamic WinRT
projection for Node (public preview since 2026-07): the bindings are plain
JavaScript generated from the `.winmd`, with no native addon and no node-gyp
build of our own.

```text
listening on \\.\pipe\comio_node_5296
connector transport = namedpipe
client connected
accepted from "\\\\.\\pipe\\comio_node_5296"
server read "hello from node"
hyper-v listening on hv://wildcard/vsock:54321
hyper-v accepted
hyper-v server read "hello over hvsock"
NODE-ROUND-TRIP-OK
```

## Running it

Node 26 or later: [`actctx.js`](actctx.js) uses the built-in `node:ffi`,
which is experimental and behind `--experimental-ffi` (the `accept` script
passes it).

```bash
npm install
npm run stage    # builds the DLL, writes the manifest, generates bindings into out/
npm run accept
```

`stage.sh` builds the server DLL (`cargo build -p winasync-dll`) and runs
`cargo test --test activation`, which writes the reg-free manifest out of
`activation::manifest()`. It then generates two binding packages, `out/comio`
from `metadata/Pbk20191.WinAsync.Comio.winmd` and `out/streams` for the system
`DataWriter`/`DataReader`, and copies the DLL and the manifest beside them.

The dynwinrt packages are pinned to an exact preview version in
`package.json`, because a preview may change its generated API between
releases.

## How the component is reached

The same way as from Python, for the same reason. The machine-wide WinRT
registration tree belongs to TrustedInstaller, so the component is reached
reg-free: `actctx.js` loads our manifest into an activation context, and the
generated constructors' `RoGetActivationFactory` finds the class there.

dynwinrt activates by class name only and has no hook for a DLL path, so this
is the one step it cannot do for us. Its own flat-Win32 layer was not an
option either: that layer generates from `Windows.Win32.winmd`, which the
Windows SDK does not ship. `node:ffi` calls kernel32 directly instead.

## What this reader found

- **Constructors work.** `new NamedPipeConnectionListener()` works as it does
  in C#. dynwinrt, like CsWinRT, is satisfied by `ActivatableAttribute`, so
  only PyWinRT needs the `.ctor` row that `windows-rdl` does not emit. That
  supports the earlier conclusion that the upstream report belongs with
  PyWinRT.
- **Events arrive on the event loop.** `onConnectionReceived` fires from a
  Rust thread-pool thread, and dynwinrt delivers it back to JavaScript. Async
  methods are Promises.
- **Interface arguments are queried at the call.** `new DataWriter(stream)`
  accepts our `ITransportStream` where `IOutputStream` is expected, because
  the projection does the `QueryInterface`. A composed stream needs exactly
  that.
- **The Hyper-V members are callable.** That is because they take strings;
  see `python/README.md` for why the GUID-pair shape was reshaped.
- **Two generated packages do not share interface classes.** `IOutputStream`
  from `out/streams` and the one from `out/comio` are different JavaScript
  classes for the same WinRT interface, so `IOutputStream.from(x)` across
  packages needs a raw value. `projectAs` is only for runtime classes. The
  script avoids the issue by passing streams straight into
  `DataWriter`/`DataReader`, which query for themselves.
