'use strict';
// The acceptance test from Node: bind a listener, connect to it, and move
// bytes over a Windows named pipe served by Rust -- through a projection
// generated from the published `.winmd`, with nothing registered.
//
// The same script as `python/accept.py`, in a third language and through a
// third, independent reader of the metadata: dynwinrt.

const path = require('node:path');

// `stage.sh` puts the component, its manifest and both generated binding
// packages under out/.
const OUT = path.join(__dirname, 'out');
require('./actctx').activate(path.join(OUT, 'winasync.manifest'));

const { roInitialize } = require('@microsoft/dynwinrt');
// MTA: every completion and event arrives on a Rust thread-pool thread, and
// dynwinrt hands it back to this event loop.
roInitialize(1);

const comio = require(path.join(OUT, 'comio'));
const { DataReader, DataWriter } = require(path.join(OUT, 'streams'));

async function namedPipeRoundTrip() {
  const pipe = String.raw`\\.\pipe\comio_node_` + process.pid;

  // A real constructor. PyWinRT needs `activate_instance.py` for this,
  // because it wants a `.ctor` row `windows-rdl` does not emit; dynwinrt,
  // like CsWinRT, is satisfied by `ActivatableAttribute`.
  const listener = new comio.NamedPipeConnectionListener();
  const arrived = new Promise((resolve) =>
    listener.onceConnectionReceived((_sender, args) =>
      resolve({ stream: args.stream, from: args.remoteEndpoint })));
  await listener.bindAsync(pipe);
  console.log(`listening on ${listener.information}`);

  const connector = new comio.NamedPipeConnector();
  console.log(`connector transport = ${connector.transport}`);
  const client = await connector.connectAsync(pipe);
  console.log('client connected');

  const server = await withTimeout(arrived, 10_000, 'ConnectionReceived');
  console.log(`accepted from ${JSON.stringify(server.from)}`);

  // `ITransportStream` goes straight into the system `DataWriter`/
  // `DataReader`: the projection queries for `IOutputStream`/`IInputStream`
  // at the call, as a composed stream would need it to.
  const writer = new DataWriter(client);
  writer.writeString('hello from node');
  await writer.storeAsync();
  await writer.flushAsync();

  const reader = new DataReader(server.stream);
  const n = await reader.loadAsync(64);
  const got = reader.readString(n);
  console.log(`server read ${JSON.stringify(got)}`);

  listener.close();
  if (got !== 'hello from node') throw new Error(`read ${JSON.stringify(got)}`);
}

// The same round trip over AF_HYPERV, which skips where the provider is
// absent -- most machines without Hyper-V. See `python/accept.py` for why
// the endpoint is a string.
async function hyperVRoundTrip() {
  const service = 'vsock:54321';
  const listener = new comio.HyperVConnectionListener();
  const arrived = new Promise((resolve) =>
    listener.onceConnectionReceived((_sender, args) => resolve(args.stream)));
  try {
    // Binding is lazy, so an absent provider surfaces here.
    await listener.bindServiceNameAsync(service);
  } catch (e) {
    console.log(`hyper-v skipped: ${e.message}`);
    return;
  }
  console.log(`hyper-v listening on ${listener.information}`);

  const connector = new comio.HyperVConnector();
  const client = await connector.connectAsync(`hv://loopback/${service}`);
  const server = await withTimeout(arrived, 10_000, 'hyper-v ConnectionReceived');
  console.log('hyper-v accepted');

  const writer = new DataWriter(client);
  writer.writeString('hello over hvsock');
  await writer.storeAsync();
  await writer.flushAsync();

  const reader = new DataReader(server);
  const n = await reader.loadAsync(64);
  const got = reader.readString(n);
  console.log(`hyper-v server read ${JSON.stringify(got)}`);
  listener.close();
  if (got !== 'hello over hvsock') throw new Error(`read ${JSON.stringify(got)}`);
}

function withTimeout(promise, ms, what) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${what} timed out`)), ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

(async () => {
  await namedPipeRoundTrip();
  await hyperVRoundTrip();
  console.log('NODE-ROUND-TRIP-OK');
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
