# NuPrint cross-platform application specification

Status: Rust + Tauri V1 in progress: printing text, pictures and single PDF pages, the job queue, saved printers and diagnostics are implemented (Windows dev build); designs/templates, multi-page ranges and packaging are pending. Grounded in the working Python v0.1 client. Updated 2026-10-12. See [Rust + Tauri architecture and migration](rust-tauri-spec.md) and [multi-printer support](printer-support.md).

## Product outcome

A Windows, macOS or Linux user can make a readable note, label, photo strip or document excerpt, see exactly what will fit on the selected printer's roll, and print through Bluetooth without a phone. Frequent layouts are reusable. Jobs survive ordinary app restarts without silently printing duplicates. V1 covers direct Bluetooth printing from NuPrint's desktop app and CLI.

The first printer is the lowercase SC03h; other Bluetooth printers are added through independently validated protocol adapters and model profiles. Initial product test targets are Windows 11 x64, macOS 14+ Apple Silicon and Ubuntu 24.04 LTS x64. Publish support per OS/backend, model and firmware; a shared codebase does not establish hardware compatibility. Broader OS/architecture support follows testing. BLE is the first transport; Classic Bluetooth/SPP requires a separate future backend.

No account, subscription or network connection is required for normal editing and printing. Local installation/update downloads are separate from the print workflow.

## Current baseline and planned releases

| Capability | v0.1 now | Planned destination |
| --- | --- | --- |
| Direct BLE discovery/status/printing | Implemented; one unit verified | Harden and retain |
| Text, image and one PDF page | Implemented | Editing, page ranges and batch jobs |
| Exact bitmap preview, PNG export, darkness | Implemented | Zoom, physical sizing, crop, presets |
| Cancel current transfer | Implemented | Queue-aware cancellation and recovery |
| Adjustable post-print feed | Implemented: SC03h 0–12 nominal mm; reduced feed awaits paper calibration | Profile-specific control, saved per printer |
| Rust + Tauri desktop on three OS families | Not implemented | M0–M2 parity and platform validation |
| Saved printer choice, preferences | Not implemented | M3 |
| Durable queue and job history | Not implemented | M3 |
| Templates, QR/barcodes, reusable designs | Not implemented | M3 |
| Multiple model/protocol profiles | SC03h only | M4: second physical model and extension acceptance |
| Gap-sensing die-cut labels | Not implemented | Profile-specific feature after hardware evidence |

## Primary workflows

### First print

1. Open the app to an editor with a printer selector and a sample note.
2. Find nearby supported printers; show name and signal, with an explicit empty/error state.
3. Select a printer and check its status. Explain nearby-device contention in plain language when the phone holds the connection.
4. Show a small calibration-strip preview. Pressing Print sends exactly one strip.
5. Ask the user to inspect the output only when calibrating or validating hardware; normal successful jobs do not open a confirmation dialog.

Acceptance: a new user prints using the GUI without a shell or Android app. The verified SC03h path does not require pairing or administrator access; other profiles expose any documented pairing requirement explicitly. Ordinary discovery must find the AF30-advertising unit. Permission denial and unsupported devices have distinct, actionable states.

### Compose a note

Enter text with font, size, alignment, line spacing and margins. Start with plain text plus heading/body styles; defer full rich-text editing. Wrap by rendered glyph width. Show missing-glyph/fallback information before printing, and validate shaping for any language advertised as supported.

Acceptance: preview and printed raster are identical; editing after preview invalidates the queued snapshot or creates a new snapshot. Long text has a visible length estimate and is split only at user-approved page boundaries.

### Print a picture or document excerpt

Open supported raster files or paste a clipboard image. Rotate in 90-degree steps, crop, choose fit-width or actual size, adjust brightness/contrast, and choose photo shading (Floyd–Steinberg), high-contrast shading (Atkinson, which clips highlights to paper and shadows to solid ink) or crisp thresholding for text. Preserve aspect ratio unless the user explicitly stretches an image. Composite transparency onto white.

