# NuPrint

[![CI](https://github.com/wapenshaw/nuprint/actions/workflows/ci.yml/badge.svg)](https://github.com/wapenshaw/nuprint/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Print directly from a computer to small Bluetooth LE thermal printers, starting with the ZEITEL **SC03h**. No phone app, pairing or system print queue is needed.

NuPrint is a **Rust + Tauri 2** desktop app for Windows, macOS and Linux, plus a `nuprint` command-line tool. Both use the same Rust printer engine. The original Python app is kept as a proof of concept and protocol reference in [`poc/python`](poc/python).

| | Status (2026-10-13) |
| --- | --- |
| Rust engine | SC03h packets match the Python reference byte-for-byte on 40 golden job vectors. 148 offline Rust tests pass. |
| Desktop app (Windows, dev build) | Found SC03h-3DE8 and read firmware 3.0.49 with ready status. Printed two short text jobs; the printer reported ready after each and the owner reported them working. |
| Desktop app (macOS) | Bluetooth permission prompt, discovery and four short text prints on SC03h-3DE8 (dev build and a packaged debug app, Apple silicon); ready status after each, owner reported them working. |
| Packaged installers, Linux | Release installers not yet built or tested. The SC03h profile is **experimental** on Linux. |
| Python PoC (Windows) | Owner-confirmed correct 384 × 164-dot text strip on SC03h-3DE8. |
| Other models | 34 **experimental** profiles for models whose vendor-app settings are identical to the SC03h (for example X2h, X5h, X6h, X7h, X103h, SC04h, P1, D1). None has been tested on hardware. |

## Supported printers

- **SC03h** (lowercase, names starting `SC03h-`): verified on Windows and macOS; experimental on Linux.
- **Experimental models:** printers whose settings in the vendor's Android app are identical to the SC03h's. NuPrint offers them with an **Experimental** badge, and saving one asks you to confirm (`--allow-experimental` in the CLI). They use the SC03h's tested settings with their own exact name match, and a printer must still show the expected Bluetooth print service before anything is sent. If you try one, please share a diagnostics export and whether the paper looked right. For a model NuPrint doesn't list, `nuprint inspect --device NAME --output inspect.json` records its Bluetooth services without sending it any printer command. The list and the evidence behind it are in [docs/apk-printer-models.md](docs/apk-printer-models.md).
- **Everything else**, including uppercase variants such as `SC03H-` or `X6H-` (different heat settings), shows as an unknown device and is never sent print commands.

## Installing

Download the installer for your system from [Releases](https://github.com/wapenshaw/nuprint/releases), and check it against `SHA256SUMS` if you like:

| System | File |
| --- | --- |
| Windows 10/11 (x64) | `NuPrint_<version>_x64-setup.exe`, installs for the current user without admin rights |
| macOS 14+, Apple silicon | `NuPrint_<version>_aarch64.dmg` |
| macOS 14+, Intel | `NuPrint_<version>_x64.dmg` |
| Debian/Ubuntu (x64) | `NuPrint_<version>_amd64.deb`, installs BlueZ as a dependency |

The installers are not code-signed yet. On Windows, SmartScreen may warn about an unrecognised app: choose **More info → Run anyway**. On macOS, the first launch is blocked: open **System Settings → Privacy & Security** and choose **Open Anyway** for NuPrint. The `nuprint` CLI is built from source (see [Building from source](#building-from-source)).

## Using the desktop app

NuPrint is one window. On the left is what to print, on the right its paper preview with the print bar underneath. The toolbar holds the printer button, **Queue** and **Settings** (Ctrl+, or ⌘,). The queue slides in over the preview, and settings open on top. Neither moves anything in the window.

1. Turn on the printer, load thermal paper and close any phone printer app.
2. Open the printer button in the toolbar and choose **Find printers…**. In **Settings → Printers**, press **Save printer** on the found `SC03h-…` device (or a listed experimental model, after confirming). **Check connection**, in the printer menu or beside the saved printer, reads its status without moving paper. The light on the little printer shows the result in the toolbar, the menu and the printer list: green for ready, amber for a problem such as no paper, red for no connection. With several printers saved, the menu picks the one jobs go to.
3. Write a note, or open a picture or one PDF page. The preview is the exact bitmap that will be printed, repeated for each copy and followed by the extra feed as blank paper, with a millimetre ruler alongside.
4. In the print bar, choose darkness, copies and extra feed (0–12 mm), then **Print**. The bar shows how much paper the job will use. **Save as PNG** (the save icon) above the preview writes the exact bitmap to a file.

**Advanced settings.** For an experimental printer, or the SC03h, **Settings → Printers → Advanced** edits that printer's values: heat per darkness level, print speed, chunk size, pacing and every other tunable setting. Common ones have their own boxes; everything else is in the JSON below them. Edits belong to that saved printer on this computer, are always shown as **Experimental**, and never change the built-in profile. How the printer is recognised (name match, protocol) cannot be edited. **Reset to defaults** goes back to the built-in values, and **Export profile…** writes the saved settings as a complete profile file. Test with one short strip after each change. A wrong value can fail or waste paper.

After **Print**, the status bar at the bottom shows the latest job's state and data transfer, with **Cancel** while it can still be cancelled. It stays there with the outcome until the next message or **Dismiss**; **View queue** opens the full history in the **Queue** panel. Each job there shows its receipt while NuPrint keeps its content (waiting, failed or uncertain jobs); completed jobs keep none, so they show blank paper of the length they used.

Jobs go to a durable queue, so they survive restarts. If the app stops during a transfer, the job is marked **uncertain** and never resent automatically; check the paper and retry if needed. "Transfer complete · printer reports ready" means the printer accepted the data and reported ready, not that someone saw the paper.

**Settings → Appearance** picks the mode (match the system, light or dark), the accent colour (indigo, coral, ocean or teal) and the app icon (little cube, curl, ribbon or air roll). Light or dark also applies to the window's title bar. The chosen icon replaces the little cube in the Dock (macOS) or on the window and taskbar while NuPrint is open. Finder, Launchpad and shortcuts keep the little cube. All three choices are remembered.

Diagnostics can be exported from **Settings → Diagnostics**. Device names, addresses and job titles (including in the log) are redacted unless you choose to include them. Documents are never uploaded anywhere.

## Building from source

Requirements: Rust 1.99 (installed automatically from `rust-toolchain.toml` by rustup), Node.js 22+, and on Linux the [Tauri system packages](https://tauri.app/start/prerequisites/) plus BlueZ.

```powershell
# Once: fetch the pinned PDFium build used for PDF rendering (into ignored vendor/)
.\scripts\fetch-pdfium.ps1            # macOS (Apple silicon or Intel)/Linux: ./scripts/fetch-pdfium.sh

# Desktop app (dev mode with hot reload)
cd apps\desktop
npm ci
npx tauri dev

# Installer for this OS (output under target\release\bundle)
npx tauri build
```

The command-line tool:

```powershell
cargo run -p nuprint-cli -- profiles
cargo run -p nuprint-cli -- scan                      # discovery only
cargo run -p nuprint-cli -- diagnose --output output\diagnostics.json   # connects, no paper
cargo run -p nuprint-cli -- inspect --device "X6h-1A2B" --output output\inspect.json   # any device; services only

# Preview only: no Bluetooth, no paper
cargo run -p nuprint-cli -- text "Hello!" --preview output\note.png
cargo run -p nuprint-cli -- file picture.png --preview output\picture.png

# These print immediately
cargo run -p nuprint-cli -- test
cargo run -p nuprint-cli -- text "Short note" --size 32 --feed-mm 3
cargo run -p nuprint-cli -- file document.pdf --page 2 --threshold
cargo run -p nuprint-cli -- file photo.png --atkinson   # higher-contrast dithering
```

Printing commands accept `--device NAME_OR_ADDRESS` (default: first supported printer found), `--darkness light|normal|dark`, `--feed-mm`, `--copies` and `--allow-experimental` (needed for experimental models, and for the SC03h on Linux for now). `--preview` without `--profile` renders for the best-supported profile. Exit status: 0 confirmed, 2 sent without printer confirmation, 3 outcome uncertain, 130 cancelled, 1 error.

Checks that run without a printer:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cd apps\desktop; npm run typecheck; npm test
```

## Slint demo

`apps/slint-demo` is a second front end written in [Slint](https://slint.dev), driving the same `AppService` as the desktop app and CLI. It reproduces the desktop app's screens (toolbar with printer menu, text/image/PDF/test-pattern workspace, paper preview, print bar, queue drawer, settings with printers, appearance and diagnostics, and the save, remove, advanced and inspect dialogs) with its own light/dark themes and accents. It is a demonstration of how the core can sit behind another UI, not a supported product, and it has not been tried against a printer. Two things the desktop app has that it lacks: the app-icon choice only changes the header logo, not the window icon, and profile export is not offered in the advanced dialog. It keeps its own data folder (`NuPrint-SlintDemo` under the local app data folder, or `NUPRINT_SLINT_DATA`), so it never touches the installed app's printers or jobs.

It draws with Skia on the CPU. Skia's default Direct3D path on Windows let game overlays attach to the window and flickered on an HDR, high-refresh monitor; set `SLINT_BACKEND` to choose another renderer. When the window moves to a monitor with a different scale, the demo restores its previous logical size, because Slint otherwise grows the window during a title-bar drag. This workaround has not yet been confirmed on a real drag. A release build looks for `pdfium.dll` (or `libpdfium`) beside the executable, or in `NUPRINT_PDFIUM_DIR`; without it, PDF support shows as unavailable.

```powershell
cargo run -p nuprint-slint-demo
```

## Troubleshooting

- **Not found:** make sure Bluetooth is on, power-cycle the printer, keep it close and close phone printer apps that may hold the connection.
- **Connected, but no status:** power-cycle the printer and check the connection again. NuPrint never pairs or unpairs devices; removing a stale Windows pairing can help.
- **Blank paper:** the thermal side must face the print head.
- **Stops halfway:** check paper, lid and charge. NuPrint honours the printer's buffer pause/resume signals. Cancelling stops further transfer, but already buffered rows may still print.
- **macOS:** the first time you press **Find printers**, macOS asks whether NuPrint may use Bluetooth; choose **Allow**. If access was refused, **Settings → Printers** says so and offers **Open Bluetooth settings** (Privacy & Security → Bluetooth): turn NuPrint on there and find printers again.

## Repository

| Path | Contents |
| --- | --- |
| `crates/nuprint-protocol` | Pure printer protocol drivers (SC03h / iPrint 5178), bitmap type, status decoding |
| `crates/nuprint-transport` | BLE transport (btleplug) and a scriptable mock |
| `crates/nuprint-render` | Text, image and PDF rendering to the exact print bitmap; bundled Noto fonts |
| `crates/nuprint-core` | Application service: profiles, sessions, job queue, SQLite storage, typed API |
| `crates/nuprint-cli` | `nuprint` command-line tool |
| `apps/desktop` | Tauri 2 host (`src-tauri`) and React frontend |
| `apps/slint-demo` | Demo front end in Slint over the same service (not a supported product) |
| `profiles/` | Versioned printer model profiles with evidence |
| `fixtures/` | Golden protocol vectors generated from the Python reference |
| `poc/python` | Original Python/Tk app, kept runnable as the parity oracle ([its README](poc/python/README.md)) |
| `docs/` | Specifications, protocol evidence and validation records ([index](docs/README.md)) |

Coding agents should start with [AGENTS.md](AGENTS.md). Vendor APKs, decompiled sources, PDFium binaries, logs and build output are excluded from Git.

## Contributing

Bug reports, printer reports and pull requests are welcome. Reports from owners of the experimental models are especially useful, because they are the only way a profile gets promoted. See [CONTRIBUTING.md](CONTRIBUTING.md) for the development setup, the checks to run and the rules for protocol changes. Report security problems privately as described in [SECURITY.md](SECURITY.md). Everyone taking part follows the [code of conduct](CODE_OF_CONDUCT.md).

## Licence and trademarks

NuPrint is released under the [MIT License](LICENSE). The bundled Noto fonts are under the SIL Open Font License ([OFL.txt](crates/nuprint-render/assets/fonts/OFL.txt)). PDFium is downloaded separately at build time under its own licence and is not part of this repository.

NuPrint is an independent project. It is not affiliated with, endorsed by or supported by ZEITEL or the makers of the printers' vendor app. Product names are used only to identify compatible hardware. The printer protocol was implemented independently for interoperability, from hardware observations and analysis of the vendor's Android app, with the evidence recorded in [docs/protocol.md](docs/protocol.md). No vendor code or app files are included.
