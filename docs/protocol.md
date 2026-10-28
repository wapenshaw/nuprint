# SC03h protocol observations

## Primary evidence

Analyzed the supplied `apks/base.apk` with JADX 1.5.6. SHA-256:

`1BDBD04341EF684557BF1CA2024A0E6C067327E8059F1DC9D1E780FD3CA5C4F1`

The base APK contains Java/Dalvik printer models, command tables, raster conversion and transport code. The language/density split APKs are not needed to recover this protocol. Decompiled code is kept locally under `.tools/` and is not distributed with this implementation.

Relevant classes:

- `com.Utils.PrintModelUtils`: lowercase `sc03h` definition (line 156 of the locally decompiled class).
- `com.Utils.PrinterModel.DataBean`: maps constructor parameters to model settings.
- `com.lib.blueUtils.BluetoothOrder`: commands, UUIDs, checksum table and status meanings.
- `com.lib.blueUtils.PrintDataUtils`: `eachLinePixToCmdB`, raw row packing, energy, feed and print sequence.
- `com.lib.blueUtils.BluetoothUtils`: BLE services and `notifyData1` pause/resume/status handling.
- `com.lib.blueUtils.PrinterModelUtils`: density-dependent energy calculation.

These are observations for interoperability, not a claim of an official vendor specification.

## Exact model matters

The lowercase `SC03h-` profile specifies 384 pixels, 200 dpi nominal (8 dots/mm), image speed 10, text speed 10, 180-byte image writes, image energy 5000, and text energy 8000. Its `useSPP` flag is true, but the physical unit also exposes a working BLE print service. The uppercase `SC03H-` profile has different energy/speed values and is deliberately not accepted by this client.

NuPrint rasterizes both text and pictures in image mode with energy 5000. Light/dark use ±15%, matching the app's concentration adjustment. It does not use grayscale mode, compression, SPP, label calibration, firmware updates or persistent printer configuration writes. Heat/mode/speed commands configure the current print operation.

## Hardware observations (2026-10-12)

- Advertised name: `SC03h-3DE8`.
- Advertised service: `0000af30-0000-1000-8000-00805f9b34fb`.
- Actual GATT print service: `0000ae30-0000-1000-8000-00805f9b34fb`.
- Print write characteristic: AE01, write without response.
- Notifications: AE02 and AE04. Observed replies arrived on AE02.
- Additional AE03/AE05/AE10 and AE3A services are present but unused.
- Windows negotiated a 509-byte maximum write without response; the client still caps chunks at the model's 180 bytes, or the negotiated size if smaller.

Discover by name, then verify the GATT service and write characteristic. Filtering discovery only by AE30 would miss this physical unit's AF30 advertisement.

Device-info response:

```text
51 78 A8 01 0A 00 81 00 03 33 2E 30 2E 34 39 00 15 FF
```

Firmware string: `3.0.49`.

Status response before and after the test:

```text
51 78 A3 01 03 00 00 12 27 88 FF
```

The first payload byte is 00 (ready). The remaining status bytes are retained in diagnostic logs; no unsupported battery-percentage interpretation is displayed.

The client sent a 384 × 164-dot test strip. The owner confirmed the text and border printed correctly. This validates direct Windows BLE printing, the basic image-mode command sequence, checksum and raster packing on this unit. It does not establish physical reliability for every image, PDF, long job, paper condition or firmware variant.

## Frames and pixels

```text
51 78 | opcode | direction | payload length (uint16 LE) | payload | CRC8 | FF
```

Commands have direction 00; observed responses have direction 01. CRC covers only the payload, using polynomial 07, initial value 00, no reflection and no final XOR. Known checksum `crc8("123456789") = F4`. For example the app's 48-step feed command is `51 78 A1 00 02 00 30 00 F9 FF`.

| Byte offset | Length | Field |
| --- | --- | --- |
| 0 | 2 | Magic 51 78 |
| 2 | 1 | Opcode |
| 3 | 1 | Direction: 00 command / 01 observed response |
| 4 | 2 | Payload byte count N, little-endian |
| 6 | N | Payload |
| 6 + N | 1 | CRC8(payload) |
| 7 + N | 1 | FF terminator |

Total frame size is N + 8. The notification decoder's 512-byte payload ceiling is an application guard for this model, not a declared limit of the wire format's uint16 length. Status bytes are binary values; do not parse their hex display as decimal battery percentages.

Each raw A2 row holds 48 bytes (384 pixels). **LSB first within each byte; 1 means black.** Pillow's mode-1 bytes have the opposite bit order and polarity, so both are transformed. The APK's non-A4 raster expression puts pixel 0 in bit 0 and pixel 7 in bit 7. Some online examples use MSB-first; those examples do not match this APK path.