For PDFs, show thumbnails, page selection/ranges, crop and fit controls. Preview each selected page at the selected model's printable dot width (384 on SC03h). Full A4 pages can be unreadable on narrow paper; show the actual result and let the user crop an excerpt. PDF passwords remain in memory and are never stored in job metadata.

Acceptance: selected pages and copies print in the displayed order. Invalid ranges, corrupt files and unsupported encryption produce actionable errors before job admission. The app never silently truncates overlong content.

### Reuse a label or template

Create a fixed-width canvas with automatic or fixed height. Add text, image, line, rectangle, QR and supported barcode blocks. Use millimetres in the editor, with a pixel grid available for precision. Support selection, move, resize, duplicate, alignment, undo/redo and keyboard navigation. Save a named template with field placeholders such as title, date or price.

Initial templates target continuous paper, including continuous adhesive rolls. The word label does not imply physical gap detection. Die-cut media stays unavailable for a profile until its sensor/calibration workflow is tested.

Acceptance: a saved template reopens without layout drift. Variables expand before preview and validation. QR/barcode quiet zones and minimum module widths cannot be clipped silently. Codes must scan from physical prints in the release validation set.

### Manage jobs

Expose **Extra feed (mm)** beside print settings. Keep it separate from design margins, page/copy spacing and media calibration. Save the preference per printer and snapshot the effective distance into each job; show it in the total paper estimate. For SC03h, start with whole millimetres 0–12, default 12 to preserve the reference behavior. Zero requests no added movement after the final raster; explain that the final line may still be inside the printer. Reduced distances need physical measurement before advertising a calibrated minimum.

For multi-page/copy jobs, apply the trailing feed once after the final page of the final copy. Any inter-page/copy gap is a separate explicit setting included in the estimate; never silently append the default 12 mm to every page. Media/profile requirements override unsupported choices visibly. Do not add feed to a cancelled/failed job automatically.

Pressing Print snapshots the current design and settings and appends a job. Show title, target device, page/copy count, estimated paper length and status. Support cancel, retry as a new job, remove, clear history and pause dispatch between jobs. The active transfer exposes byte progress; it is not labelled physical page progress.

Queue pause does not issue an invented printer pause opcode. An active job finishes or is explicitly cancelled. A lost connection after a write may have printed part of the page; mark that outcome uncertain and let the user choose whether to submit a new copy.

Acceptance: jobs are serialized for a printer. Rapid repeated clicks do not enqueue accidental duplicate copies from one submission action. A restart cannot automatically replay a partly transmitted job. Reprinting a completed item requires retained content or reselecting its source.

### Select a different printer

Save multiple printers with model/profile and verification status. The picker distinguishes verified models, experimental models and unsupported nearby devices. Changing the target rerenders the source design at the new width/resolution and refreshes available settings. Existing queued jobs keep their original target/profile snapshot.

Acceptance: a second physically tested model works through the same editor and queue without SC03h-specific UI changes. Identical names, ambiguous profiles and different transport types never silently select a protocol. Users can route jobs to different devices; simultaneous hardware dispatch is deferred until tested.

## Screen and interaction specification

| Area | Main content | Important states |
| --- | --- | --- |
| Workspace | Source/editor left; paper preview right; Print and settings adjacent | Empty, dirty, rendering, ready, invalid |
| Printer panel | Saved devices, discovery, Check connection, model/firmware/profile support | Permission denied, adapter off, scanning, candidate, unsupported, ambiguous, experimental, ready, fault |
| Queue | Ordered jobs, page/copy details, progress and actions | Held, queued, preparing, transferring, waiting, complete, failed, uncertain, cancelled |
| Templates | Searchable saved layouts and create/duplicate/delete | Empty library, unsaved edits, missing assets |
| History | Metadata; optional retained content for reprint | Cleared content, failed, completed, uncertain |
| Settings | Printer preference, media, defaults, retention | Valid saved settings; migrations; reset defaults |
| Diagnostics | App/build version, service/status report, export | No device, connection failure, protocol mismatch |

Use familiar desktop controls and a restrained paper-oriented interface. Keep normal workflows free of packet hex and implementation settings. Advanced diagnostics can expose the protocol details. Success and errors need text labels, not colour alone.

