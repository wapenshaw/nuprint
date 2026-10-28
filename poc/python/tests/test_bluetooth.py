import asyncio
from contextlib import asynccontextmanager
from types import SimpleNamespace
from unittest.mock import AsyncMock
from PIL import Image
import pytest
from nuprint import bluetooth
from nuprint.bluetooth import Session, uuid
from nuprint.protocol import FrameDecoder, packet


def session(write_size=20):
    client = SimpleNamespace(is_connected=True, mtu_size=23, write_gatt_char=AsyncMock())
    value = Session(client, lambda _: None)
    value.write_char = SimpleNamespace(properties=["write-without-response"], max_write_without_response_size=write_size)
    return value


def notify(value, opcode, payload):
    value.receive(SimpleNamespace(uuid=uuid("ae02"), handle=1), packet(opcode, payload, flags=1))


async def test_chunk_size_respects_negotiated_limit():
    value = session()
    await value.send(bytes(range(53)))
    calls = value.client.write_gatt_char.call_args_list
    assert [len(c.args[1]) for c in calls] == [20, 20, 13]
    assert b"".join(c.args[1] for c in calls) == bytes(range(53))
    assert all(c.kwargs["response"] is False for c in calls)


async def test_buffer_full_pauses_until_resume():
    value = session()
    notify(value, 0xAE, b"\x10")
    task = asyncio.create_task(value.send(b"hello"))
    await asyncio.sleep(0.02)
    assert value.client.write_gatt_char.await_count == 0
    notify(value, 0xAE, b"\x00")
    await task
    assert value.client.write_gatt_char.await_count == 1


async def test_paper_out_prevents_writes():
    value = session()
    notify(value, 0xA3, b"\x01\x00\x00")
    with pytest.raises(RuntimeError, match="Out of paper"):
        await value.send(b"print")
    value.client.write_gatt_char.assert_not_called()


async def test_mid_job_fault_stops_next_chunk():
    value = session()
    async def write(*args, **kwargs):
        notify(value, 0xA3, b"\x04")
    value.client.write_gatt_char.side_effect = write
    with pytest.raises(RuntimeError, match="too hot"):
        await value.send(bytes(100))
    assert value.client.write_gatt_char.await_count == 1


async def test_status_requires_fresh_response():
    value = session()
    notify(value, 0xA3, b"\x00")
    with pytest.raises(RuntimeError, match="no valid printer status"):
        await value.query_status(timeout=0.01)


async def test_corrupt_flow_notification_does_not_resume():
    value = session()
    notify(value, 0xAE, b"\x10")
    raw = bytearray(packet(0xAE, b"\x00", flags=1))
    raw[-2] ^= 1
    value.receive(SimpleNamespace(uuid=uuid("ae02"), handle=1), raw)
    assert not value.writable.is_set()


async def test_disconnect_when_cancelled(monkeypatch):
    service = SimpleNamespace(uuid=uuid("ae30"), characteristics=[])
    write = SimpleNamespace(uuid=uuid("ae01"), service_uuid=service.uuid, properties=["write-without-response"])
    read = SimpleNamespace(uuid=uuid("ae02"), service_uuid=service.uuid, properties=["notify"])
    service.characteristics = [write, read]
    class Services(list):
        characteristics = {1: write, 2: read}
    client = SimpleNamespace(connect=AsyncMock(), disconnect=AsyncMock(), start_notify=AsyncMock(), services=Services([service]))
    monkeypatch.setattr(bluetooth, "find_device", AsyncMock(return_value=SimpleNamespace(name="SC03h-test", address="test")))
    monkeypatch.setattr(bluetooth, "BleakClient", lambda *a, **k: client)
    entered = asyncio.Event()
    async def job():
        async with bluetooth.connect("test", lambda _: None):
            entered.set()
            await asyncio.sleep(100)
    task = asyncio.create_task(job())
    await entered.wait()
    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await task
    client.disconnect.assert_awaited_once()


async def test_zero_feed_reaches_transport_and_still_checks_ready(monkeypatch):
    value = session()
    value.status = 0
    value.query_status = AsyncMock()
    value.send = AsyncMock()

    @asynccontextmanager
    async def connect(*args):
        yield value

    monkeypatch.setattr(bluetooth, "connect", connect)
    monkeypatch.setattr(bluetooth.asyncio, "sleep", AsyncMock())
    await bluetooth.print_image(Image.new("1", (384, 2), 1), log=lambda _: None, feed_mm=0)
    frames = FrameDecoder().feed(value.send.call_args.args[0])
    assert [f.opcode for f in frames] == [0xA4, 0xAF, 0xBE, 0xBD, 0xA2, 0xA2]
    assert value.query_status.await_count == 2


async def test_invalid_feed_never_opens_bluetooth(monkeypatch):
    connect = AsyncMock()
    monkeypatch.setattr(bluetooth, "connect", connect)
    with pytest.raises(ValueError, match="Extra feed"):
        await bluetooth.print_image(Image.new("1", (384, 1), 1), feed_mm=-1)
    connect.assert_not_called()
