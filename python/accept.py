"""The acceptance test: a Python script binds a listener, and awaits a
connection arriving over a Windows named pipe served by Rust.

Nothing here is registered: the component is reached through the reg-free
manifest loaded into this process's activation context.
"""
import asyncio, os, pathlib, sys

PKG_ROOT = pathlib.Path(__file__).parent / "out" / "rustwinui3loop-Rustwinui3loop" / "rustwinui3loop"
sys.path.insert(0, str(PKG_ROOT))
import actctx
actctx.activate(str(PKG_ROOT / "rustwinui3loop" / "rustwinui3loop.manifest"))

import rustwinui3loop as rw
from activate_instance import activate
from winrt.windows.storage.streams import DataReader, DataWriter

PIPE = rf"\\.\pipe\comio_py_{os.getpid()}"


async def main() -> int:
    loop = asyncio.get_running_loop()
    arrived: asyncio.Future = loop.create_future()

    listener = activate(rw.NamedPipeConnectionListener)

    def on_connection(_sender, args):
        # Runs on a Rust thread-pool thread, so hop back to the loop.
        stream = args.stream
        loop.call_soon_threadsafe(
            lambda: arrived.done() or arrived.set_result((stream, args.remote_endpoint))
        )

    # The projection types the delegate as a plain Callable, so a Python
    # function is the handler.
    listener.add_connection_received(on_connection)
    await listener.bind_async(PIPE)
    print(f"listening on {listener.information}")

    connector = activate(rw.NamedPipeConnector)
    print(f"connector transport = {connector.transport}")
    client = await connector.connect_async(PIPE)
    print("client connected")

    server, endpoint = await asyncio.wait_for(arrived, timeout=10)
    print(f"accepted from {endpoint!r}")

    writer = DataWriter(client)
    writer.write_string("hello from python")
    await writer.store_async()
    await writer.flush_async()

    reader = DataReader(server)
    n = await reader.load_async(64)
    got = reader.read_string(n)
    print(f"server read {got!r}")

    listener.close()
    assert got == "hello from python", got
    print("ROUND-TRIP-OK")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