Keyboard targets: Ctrl+O open, Ctrl+S save design, Ctrl+P preview/submit, Ctrl+Z/Ctrl+Y undo/redo, with native Cmd equivalents on macOS; Escape dismisses transient dialogs. Accessible labels and logical tab order are required. Progress and failure messages must be readable without opening logs. Test scaling and keyboard/accessibility behavior on every shipped OS, including 100%, 150% and 200% Windows DPI.

## Defaults and limits

These are proposed application limits, not printer hardware capabilities. Width, media, heat and raster bounds are profile-specific; this table gives the initial SC03h values. Other models must not inherit these values without evidence:

| Setting | Default / initial bound |
| --- | --- |
| Printable width | 384 dots, approximately 48 mm |
| Media | Continuous roll; nominal 58 mm paper is not a 58 mm print area |
| Heat | Normal 5000; light 4250; dark 5750. The print screen offers these presets only; Printers → Advanced edits a saved printer's values deliberately (experimental, per printer) |
| Extra feed after job | SC03h: nominal 12 mm default, integer 0–12; per-printer preference, separate from raster margins |
| Raster height | 4000 rows per page until longer transfers are validated |
| Copies | 1; range 1–20 with total paper estimate |
| Queue capacity | 100 jobs or 256 MiB retained job data, whichever is reached first |
| Import limits | 50 MiB per source, 100 PDF pages, 40 million decoded source pixels |
| Preview | Fit-width view; 100% pixel view and physical-size estimate |
| Completed content | Print data deleted by default; metadata and a first-page preview (`thumbnail.png`) kept for 30 days, until the job or history is removed |
| Failed/interrupted content | Retain for recovery up to 7 days, with visible clear action |
| Auto-start / background printing | Off; opt-in only |

Reject exceeding limits with a useful message before allocating the full raster or accepting the job. Users can crop/select fewer pages. Account for transformed dimensions, decoded size and total copies, not only file size.

## Privacy and packaging

Persist preferences/designs locally. Do not upload images, text, PDF contents or Bluetooth identifiers. Diagnostic export excludes print content by default and lets the user review identifying fields. No cloud telemetry is required for V1.

Ship native packages built and tested for each target: Windows installer, macOS app/DMG and supported Linux package(s). Preserve user designs during upgrades and present an explicit choice for data deletion during uninstall. Signing/notarization and release distribution require configured publisher identities; do not label unsigned artifacts as signed or trusted. The Rust release does not require Python.

## Delivery plan and exit criteria

### M0–M2: Rust port and desktop parity

- Create the Rust workspace and TypeScript/React Tauri shell; retain the runnable Python reference.
- Port pure SC03h protocol behavior and compare packet vectors for identical bitmap inputs.
- Restore text/image/PDF preview, diagnostics, printing and cancellation with Rust-owned rendering.
- Exit: build and packaged short-print evidence on Windows, macOS and Linux; no Python runtime in the shipped app.

### M3: everyday printing and reusable layouts

- Persist printer preferences, durable jobs and explicit uncertain outcomes.
- Add image transforms, PDF ranges, clipboard import, copies and paper estimates.
- Add versioned design files, block editor, template library and placeholders.
- Add QR/barcodes after raster/module sizing and physical readability tests.
- Add useful starter layouts: note, checklist, address, price label and photo strip.
- Exit: queue recovery, template round-trip fidelity, undo/redo, glyph rendering and physical code-scanning acceptance pass.

### M4–M5: multi-model support and cross-platform release

- Add a second physical printer through a reviewed protocol/profile implementation and record its support matrix.
- Complete accessibility, DPI, upgrade/uninstall and retention verification.
- Publish tested OS/adapter/firmware matrix and troubleshooting instructions.
- Exit: all applicable rows in [the validation plan](validation.md) have recorded results; no claims depend solely on successful BLE writes.

## Explicitly deferred

Simultaneous multi-printer dispatch, Classic/SPP/USB transports, cloud accounts, mobile apps, native Office document rendering, OCR, AI generation, firmware updates, arbitrary raw commands, and automatic die-cut calibration. New model profiles are in scope when backed by evidence; universal printer compatibility is not promised.
