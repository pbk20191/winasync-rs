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

    await hyperv_round_trip()

    print("ROUND-TRIP-OK")
    return 0


async def hyperv_round_trip() -> None:
    """The same round trip over an AF_HYPERV socket.

    This is the transport with the strongest claim to this crate existing:
    the only one crossing a host/guest trust boundary, and the only one WinRT
    cannot even name -- `HostNameType` is DomainName, Ipv4, Ipv6, Bluetooth.

    It is also the one that had to be reshaped to be reachable from here.
    PyWinRT's ABI call shape census has no callback for a
    `(GUID, GUID) -> pointer` member, so the typed form projected and then
    raised; the metadata takes `hv://<vm>/<service>` instead. Nothing is lost
    -- `HvSocketAddr`'s `Display` and `FromStr` are exact inverses, and either
    half may still be spelled as a plain GUID.

    Skips where the AF_HYPERV provider is absent, which is most machines
    without Hyper-V enabled.
    """
    loop = asyncio.get_running_loop()
    arrived: asyncio.Future = loop.create_future()
    service = "vsock:54321"

    listener = activate(rw.HyperVConnectionListener)
    listener.add_connection_received(
        lambda _s, args: loop.call_soon_threadsafe(
            lambda: arrived.done() or arrived.set_result(args.stream)
        )
    )
    try:
        # Binding is lazy, so an absent provider surfaces here rather than
        # at construction.
        await listener.bind_service_name_async(service)
    except OSError as e:
        print(f"hyper-v skipped: {e}")
        return
    print(f"hyper-v listening on {listener.information}")

    connector = activate(rw.HyperVConnector)
    client = await connector.connect_async(f"hv://loopback/{service}")
    server = await asyncio.wait_for(arrived, timeout=10)
    print("hyper-v accepted")

    writer = DataWriter(client)
    writer.write_string("hello over hvsock")
    await writer.store_async()
    await writer.flush_async()

    reader = DataReader(server)
    n = await reader.load_async(64)
    got = reader.read_string(n)
    print(f"hyper-v server read {got!r}")
    listener.close()
    assert got == "hello over hvsock", got


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
