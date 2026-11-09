'use strict';
// Loads the component's reg-free WinRT manifest into an activation context.
//
// The Node counterpart of `python/actctx.py`, for the same reason: the
// machine-wide WinRT registration tree is owned by TrustedInstaller, so a
// third-party component is reached reg-free, through a manifest on the
// consuming process. `node.exe`'s manifest is not ours to edit, so ours is
// activated at run time.
//
// dynwinrt activates classes by name only (`RoGetActivationFactory`), and
// has no hook for a DLL path, so this is the one piece it cannot do for us.
// Its own flat-Win32 layer generates bindings from `Windows.Win32.winmd`,
// which the Windows SDK does not ship; Node's built-in `node:ffi` reaches
// kernel32 directly instead. That module is experimental, hence the
// `--experimental-ffi` flag in package.json.
//
// An activation context is per thread. Call this on the thread that
// constructs the objects -- the main JavaScript thread -- before the first
// one is made. Objects already created keep working after that.

const ffi = require('node:ffi');

const kernel32 = ffi.dlopen('kernel32.dll', {
  CreateActCtxW: { arguments: ['pointer'], return: 'pointer' },
  ActivateActCtx: { arguments: ['pointer', 'pointer'], return: 'int32' },
  GetLastError: { arguments: [], return: 'uint32' },
}).functions;

// `ACTCTXW` on 64-bit Windows: cbSize, dwFlags, lpSource, then fields this
// call leaves zero. The only field that is not a constant is `lpSource`, a
// pointer to the UTF-16 path at offset 8.
const ACTCTXW_SIZE = 56;
const INVALID_HANDLE_VALUE = 0xffffffffffffffffn;

// The context is activated for the life of the process and never released,
// as in the Python version, so its source path must outlive the call too.
const retained = [];

function activate(manifestPath) {
  const source = Buffer.from(manifestPath + '\0', 'utf16le');
  retained.push(source);

  const ctx = Buffer.alloc(ACTCTXW_SIZE);
  ctx.writeUInt32LE(ACTCTXW_SIZE, 0);
  ctx.writeBigUInt64LE(BigInt(ffi.getRawPointer(source)), 8);

  const handle = BigInt.asUintN(64, BigInt(kernel32.CreateActCtxW(ctx)));
  if (handle === 0n || handle === INVALID_HANDLE_VALUE) {
    throw new Error(`CreateActCtxW(${manifestPath}) failed: error ${kernel32.GetLastError()}`);
  }
  const cookie = Buffer.alloc(8);
  if (!kernel32.ActivateActCtx(handle, cookie)) {
    throw new Error(`ActivateActCtx failed: error ${kernel32.GetLastError()}`);
  }
}

module.exports = { activate };