Normal print sequence:

| Opcode | Payload | Purpose |
| --- | --- | --- |
| A3 | 00 | Query current status before writing |
| A4 | 33 | App's quality-3 setting |
| AF | 88 13 | Image energy 5000, little-endian |
| BE | 00 | Image mode |
| BD | 0A | Print speed parameter 10 |
| A2 | 48 bytes per row | Uncompressed monochrome image |
| BD | 19 | Feed speed parameter 25; omitted if extra feed is zero |
| A1 | 30 00, twice by default | Configurable post-image feed; omitted at zero |
| A3 | 00 | Query status after allowing buffer to drain |

BD is a speed parameter, not a replacement for advancing paper. The implementation avoids A6 lattice/label commands since they are not required by the verified basic raster path.

## Adjustable post-print feed

The original client always appended two 48-step A1 commands: 96 steps, approximately 12 mm using the model's nominal 8 dots/mm scale. The owner reports approximately 11 mm of blank paper after printing, consistent with this explicit feed. This is distinct from blank rows inside the bitmap and the physical distance between print head and tear edge.

APK evidence: `BluetoothOrder.paper` encodes 48 steps; `PrintDataUtils.paperLe(int)` encodes an arbitrary uint16 little-endian distance with opcode A1 and payload CRC. `PrintDataUtils.feedPaper(int)` instead emits BD (speed). Do not change BD expecting a shorter feed distance.

The current client exposes `feed_mm` / `--feed-mm` / **Extra feed (mm)** as an integer 0–12. It converts nominal mm to eight steps each and emits chunks of at most 48 steps, preserving the original two-command sequence at 12. Zero omits both post-image BD and A1 commands; fresh status polling and normal completion cleanup still run. Invalid values fail before opening Bluetooth. The maximum is an application bound, not the uint16 protocol limit. Reverse feeding is not offered.

Default 12 mm retains the physically verified baseline. Other values are APK-derived and covered by offline packet/transport tests, but not yet measured on paper. At low values, the end of the image may remain inside the printer; no software setting removes the physical head-to-exit distance. PNG export excludes this mechanical feed; the GUI estimates total paper using image height plus selected feed. No automatic cropping of source whitespace is performed.

For other APK profiles, see the [model inventory](apk-printer-models.md). Feed scale/defaults are model-specific and must not be copied globally.

## Notifications, pacing and errors

APK flow-control frames:

```text
51 78 AE 01 01 00 10 70 FF  -> pause (buffer full)
51 78 AE 01 01 00 00 00 FF  -> resume
```

The parser handles notification fragmentation/concatenation and validates lengths and CRC. A full buffer blocks further writes with a bounded timeout. Pacing is conservatively 25 ms per chunk and 250 ms every approximately 4 KB. These timings are implementation choices for reliability, not asserted firmware requirements.

The APK maps status byte bits to: 01 paper out, 02 lid open, 04 overheated, 08 low power, 10 charging, 80 printing. The first four block a print. The app requires a fresh A3 reply before any print, monitors status during transfer and queries again after a settling delay. It reports data transfer plus printer-ready status, not a sensor-confirmed physical-page completion.

Uncompressed rows are used for simplicity and were physically validated. Buffer-full behavior, paper-out handling, cancellation, disconnect cleanup and raster correctness have automated tests; not all error conditions have been induced on hardware.

## Library references

- [Bleak client API](https://bleak.readthedocs.io/en/latest/api/client.html): characteristic objects, explicit write mode, negotiated limits, notifications and lifecycle.
- [Bleak troubleshooting](https://bleak.readthedocs.io/en/latest/troubleshooting.html): Windows event-loop/COM considerations. This app keeps BLE operations on a worker thread and avoids pywin32/STA imports.
- [Pillow Image API](https://pillow.readthedocs.io/en/stable/reference/Image.html): image conversion and dithering.
- [pypdfium2 API](https://pypdfium2.readthedocs.io/en/stable/python_api.html): selected-page rasterization and explicit native-resource cleanup. All GUI PDF rendering stays on the Tk thread.

Protocol authority for this implementation is the supplied APK plus the actual printer observations above. Community documentation was used only to locate likely protocol families, not as a substitute for model-specific evidence.

For executable CLI/Python examples and the exact connection lifecycle, see [printer-interaction.md](printer-interaction.md). For implementation boundaries and current timeout values, see [technical-spec.md](technical-spec.md).
