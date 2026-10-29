# Building NuPrint from source

NuPrint is a **Rust + Tauri 2** desktop app plus a `nuprint` command-line tool. Both use the same Rust printer engine. The original Python app is kept as a proof of concept and protocol reference in [`poc/python`](../poc/python).

## Requirements

Rust 1.99 (installed automatically from `rust-toolchain.toml` by rustup), Node.js 26+, and on Linux the [Tauri system packages](https://tauri.app/start/prerequisites/) plus BlueZ.

## Desktop app

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

## Command-line tool

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

Printing commands accept `--device NAME_OR_ADDRESS` (default: first supported printer found), `--darkness light|normal|dark`, `--feed-mm`, `--copies` and `--allow-experimental` (needed for experimental models, and for the SC03h on Linux for now). `--preview` without `--profile` renders for the best-supported profile. Exit status: 0 confirmed, 2 sent without printer confirmation, 3 outcome uncertain, 130 cancelled, 1 error. [Printer interaction](printer-interaction.md) explains each command and option.

## Checks

These run without a printer:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cd apps\desktop; npm run typecheck; npm test
```

PDF tests skip themselves when PDFium has not been fetched. Set `NUPRINT_REQUIRE_PDFIUM=1` to make them fail instead; CI does.

[Validation](validation.md#routine-offline-checks) adds the Python reference checks, and [CONTRIBUTING.md](../CONTRIBUTING.md) lists what a pull request needs.

## Slint demo

`apps/slint-demo` is a second front end written in [Slint](https://slint.dev), driving the same `AppService` as the desktop app and CLI. It reproduces the desktop app's screens (toolbar with printer menu, text/image/PDF/test-pattern workspace, paper preview, print bar, queue drawer, settings with printers, appearance and diagnostics, and the save, remove, advanced and inspect dialogs) with its own light/dark themes and accents. It is a demonstration of how the core can sit behind another UI and ships in each release only as a preview; it has not been tried against a printer from an installer. Two things the desktop app has that it lacks: the app-icon choice only changes the header logo, not the window icon, and profile export is not offered in the advanced dialog. It keeps its own data folder (`NuPrint-SlintDemo` under the local app data folder, or `NUPRINT_SLINT_DATA`), so it never touches the installed app's printers or jobs.

It draws with Skia on the CPU. Skia's default Direct3D path on Windows let game overlays attach to the window and flickered on an HDR, high-refresh monitor; set `SLINT_BACKEND` to choose another renderer. When the window moves to a monitor with a different scale, the demo restores its previous logical size, because Slint otherwise grows the window during a title-bar drag. This workaround has not yet been confirmed on a real drag. A release build looks for `pdfium.dll` (or `libpdfium`) in `NUPRINT_PDFIUM_DIR`, in the `pdfium` folder its installer puts it in, or beside the executable; without it, PDF support shows as unavailable.

```powershell
cargo run -p nuprint-slint-demo
```

Its installers are built with [cargo-packager](https://github.com/crabnebula-dev/cargo-packager), configured under `[package.metadata.packager]` in `apps/slint-demo/Cargo.toml`. The release workflow builds them beside the Tauri installers. To build one locally, copy the platform's PDFium library and its `LICENSE` into `target/slint-pdfium` first:

```powershell
cargo install cargo-packager --version 0.11.8 --locked
New-Item -ItemType Directory -Force target\slint-pdfium
Copy-Item vendor\pdfium\win-x64\bin\pdfium.dll, vendor\pdfium\win-x64\LICENSE target\slint-pdfium
cargo build -p nuprint-slint-demo --release --locked
cargo packager -p nuprint-slint-demo --release -f nsis   # dmg or deb on the other systems
```

The installer lands in `target\release`. On macOS the release workflow builds the app bundle (`-f app`), signs it ad hoc and makes the DMG itself; see `.github/workflows/release.yml`.

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/nuprint-protocol` | Pure printer protocol drivers (SC03h / iPrint 5178), bitmap type, status decoding |
| `crates/nuprint-transport` | BLE transport (btleplug) and a scriptable mock |
| `crates/nuprint-render` | Text, image and PDF rendering to the exact print bitmap; bundled Noto fonts |
| `crates/nuprint-core` | Application service: profiles, sessions, job queue, SQLite storage, typed API |
| `crates/nuprint-cli` | `nuprint` command-line tool |
| `apps/desktop` | Tauri 2 host (`src-tauri`) and React frontend |
| `apps/slint-demo` | Demo front end in Slint over the same service (released as a preview) |
| `profiles/` | Versioned printer model profiles with evidence |
| `fixtures/` | Golden protocol vectors generated from the Python reference |
| `poc/python` | Original Python/Tk app, kept runnable as the parity oracle ([its README](../poc/python/README.md)) |
| `docs/` | Specifications, protocol evidence and validation records ([index](README.md)) |

Coding agents should start with [AGENTS.md](../AGENTS.md). Vendor APKs, decompiled sources, PDFium binaries, logs and build output are excluded from Git.
