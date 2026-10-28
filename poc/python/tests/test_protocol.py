import struct
import pytest
from PIL import Image
from nuprint.protocol import FrameDecoder, crc8, packet, print_job, raster_rows, status_text


@pytest.mark.parametrize("opcode,payload,expected", [
    (0xA3, b"\x00", "5178a30001000000ff"),
    (0xA4, b"\x33", "5178a40001003399ff"),
    (0xA1, b"\x30\x00", "5178a10002003000f9ff"),
    (0xBE, b"\x01", "5178be0001000107ff"),
])
def test_apk_golden_commands(opcode, payload, expected):
    assert packet(opcode, payload).hex() == expected


def test_crc_standard_check_vector():
    assert crc8(b"123456789") == 0xF4


def test_real_printer_response():
    frames = FrameDecoder().feed(bytes.fromhex("5178a301030000122788ff"))
    assert len(frames) == 1
    assert frames[0].opcode == 0xA3
    assert frames[0].flags == 1
    assert frames[0].payload == b"\x00\x12\x27"


def test_fragmentation_noise_concatenation_and_crc_recovery():
    a = packet(0xAE, b"\x10", flags=1)
    b = packet(0xAE, b"\x00", flags=1)
    corrupt = bytearray(a)
    corrupt[-2] ^= 1
    decoder = FrameDecoder()
    assert not decoder.feed(b"garbage" + a[:1])
    assert not decoder.feed(a[1:7])
    result = decoder.feed(a[7:] + corrupt + b)
    assert [f.payload for f in result] == [b"\x10", b"\x00"]


def test_decoder_rejects_oversize_header_and_recovers():
    result = FrameDecoder().feed(bytes.fromhex("5178a301ffff") + packet(0xA3, flags=1))
    assert len(result) == 1


def test_black_pixel_lsb_order_and_white_polarity():
    image = Image.new("1", (384, 2), 1)
    image.putpixel((0, 0), 0)
    image.putpixel((7, 1), 0)
    image.putpixel((383, 1), 0)
    first, second = list(raster_rows(image))
    assert first == b"\x01" + bytes(47)
    assert second == b"\x80" + bytes(46) + b"\x80"


def test_all_black_row():
    assert list(raster_rows(Image.new("1", (384, 1), 0))) == [b"\xff" * 48]


def test_job_heat_mode_and_feed_match_lowercase_model():
    frames = FrameDecoder().feed(print_job(Image.new("1", (384, 3), 1)))
    assert [f.opcode for f in frames] == [0xA4, 0xAF, 0xBE, 0xBD, 0xA2, 0xA2, 0xA2, 0xBD, 0xA1, 0xA1]
    assert frames[1].payload == struct.pack("<H", 5000)
    assert frames[3].payload == b"\x0a"
    assert frames[-1].payload == b"\x30\x00"


@pytest.mark.parametrize("image", [Image.new("L", (384, 2)), Image.new("1", (380, 2)), Image.new("1", (384, 4001))])
def test_invalid_raster_rejected(image):
    with pytest.raises(ValueError):
        print_job(image)


def test_status_bits_from_apk():
    assert status_text(0) == "Ready"
    assert status_text(5) == "Out of paper, Print head too hot"
    assert status_text(128) == "Printing"


@pytest.mark.parametrize("distance,expected", [(0, []), (1, [8]), (3, [24]), (6, [48]), (7, [48, 8]), (12, [48, 48])])
def test_feed_control_preserves_raster_and_bounds_commands(distance, expected):
    image = Image.new("1", (384, 2), 1)
    image.putpixel((383, 1), 0)
    original = FrameDecoder().feed(print_job(image))
    frames = FrameDecoder().feed(print_job(image, feed_mm=distance))
    assert frames[:6] == original[:6]  # Heat, mode, speed and both image rows.
    assert [int.from_bytes(f.payload, "little") for f in frames if f.opcode == 0xA1] == expected
    if distance == 0:
        assert len(frames) == 6  # No feed speed or zero-length movement command.
    else:
        assert frames[6].opcode == 0xBD and frames[6].payload == b"\x19"


@pytest.mark.parametrize("distance", [-1, 13, 65536, 1.5, float("nan"), True, "3", None])
def test_invalid_feed_is_rejected(distance):
    with pytest.raises(ValueError, match="Extra feed"):
        print_job(Image.new("1", (384, 1), 1), feed_mm=distance)
