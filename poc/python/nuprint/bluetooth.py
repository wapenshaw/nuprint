"""Short-lived GATT sessions; no pairing, service installation or admin needed."""

import asyncio
from contextlib import asynccontextmanager
import json
import time
from bleak import BleakClient, BleakScanner
from .protocol import DEFAULT_FEED_MM, FrameDecoder, packet, print_job, status_text


def uuid(short: str) -> str:
    return f"0000{short}-0000-1000-8000-00805f9b34fb"


SERVICES = {uuid("ae30"), uuid("ae00")}
WRITE = uuid("ae01")
NOTIFY = {uuid("ae02"), uuid("ae04")}
MODEL_PREFIX = "SC03h-"
DEFAULT_NAME = "SC03h-3DE8"


async def scan(timeout: float = 8):
    found = await BleakScanner.discover(timeout=timeout, return_adv=True)
    # Name matching is intentional: SC03h devices advertise AF30 but serve AE30.
    return sorted([(device, adv) for device, adv in found.values()
                   if (adv.local_name or device.name or "").startswith(MODEL_PREFIX)],
                  key=lambda item: item[0].address)


async def find_device(target: str):
    target = target.strip()
    if target.casefold() == MODEL_PREFIX.casefold():
        found = await BleakScanner.discover(timeout=12, return_adv=True)
        devices = [device for device, adv in found.values()
                   if (adv.local_name or device.name or "").startswith(MODEL_PREFIX)]
        if len(devices) > 1:
            names = ", ".join(device.name or device.address for device in devices)
            raise RuntimeError(f"Multiple SC03h printers found ({names}); select one by its full name or Bluetooth address.")
        device = devices[0] if devices else None
    else:
        device = await BleakScanner.find_device_by_filter(
            lambda d, a: d.address.casefold() == target.casefold()
            or (a.local_name or d.name or "").casefold() == target.casefold(), timeout=12)
    if device is None:
        raise RuntimeError(f"{target} was not found. Turn the printer on, keep it near the PC, "
                           "and close the phone app. Windows Bluetooth must be on.")
    if not (device.name or "").startswith(MODEL_PREFIX):
        raise RuntimeError("This client supports lowercase SC03h only. Device name did not match that model.")
    return device


class Session:
    def __init__(self, client, log):
        self.client = client
        self.log = log
        self.writable = asyncio.Event()
        self.writable.set()
        self.status_changed = asyncio.Event()
        self.status = None
        self.last_status = 0.0
        self.decoders = {}
        self.notifications = []
        self.write_char = None

    def receive(self, characteristic, data):
        raw = bytes(data)
        self.notifications.append({"characteristic": characteristic.uuid, "hex": raw.hex(" ")})
        self.log(f"RX {characteristic.uuid[4:8]}: {raw.hex(' ')}")
        decoder = self.decoders.setdefault(characteristic.handle, FrameDecoder())
        for frame in decoder.feed(raw):
            if frame.flags != 1:
                continue
            if frame.opcode == 0xAE and frame.payload:
                if frame.payload[0] == 0x10:
                    self.writable.clear()
                    self.log("Printer buffer full; pausing transfer")
                elif frame.payload[0] == 0:
                    self.writable.set()
            elif frame.opcode == 0xA3 and frame.payload:
                self.status = frame.payload[0]
                self.last_status = time.monotonic()
                self.status_changed.set()
                self.log(f"Printer: {status_text(self.status)}")

    def check(self):
        if not self.client.is_connected:
            raise RuntimeError("Bluetooth disconnected. Some paper may already have printed; check before retrying.")
        if self.status is not None and self.status & 0x0F:
            raise RuntimeError(f"Printer reports: {status_text(self.status)}. Fix this before printing again.")

    async def send(self, data: bytes, progress=None, check_status=True):
        char = self.write_char
        response = "write-without-response" not in char.properties
        limit = min(180, char.max_write_without_response_size if not response else max(20, self.client.mtu_size - 3))
        if limit < 1:
            raise RuntimeError("Printer reported an invalid Bluetooth write size")
        for offset in range(0, len(data), limit):
            if check_status:
                self.check()
            try:
                await asyncio.wait_for(self.writable.wait(), timeout=20)
            except TimeoutError as exc:
                raise RuntimeError("Printer buffer stayed full for 20 seconds; transfer stopped.") from exc
            if check_status:
                self.check()
            chunk = data[offset:offset + limit]
            await asyncio.wait_for(self.client.write_gatt_char(char, chunk, response=response), timeout=10)
            if progress:
                progress(min(offset + len(chunk), len(data)), len(data))
            # Conservative pacing; do not send the entire stream at full BLE speed.
            await asyncio.sleep(0.025)
            if offset // 4096 != (offset + len(chunk)) // 4096:
                await asyncio.sleep(0.25)

    async def query_status(self, timeout=3):
        self.status_changed.clear()
        await self.send(packet(0xA3), check_status=False)
        try:
            await asyncio.wait_for(self.status_changed.wait(), timeout)
        except TimeoutError as exc:
            raise RuntimeError("Connected, but no valid printer status response arrived. "
                               "Switch the printer off/on and close other printer apps.") from exc
        return self.status


