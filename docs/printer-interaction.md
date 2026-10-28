# Interacting with the SC03h

This guide describes the implemented `nuprint` CLI and, for reference, the Python PoC API. It does not require the Android app, a printer driver or a virtual print server. The desktop app performs the same operations through **Printers → Find printers / Check connection** and **Print**.

## Prepare the printer

Turn on the computer's Bluetooth and the printer. Keep it close to the PC. Close the phone's printer app so it does not hold the connection. Load paper before a physical print. Select the lowercase model `SC03h-3DE8`; uppercase `SC03H` uses a different profile.

Work from the repository root. `nuprint` below means `cargo run -p nuprint-cli --` (or the built `target\debug\nuprint.exe`). Printing to the SC03h on Linux, or to any experimental model, additionally needs `--allow-experimental`.

## Discovery and diagnosis: no paper movement

```powershell
nuprint scan
nuprint diagnose
nuprint diagnose --device SC03h-3DE8 --output output\diagnostics.json --include-identifiers
nuprint scan --all
nuprint inspect --device "X6h-1A2B" --output output\inspect.json
```

Discovery lists supported names, addresses (not available on macOS), signal strength and matching profiles; `scan --all` also lists unknown devices. Diagnosis connects, enumerates services, enables notifications, sends A8 device-info and A3 status queries, prints the raw replies, optionally saves a report (identifiers redacted unless `--include-identifiers`), then disconnects. Device-info arrival is best-effort; a valid fresh status response is required for diagnostic success.

`inspect` is for printers that no profile matches, or that match one but fail its checks. It connects, lists the GATT services and characteristics with their properties and the negotiated write length, then disconnects. It writes nothing to the device: no status query, no subscription, no vendor command. It also reports which profiles would accept the device and why the others were rejected. The optional JSON report keeps the model part of the name, masks the per-unit suffix (`SC03h-****`) and omits the address unless `--include-identifiers` is given. A report plus the model's name in the vendor app is what a new experimental profile starts from; see [the APK catalog](apk-printer-models.md).

A name, address or OS device identifier may be passed to `--device`; without it the first supported printer found is used. The discovered name must still match a profile. Use an address returned by the current scan rather than copying someone else's identifier. Bluetooth addresses can change on some devices; this unit's stability across all reset modes has not been established.

Expected observations for the verified unit:

| Item | Observation |
| --- | --- |
| Name | SC03h-3DE8 |
| Firmware | 3.0.49 |
| Advertisement | AF30 service UUID |
| Connected print service | AE30 |
| Write channel | AE01, write without response |
| Status channels subscribed | AE02 and AE04; replies observed on AE02 |
| Ready status | First A3 payload byte 00 |

All short UUIDs expand to `0000xxxx-0000-1000-8000-00805f9b34fb`. The AF30/AE30 difference is real: an AE30-only scan filter would miss the advertisement.

## Preview first, without connecting

```powershell
nuprint test --preview output\test-preview.png
nuprint text "Hello from Windows" --size 24 --preview output\note.png
nuprint file photo.jpg --preview output\photo.png
nuprint file document.pdf --page 2 --threshold --preview output\page2.png
```

`--preview` saves the exact print bitmap as PNG and never invokes Bluetooth. The bitmap is 384 pixels wide and at most 4000 rows. Text uses the bundled Noto Sans (or `--mono`) on every OS; `file --rotate 90` rotates pictures and PDF pages, and `--password` opens a protected PDF. `--threshold` gives crisp text/line-art conversion; `--atkinson` gives higher-contrast dithering that clips light greys to paper and dark greys to solid ink, which can look cleaner on thermal paper; the default uses Floyd–Steinberg photo dithering. PDF page numbers start at 1, and each invocation selects one page.

## Print commands: consume paper immediately

Use these when a physical print is intended:

```powershell
nuprint test
nuprint text "Hello from Windows" --size 24
nuprint file photo.jpg --darkness normal
nuprint file document.pdf --page 2 --threshold
```

Heat presets are `light`, `normal`, `dark`. They map to 4250, 5000 and 5750 in image mode. The app renders text as an image too. Do not substitute the uppercase model's settings, and do not add heat entry to the print screen just to make output darker. A deliberate, per-printer edit lives in **Printers → Advanced** (see [printer support](printer-support.md#settings-edited-on-this-computer)); test it with one short strip.

Use `--feed-mm N` on `test`, `text` or `file` for extra paper after the image (whole numbers 0–12; default 12), and `--copies N` (up to 20). Feed is sent once, after the last copy. For example, `nuprint text "Short note" --feed-mm 3` requests about 3 mm. Zero sends no extra-feed commands, but still waits for ready. Low values can leave the last line inside the printer; reduced amounts have offline coverage and await physical calibration. Source-image margins are independent. PNG previews exclude feed; the app's paper estimate includes it.

