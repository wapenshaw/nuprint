<p align="center">
  <img src="apps/desktop/src-tauri/icons/128x128@2x.png" width="112" alt="NuPrint's little cube icon">
</p>

<h1 align="center">NuPrint</h1>

<p align="center">
  Print notes, pictures and PDF pages from your computer to a pocket Bluetooth thermal printer.<br>
  No phone app, no pairing, no printer driver.
</p>

<p align="center">
  <a href="https://github.com/wapenshaw/nuprint/releases/latest"><img src="https://img.shields.io/github/v/release/wapenshaw/nuprint?label=download" alt="Latest release"></a>
  <img src="https://img.shields.io/badge/Windows%20%C2%B7%20macOS%20%C2%B7%20Linux-555" alt="Windows, macOS and Linux">
  <a href="https://github.com/wapenshaw/nuprint/actions/workflows/ci.yml"><img src="https://github.com/wapenshaw/nuprint/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
</p>

<p align="center">
  <img src="docs/images/app-text.png" alt="NuPrint with a short to-do note on the left and its paper preview on the right">
</p>

## What it does

- **Write, open, print.** Type a note with headings, or open a picture or one page of a PDF.
- **See the exact print.** The preview is the bitmap the printer will receive, dot for dot, with a millimetre ruler and a paper estimate.
- **Talks to the printer directly.** NuPrint finds the printer over Bluetooth LE and sends the job itself. Nothing to install on the printer side.
- **A queue you can trust.** Jobs survive a restart, can be cancelled, and are never resent on their own if a transfer was interrupted.
- **Private.** Documents never leave your computer, and diagnostics hide device names and job titles unless you include them.
- **Yours to style.** Light or dark, four accent colours and four app icons.

<p align="center">
  <img src="docs/images/app-picture-dark.png" width="49%" alt="A picture dithered for thermal paper, in dark mode with the teal accent">
  <img src="docs/images/settings-printers.png" width="49%" alt="Settings showing a saved, verified SC03h printer and the Find printers button">
</p>

## Download

Get the installer for your system from the [latest release](https://github.com/wapenshaw/nuprint/releases/latest).

| System | Installer |
| --- | --- |
| Windows 10 and 11 | `NuPrint_<version>_x64-setup.exe` (no admin rights needed) |
| macOS 14+ on Apple silicon | `NuPrint_<version>_aarch64.dmg` |
| macOS 14+ on Intel | `NuPrint_<version>_x64.dmg` |
| Debian and Ubuntu | `NuPrint_<version>_amd64.deb` |

The installers aren't code-signed yet, so Windows and macOS warn on first launch. The [user guide](docs/user-guide.md#installing) shows how to get past the warning.

Each release also has a preview of NuPrint drawn with [Slint](https://slint.dev) instead of a web view, in the `nuprint-slint_<version>_…` installers. It is less tested, and it keeps its own printers and queue.

## Get printing

1. Turn the printer on and close its phone app.
2. In NuPrint, open the printer button in the toolbar and choose **Find printers…**, then **Save printer**.
3. Write a note or open a picture or PDF page.
4. Press **Print**.

The [user guide](docs/user-guide.md) covers the queue, advanced printer settings, appearance and troubleshooting.

## Printers

| Printer | Status |
| --- | --- |
| ZEITEL **SC03h** (names starting `SC03h-`) | Tested on Windows and macOS. Experimental on Linux. |
| 34 models with the same vendor-app settings, such as X2h, X5h, X6h, SC04h, P1 and D1 | Experimental: offered with a badge and a confirmation, not yet tested on hardware |

Other printers show up as unknown devices and are never sent print commands. If you own one of the experimental models, a [printer report](https://github.com/wapenshaw/nuprint/issues/new/choose) is the fastest way to get it supported. See [supported printers](docs/user-guide.md#supported-printers) for the details.

## Documentation

- [User guide](docs/user-guide.md): installing, printing, the queue, settings and troubleshooting
- [Command line](docs/printer-interaction.md): the `nuprint` tool for scanning, previews and printing
- [Building from source](docs/building.md): development setup, checks and the repository layout
- [All documentation](docs/README.md): architecture, printer protocol, validation records and the roadmap

## Contributing

Bug reports, printer reports and pull requests are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md), report security problems as described in [SECURITY.md](SECURITY.md), and follow the [code of conduct](CODE_OF_CONDUCT.md).

## Licence

NuPrint is released under the [MIT License](LICENSE). The bundled Noto fonts are under the SIL Open Font License ([OFL.txt](crates/nuprint-render/assets/fonts/OFL.txt)). PDFium is downloaded at build time under its own licence and is not part of this repository.

NuPrint is an independent project, not affiliated with, endorsed by or supported by ZEITEL or the makers of the printers' vendor app. Product names only identify compatible hardware. The printer protocol was implemented independently for interoperability, from hardware observations and analysis of the vendor's Android app; the evidence is in [docs/protocol.md](docs/protocol.md), and no vendor code or app files are included.
