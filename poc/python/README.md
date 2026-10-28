# NuPrint Python proof of concept

The original Windows-only NuPrint app (Python 3.11+, Bleak, Pillow, pypdfium2, Tk). It is superseded by the Rust + Tauri app at the repository root, and kept because it is the **verified reference**:

- A Windows hardware check on 2026-10-12 confirmed a 384 × 164-dot text/alignment strip printed on a lowercase SC03h unit (firmware 3.0.49).
- `tools/gen_fixtures.py` generates the golden vectors in `../../fixtures/sc03h/` that the Rust protocol crate must match byte-for-byte.

Do not add features here. Change it only to keep it runnable or to regenerate fixtures.

## Running

Run from this directory in PowerShell:

```powershell
.\setup.ps1                       # or: uv sync --extra dev --locked
.\.venv\Scripts\python.exe -m nuprint gui

# Discover / query only, without printing
.\.venv\Scripts\python.exe -m nuprint scan
.\.venv\Scripts\python.exe -m nuprint diagnose --output output\diagnostics.json

# Preview only: no Bluetooth connection or paper use
.\.venv\Scripts\python.exe -m nuprint text "Hello" --preview output\preview.png

# These print immediately
.\.venv\Scripts\python.exe -m nuprint test
.\.venv\Scripts\python.exe -m nuprint file document.pdf --page 2 --threshold

# Offline tests, fixtures and the PyInstaller build (dist\NuPrint\NuPrint.exe)
.\.venv\Scripts\python.exe -m pytest -q
.\.venv\Scripts\python.exe tools\gen_fixtures.py
.\build.ps1
```

Printing commands accept `--device NAME_OR_ADDRESS` (default `SC03h-3DE8`; if several are nearby, select one by its full name or Bluetooth address), `--darkness light|normal|dark` and `--feed-mm 0`–`12` (default 12). `Start NuPrint.cmd` opens the built executable. The log is written to `%LOCALAPPDATA%\NuPrint\nuprint.log`. Runtime logs and diagnostic JSON can include the printer's name and Bluetooth address; review or redact them before sharing.

| Module | Responsibility |
| --- | --- |
| `nuprint/protocol.py` | CRC-8 framing, LSB-first raster packing, SC03h print sequence |
| `nuprint/bluetooth.py` | Bleak/Windows GATT transport, fresh status, flow control, cleanup |
| `nuprint/render.py` | Text, image and PDF-page rasterization |
| `nuprint/gui.py` | Tk desktop app with background Bluetooth operations |
| `nuprint/cli.py` | CLI and preview-only commands |