In PowerShell, a multiline string can be passed directly to `text`:

```powershell
$note = "Shopping list`nMilk`nBread"
nuprint text $note
```

Ctrl+C cancels a CLI print; the app has Cancel on each queued or active job. The CLI exit status is 0 when the printer confirmed ready, 2 when the transfer finished without confirmation, 3 when the outcome is uncertain, 130 when cancelled and 1 on error. Cancellation stops subsequent writes, but already-buffered rows can still print. Inspect the paper before retrying a failed or cancelled job.

## Python PoC API (reference only)

The Python package in `poc/python` remains the parity reference. Run these from `poc/python` with its environment (`.\.venv\Scripts\python.exe`). This example queries only:

```python
import asyncio
from nuprint.bluetooth import diagnose

async def main():
    report = await diagnose("SC03h-3DE8")
    print(report["status_text"])

if __name__ == "__main__":
    asyncio.run(main())
```

This example sends one short note:

```python
import asyncio
from nuprint.bluetooth import print_image
from nuprint.render import text_image

async def main():
    bitmap = text_image("Hello from Windows", size=24)
    await print_image(bitmap, target="SC03h-3DE8", darkness="normal", feed_mm=3)

if __name__ == "__main__":
    asyncio.run(main())
```

`load_file(path, page=1, dither=True)` returns `(bitmap, page_count)`. `print_image()` accepts a mode-1 Pillow image with the required dimensions, a log callback and an optional `(sent_bytes, total_bytes)` progress callback. Await it to retain the connection through completion and cleanup. It raises on failures rather than silently retrying.

Do not call blocking `asyncio.run()` from a GUI event callback. Follow the existing worker/event-queue pattern. Do not share Bleak clients across event loops or issue concurrent operations against this printer.

## Connection and transfer sequence

1. Scan by supported name or requested address; verify the lowercase model.
2. Open a GATT session (uncached on Windows); find AE30/AE00 with AE01.
3. Subscribe to supported AE02/AE04 notification characteristics before querying.
4. Send A3 and await a fresh CRC-valid response. Reject paper/lid/heat/power faults or an already-printing device.
5. Send quality, energy, image mode, speed and A2 raster rows, then the selected extra-feed commands (none at zero). Use the encoder rather than assembling unrelated raw bytes.
6. Break the stream into chunks of at most `min(180, negotiated_write_limit)`; honour pause/resume and pacing. Frames may span chunks.
7. Allow a settling delay, query status until ready or timeout, and disconnect in all outcomes.

See [protocol.md](protocol.md) for exact payloads and captured replies. A ready reply after transfer is evidence of printer status, not a guarantee that a blank/incorrectly loaded roll produced a visible image.

## Diagnose by symptom

| Symptom | First check | Next useful evidence |
| --- | --- | --- |
| Printer absent | Power, proximity, PC Bluetooth, phone-app connection | Scan output and adapter/OS version |
| Found but cannot connect | Competing app/session; power-cycle | Whether GATT services can be enumerated |
| Service mismatch | Exact model casing and discovered UUIDs | Read-only service dump; no arbitrary-characteristic writes |
| Status timeout | Notification subscription and fresh connection | Raw A3 response, length/CRC, channel |
| Paper out / lid / heat / power | Correct the physical condition | Fresh diagnostic status after recovery |
| Blank output | Thermal paper orientation; correct roll; preview contains ink | Preview PNG and physical observation |
| Eight-pixel patterns or mirrored bit groups | LSB order and black-bit polarity | Golden pixel packing tests |
| Stops during long job | AE flow messages, pacing, charge/paper | Bytes submitted, last status, disconnect/error |
| Works in dev, fails when installed | Bundled PDFium, OS Bluetooth permission (macOS) | Diagnostics report, app version, OS |

Avoid repeated full-page retries. Isolate the failing layer first. Do not update firmware, change names through writable device characteristics, calibrate labels or change OS pairing as a routine protocol test.

## Record a hardware observation

Record date, app commit, OS/build, adapter if known, printer name/firmware, command/settings, raster dimensions/hash, raw status/flow replies, transfer result and a separate paper observation. Keep personal addresses and document files out of source commits; use synthetic test patterns.

The established baseline is the owner-confirmed 384 × 164-dot strip on 2026-10-12. A new test should state which additional behavior it proves; a successful connection alone does not expand the supported model list.
