"""SC03h wire format, independently implemented from local APK observations.

See docs/protocol.md. Pixels are LSB-first, 1 = black. SC03H is a different
profile; do not silently apply these heat settings to other models.
"""

from dataclasses import dataclass
import struct
from PIL import Image

WIDTH = 384
MAX_HEIGHT = 4000
DEFAULT_FEED_MM = 12
MAX_FEED_MM = 12
MAGIC = b"\x51\x78"


def crc8(data: bytes) -> int:
    crc = 0
    for value in data:
        crc ^= value
        for _ in range(8):
            crc = ((crc << 1) ^ (0x07 if crc & 0x80 else 0)) & 0xFF
    return crc


def packet(opcode: int, payload: bytes = b"\x00", flags: int = 0) -> bytes:
    if len(payload) > 65535:
        raise ValueError("Packet payload is too large")
    return MAGIC + bytes((opcode, flags)) + struct.pack("<H", len(payload)) + payload + bytes((crc8(payload), 0xFF))


@dataclass(frozen=True)
class Frame:
    opcode: int
    flags: int
    payload: bytes


class FrameDecoder:
    """Reassemble fragmented notifications and validate CRC before acting."""

    def __init__(self):
        self.buffer = bytearray()

    def feed(self, data: bytes) -> list[Frame]:
        self.buffer.extend(data)
        frames = []
        while True:
            start = self.buffer.find(MAGIC)
            if start < 0:
                self.buffer[:] = b"\x51" if self.buffer[-1:] == b"\x51" else b""
                break
            del self.buffer[:start]
            if len(self.buffer) < 6:
                break
            size = int.from_bytes(self.buffer[4:6], "little")
            # Notifications for this model are small. Bound bogus length fields.
            if size > 512:
                del self.buffer[0]
                continue
            if len(self.buffer) < size + 8:
                break
            payload = bytes(self.buffer[6:6 + size])
            if self.buffer[7 + size] != 0xFF or crc8(payload) != self.buffer[6 + size]:
                del self.buffer[0]
                continue
            frames.append(Frame(self.buffer[2], self.buffer[3], payload))
            del self.buffer[:size + 8]
        return frames


def status_text(value: int) -> str:
    states = [(1, "Out of paper"), (2, "Cover open"), (4, "Print head too hot"),
              (8, "Battery low"), (16, "Charging"), (128, "Printing")]
    return ", ".join(label for bit, label in states if value & bit) or "Ready"


def raster_rows(image: Image.Image):
    if image.mode != "1" or image.width != WIDTH or not 1 <= image.height <= MAX_HEIGHT:
        raise ValueError(f"Expected a monochrome {WIDTH}-pixel-wide image, 1–{MAX_HEIGHT} rows")
    # Pillow mode 1 is MSB-first and 1=white; printer is LSB-first and 1=black.
    reverse_invert = bytes(int(f"{value ^ 255:08b}"[::-1], 2) for value in range(256))
    packed = image.tobytes().translate(reverse_invert)
    for offset in range(0, len(packed), WIDTH // 8):
        yield packed[offset:offset + WIDTH // 8]


def feed_steps(feed_mm: int) -> int:
    """Nominal SC03h feed distance; physical calibration remains approximate."""
    if type(feed_mm) is not int or not 0 <= feed_mm <= MAX_FEED_MM:
        raise ValueError(f"Extra feed must be a whole number from 0 to {MAX_FEED_MM} mm")
    return feed_mm * 8


def print_job(image: Image.Image, darkness: str = "normal", *, feed_mm: int = DEFAULT_FEED_MM) -> bytes:
    steps = feed_steps(feed_mm)
    energy = {"light": 4250, "normal": 5000, "dark": 5750}.get(darkness)
    if energy is None:
        raise ValueError("Darkness must be light, normal or dark")
    rows = list(raster_rows(image))  # Validate completely before sending anything.
    commands = [packet(0xA4, b"\x33"), packet(0xAF, struct.pack("<H", energy)),
                packet(0xBE, b"\x00"), packet(0xBD, b"\x0a")]
    commands.extend(packet(0xA2, row) for row in rows)
    if steps:
        commands.append(packet(0xBD, b"\x19"))
        # Preserve the verified two 48-step commands at the default distance.
        while steps:
            amount = min(48, steps)
            commands.append(packet(0xA1, struct.pack("<H", amount)))
            steps -= amount
    return b"".join(commands)