@asynccontextmanager
async def connect(target: str, log=print):
    log(f"Finding {target}…")
    device = await find_device(target)
    client = BleakClient(device, timeout=20, winrt={"use_cached_services": False})
    try:
        await asyncio.wait_for(client.connect(), timeout=25)
        session = Session(client, log)
        for service in client.services:
            if service.uuid not in SERVICES:
                continue
            for char in service.characteristics:
                if char.uuid == WRITE and ({"write", "write-without-response"} & set(char.properties)):
                    session.write_char = char
        if session.write_char is None:
            raise RuntimeError("SC03h print service AE30/AE00 with write channel AE01 was not found")
        subscribed = 0
        for char in client.services.characteristics.values():
            if char.service_uuid in SERVICES and char.uuid in NOTIFY and ({"notify", "indicate"} & set(char.properties)):
                await asyncio.wait_for(client.start_notify(char, session.receive), 10)
                subscribed += 1
        if not subscribed:
            raise RuntimeError("Printer has no supported status notification channel")
        log(f"Connected to {device.name} ({device.address})")
        yield session
    finally:
        try:
            await asyncio.wait_for(client.disconnect(), timeout=10)
        except Exception as exc:
            log(f"Bluetooth disconnect cleanup: {exc}")


async def diagnose(target=DEFAULT_NAME, log=print):
    async with connect(target, log) as session:
        services = [{"uuid": s.uuid, "characteristics": [
            {"uuid": c.uuid, "properties": c.properties,
             "max_write_without_response": c.max_write_without_response_size}
            for c in s.characteristics]} for s in session.client.services]
        await session.send(packet(0xA8), check_status=False)
        status = await session.query_status()
        await asyncio.sleep(0.5)
        result = {"target": target, "address": session.client.address, "status": status,
                  "status_text": status_text(status), "services": services,
                  "notifications": session.notifications}
        log(json.dumps(result, indent=2))
        return result


async def print_image(image, target=DEFAULT_NAME, darkness="normal", log=print, progress=None, *, feed_mm=DEFAULT_FEED_MM):
    data = print_job(image, darkness, feed_mm=feed_mm)
    async with connect(target, log) as session:
        await session.query_status()
        session.check()
        if session.status & 0x80:
            raise RuntimeError("Printer is already printing. Wait for it to finish.")
        log(f"Sending {image.width} × {image.height} dots…")
        await session.send(data, progress)
        log("Data sent; waiting for printer…")
        # The print buffer can continue consuming rows after BLE writes complete.
        await asyncio.sleep(2)
        deadline = time.monotonic() + 60
        while True:
            await session.query_status()
            session.check()
            if not session.status & 0x80 and session.writable.is_set():
                break
            if time.monotonic() >= deadline:
                raise RuntimeError("Data was sent, but the printer did not become ready. Check the paper before retrying.")
            await asyncio.sleep(1)
        log("Transfer complete; printer reports ready. Check the physical print.")
