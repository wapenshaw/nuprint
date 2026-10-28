# Validation and release plan

NuPrint is implemented in [Rust + Tauri](rust-tauri-spec.md); the Python app in `poc/python` is the verified reference. The first table records the Python baseline; Rust results are under [Rust workspace](#rust-workspace-2026-10-12). Multi-model acceptance follows [printer-support.md](printer-support.md).

## Existing evidence

Baseline source: the Python PoC as of 2026-10-12 (pre-release history).

| Area | Result | Scope |
| --- | --- | --- |
| Offline suite | 30 tests passed | Packet vectors, CRC, raster polarity/order, decoding, model heat/feed sequence, bounds, image/PDF conversion and mocked BLE behavior |
| Dependency resolution | `uv lock --check` passed | Lock consistent with declared dependencies |
| Discovery | SC03h-3DE8 found | Name scan; advertisement AF30 |
| GATT | Connected and enumerated | AE30, AE01 write, AE02/AE04 subscribe |
| Device/status | Valid replies | Firmware 3.0.49; ready before/after transfer |
| Physical print | Owner confirmed correct text and border | One short 384 × 164-dot strip |
| Source GUI | Opened, preview inspected, connection check returned ready | Default-size Windows window and background BLE operation |
| Packaged GUI | EXE launched and window inspected | Local Windows x64 build; not a clean-machine matrix |

The 30-test result is a dated baseline, not a fixed expected count for future development. Buffer-full and fault handling currently have simulated tests; real fault scenarios below remain unverified unless a later result is recorded.

## Routine offline checks

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p nuprint-cli -- test --preview output\test-preview.png
cd apps\desktop; npm run typecheck; npm test; cd ..\..

# Python reference, from poc\python
uv sync --extra dev --locked
.\.venv\Scripts\python.exe -m pytest -q
git diff --check
```

Use synthetic inputs, golden frames and captured non-sensitive status replies. Offline tests must not scan Bluetooth, connect to a live printer or consume paper. Tests for new behavior should check observable requirements, not merely reproduce implementation formulas.

## Target automated coverage

The workspace has locked Rust tests, formatting/Clippy, frontend type/component tests, profile validation and Python-generated golden vectors. CI and native builds on Windows/macOS/Linux are still to be added. Keep the Python oracle for regenerating vectors when reference behavior changes.

| Area | Required cases before releasing that feature |
| --- | --- |
| Protocol | Known command frames, CRC check vector, fragmented/concatenated replies, malformed lengths/CRC, unknown opcodes, black/white/edge pixels |
| Transport | Negotiated chunk limit, explicit write mode, disconnect cleanup, fresh status, paper/heat faults before/during transfer, buffer pause/resume and timeout, cancellation while blocked |
| Discovery | Exact model distinction, AF30 advertisement, duplicate names/OS identities, unsupported GATT service, ambiguous profiles, missing required notify characteristic |
| Multiple models | Profile revisions, second width/bit-order/status policy, rerender on profile switch, capabilities hiding unsupported options, no probing writes to unknown devices |
| Rendering | EXIF rotation, alpha-white composition, aspect ratio, long words, line wrapping, supported glyphs/shaping, dithering/threshold, source and decoded-size limits |
| PDF | Page ranges/order, invalid selection, corruption, passwords, page rotation, forms where supported, oversized dimensions and native-resource cleanup |
| Queue | Atomic admission, snapshot isolation, sequential dispatch, cancel in each phase, crash/restart recovery, no automatic replay, stale events, retries create a new attempt |
| Persistence | Schema migrations/rollback, missing assets, content hashes, quota/retention, out-of-disk handling, deterministic design round trips |
| UI | Main-thread access, stale-render rejection, controls enabled correctly, error paths, dirty preview and source changes, close while printing |
| CLI handoff | Caller access control, framing/version mismatch, malformed/bounded input, job-ID mapping, duplicate submission protection, unavailable worker |

## Physical validation matrix

Run the smallest applicable physical case when hardware work is authorized. Record paper output separately from transport/status results. Do not perform the entire matrix for every source edit.

| Case | Synthetic input / procedure | Pass criteria | Current evidence |
| --- | --- | --- | --- |
| Basic note | Short text, numbers, left/right labels and border | Legible, correct orientation/alignment | Verified |
| Raster bit order | Black pixels at x=0, 7, 8, 383 plus asymmetric markers | Correct columns and edges | Golden tests; dedicated physical pattern pending |
| Photos | Small gradient and synthetic grey image | Recognizable shading, no dropped bands | Pending |
| PDF range | Three numbered pages; select 1 and 3 | Exactly selected pages, correct order | Pending feature |
| Repeated jobs | Five separately submitted short numbered notes | No mixing, loss or duplicates | Pending |
| Long page | Gradually increase toward 4000 rows | No missing rows/buffer overrun; length measured | Pending |
| Trailing feed | Same short end-marked raster at default 12 mm, then selected reduced/zero feed | Measure blank advance and last-line accessibility; raster remains identical; distinguish margins and head-to-exit distance | Original feed used in verified strip; reduced/zero physical measurement pending |
| Pause/resume | Record natural AE flow notifications on a bounded job | Sender pauses and resumes without loss | Mocked only |
| Paper/lid fault | Query a naturally safe unloaded/open state; restore before test | Blocks transfer; actionable status; recovers after correction | Mocked only |
| Connection loss | One short synthetic job with controlled loss | Reports uncertain outcome; never auto-replays | Pending |
| Cancel | Cancel a bounded long synthetic job | No new writes after cleanup; buffered remainder explained | Mocked cleanup only |
| Heat/power fault | Observe only if encountered naturally | Clear state and no continued print writes | Mocked only; do not induce overheating |
| QR/barcodes | Several specified module widths/quiet zones | Repeated successful scans from printed paper | Pending feature |

Keep roll and heat settings within the verified profile. Do not deliberately exhaust the battery or overheat the head to fill a checklist.

## GUI and packaging release checks

- Inspect text and file modes at default/minimum window sizes and 100/150/200% DPI. Check preview scrollbars, progress, error text, Print and Cancel visibility.
- Exercise keyboard navigation and readable focus indicators; validate screen-reader behavior before claiming accessibility support.
- Build from a clean clone using the lock file. The APK, `.tools/`, system Java and existing build outputs must not be required.
- Launch the packaged app on the supported Windows x64 matrix, including a PC without separately installed Python. Exercise BLE, PDF and image paths to catch bundled native-library/import issues.
- Check cancellation and exit while connecting/transferring; verify Bluetooth becomes available to a new session afterwards.
- Verify log rotation, source-content exclusion, diagnostics export, storage quota and retention behavior as implemented.
- For installers, test install/update/uninstall and preservation of user designs. Verify actual signing status and include third-party notices.

For the Rust release additionally:

- Validate a packaged SC03h short print on Windows 11 x64, macOS 14+ Apple Silicon and Ubuntu 24.04 x64 before claiming those initial targets. Record adapter/firmware and actual OS builds.
- Exercise macOS Bluetooth denial/regrant and opaque device identity; Linux BlueZ permissions and X11/Wayland; Windows adapter off/on and WebView2 handling. Never require the ordinary Linux app to run as root. Check the write size on a real BlueZ stack: rbaron's `catprinter` had to force an MTU exchange because its Python library reported a fixed 23, while btleplug reads BlueZ's `MTU` characteristic property (only on newer BlueZ versions; unverified here) and otherwise NuPrint uses the safe 20-byte size. Record the negotiated size from the diagnostics log, and whether a long job is noticeably slower on a stack that falls back.
- Test sleep/wake, app restart and unexpected disconnect on each backend. Use transport-reported or conservatively bounded write sizes rather than assuming the Windows negotiated size everywhere.
- Run PDF/image/text paths from a clean native package with no Python runtime; include native PDFium, fonts and licenses.
- Add a second physical printer through the driver/profile interfaces and record every advertised capability on its own support matrix. A fake profile is not hardware evidence.
- Compare identical canonical raster inputs against Python SC03h packet fixtures; separately review changed text/PDF renderer output.
- Use a documented native/manual macOS UI test path where Tauri WebDriver coverage is unavailable. Browser-only tests are insufficient.

## Feed-control update (2026-10-12)

Offline suite: 46 tests passed after adding feed bounds, zero/reduced/default packet checks, unchanged raster checks and simulated transport verification of ready polling without feed. Invalid feed is rejected before opening Bluetooth. No physical print was performed for this update; do not treat packet tests as proof of reduced-feed paper output. The preceding 30-test table remains the historical baseline.

Source GUI: feed values, length estimate, mocked Print handoff and invalid-input rejection checked without Bluetooth. Default/minimum window layouts inspected. Windows executable rebuilt and launched; packaged feed control visible. This is local Windows UI/package evidence, not cross-platform or reduced-feed hardware validation.

## Rust workspace (2026-10-12)

Pre-release working tree, workspace version 0.2.0, Rust 1.98.0, Windows 11 Pro 10.0.26300.

| Area | Result | Scope |
| --- | --- | --- |
| Rust tests | 147 passed (2026-10-13) | Protocol unit tests (11); Python parity (3 tests covering CRC/commands, captured replies and all 40 job vectors byte-for-byte); render text/image/PDF (13/19/7, PDF with the pinned PDFium); core unit tests (57: profiles including the experimental copies, prefix overlap and ordering, store, migrations including the schema 1 → 2 upgrade, jobs, generated bindings); service flow with a simulated SC03h-family printer (35: status gating, faults, pause/resume, disconnect → uncertain, cancel, restart recovery, idempotency, retry, failed checks logged, experimental opt-in, edited settings reaching the wire bytes and surviving restart/reset, locked identity and 18 certain-to-break values refused by name, profile export); transport (1); CLI (1: report name masking) |
| Lint | `cargo fmt --check` and `clippy -D warnings` clean | Whole workspace including the desktop host |
| Frontend | `tsc` passed; 41 Vitest tests passed | Mock backend; not native app behavior |
| Python PoC | 46 tests passed after moving to `poc/python` | Unchanged reference behavior |
| CLI previews | `test`, `text`, `file` (PNG) previews written; bad `--feed-mm`/`--copies` rejected before rendering | No Bluetooth |
| Desktop dev build | Window launched via `npx tauri dev`; owner-driven session below | Debug build, not packaged |
| Discovery/check | SC03h-3DE8 found and saved; firmware 3.0.49; status ready; write AE30/AE01, chunk 180 bytes (link limit 509) | Read-only check, no paper |
| Print transfer | Two short "Hello from NuPrint!" text jobs, 4770 and 4760 bytes: fresh ready status before sending, ready after, clean disconnect, recorded as "Transfer complete · printer reports ready" | Started by the owner from the GUI; owner reported the prints working (no detailed paper inspection recorded) |

Not yet validated for the Rust app: detailed paper inspection, packaged installers (bundled PDFium), Linux, pictures/PDF on paper, long jobs, real pause/resume, faults, cancellation on hardware and reduced-feed calibration. The SC03h profile's `verified_basic` level on Windows rests on the Python evidence plus this owner report for short text from the Rust dev build.

## macOS (2026-10-13)

Pre-release `macos-support` branch, workspace version 1.0.0, Rust 1.98.0, Node 26.10.0. Mac16,7 (Apple silicon), macOS 27.0.1 (26A434), Bluetooth chipset BCM_4388C2 (PCIe).

| Area | Result | Scope |
| --- | --- | --- |
| Offline checks | `fmt`/`clippy -D warnings` clean; 137 Rust tests and 38 Vitest tests passed | Includes the new inspection, busy-message and permission tests |
| PDFium | `fetch-pdfium.sh` fetched and verified `mac-arm64`; the 7 PDF render tests passed | Not printed on paper |
| Bluetooth permission | Dev build launched from a terminal multiplexer whose daemon has no app bundle: no prompt, scans found 0 devices silently (now reported as missing permission). Dev build under Ghostty: prompt for Ghostty, allowed. Packaged debug `NuPrint.app`: prompt named NuPrint on first Find printers, allowed | Denial and regrant through System Settings not yet exercised on hardware |
| Discovery/inspection | SC03h-3DE8 found and saved (Experimental at the time); inspection listed 2 services; write AE30/AE01, chunk 180 bytes (link limit 182) | Before the fix, a second inspection failed with "not found" because btleplug drops the peripheral after a disconnect; fixed on that branch |
| Print transfer | Four short "Hello from NuPrint!"/"MAC TEST" text jobs, 6618–8522 bytes: three from the dev build, one from the packaged debug `.app` (bundled PDFium). Each had a fresh ready status (00) before sending, ready after, a clean disconnect and "Transfer complete · printer reports ready" | Started by the owner from the GUI; owner reported the prints working (no detailed paper inspection recorded) |
| Connection check | Not run on macOS; firmware not read here (3.0.49 on Windows for the same unit) | |

The first scan right after granting permission in the packaged app found 0 devices; the next scan found the printer. Not yet reproduced or explained. Not yet validated on macOS: Check connection and firmware, Intel Macs, a signed/notarized release build, permission denial/regrant on hardware, pictures/PDF on paper, long jobs, real pause/resume, faults and cancellation on hardware. The SC03h profile's `verified_basic` level on macOS rests on these four owner-reported short text prints, one from a packaged debug build.

## Evidence record format

Add dated entries with:

```text
App commit / package version:
OS edition and build / Bluetooth adapter:
Printer name / firmware / profile:
Scenario and authorization scope:
Synthetic input ID, raster dimensions and SHA-256:
Settings, page order and copy count:
Transfer result / status / flow notifications:
Physical observation (who confirmed it):
Pass, fail or unverified; remaining limits:
```

Keep local captures and diagnostic files in ignored `output/`. Commit concise non-sensitive findings and fixtures when useful. A fixture cannot establish physical success on a different firmware or printer model.
