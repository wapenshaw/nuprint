---
name: sc03h-printer
description: Develop, diagnose and validate NuPrint's SC03h Bluetooth transport and raster protocol using APK evidence and Windows hardware observations. Use for this repository's printer integration work, not generic printer setup or unrelated Bluetooth devices.
---

# SC03h printer work

Work from the NuPrint repository root. This skill lives three directories below it. The Rust implementation lives in `crates/nuprint-protocol` (codec: `src/iprint.rs`), `crates/nuprint-transport` (BLE) and `crates/nuprint-core` (session, profiles, jobs); SC03h values are in `profiles/sc03h-iprint-384.json`. The Python PoC in `poc/python` is the evidence/parity baseline, and `poc/python/tools/gen_fixtures.py` produces the golden vectors in `fixtures/sc03h/`.

Read the reference relevant to the task:

- [Protocol and evidence](../../../docs/protocol.md): packet formats, model values, CRC, pixel order, flow/status meanings and captured replies. Read before modifying transport or encoding.
- [Printer interaction guide](../../../docs/printer-interaction.md): CLI/Python operations, connection lifecycle and bounded hardware verification.
- [Technical specification](../../../docs/technical-spec.md): current module ownership and the proposed job-queue/render contracts. Read for structural changes.
- [Rust + Tauri specification](../../../docs/rust-tauri-spec.md) and [printer support contract](../../../docs/printer-support.md): read for porting or adding printer profiles. SC03h-specific defaults must not become global defaults for other models.
- [Validation plan](../../../docs/validation.md): offline checks, physical test cases and evidence recording.

## Procedure

1. Identify whether the issue is discovery, service discovery, notifications, framing, image conversion, transfer pacing or paper output. Use `cargo run -p nuprint-cli -- scan`, then `diagnose` for connection failures; neither prints. Use `--preview` for layout or raster issues. The desktop app's activity log and diagnostics export carry the raw RX notifications.
2. Preserve the exact lowercase model distinction, AF30 advertisement / AE30 service distinction, LSB-first black bits, image energy 5000 and MTU-aware chunk cap. Obtain evidence before changing model behavior. Never substitute generic ESC/POS commands or assumptions from another cat-printer model.
3. Change the smallest responsible module. Validate offline using the captured packet vectors (`crates/nuprint-protocol/tests/python_parity.rs`) and the simulated-printer service tests (`crates/nuprint-core/tests/service_flow.rs`). Preserve fresh status gating, buffer pause/resume, bounded waits and disconnect cleanup.
4. When a physical test is within the user's authorized scope and the printer is available, start with one short strip. Record the transfer result separately from the owner's paper observation. Stop after the requested test or a failure; investigate before retrying. Never replay a partially sent job automatically.
5. Update the protocol/evidence documentation for new discoveries, label unverified inferences, and report remaining physical-validation gaps.

Physical printing is not required for documentation-only work. Existing authorization should carry forward; this skill does not add a repeated approval gate. Avoid changing pairing, firmware or Windows printer settings just to diagnose the application.

For APK analysis, the base APK already contains the relevant classes listed in the protocol notes. Keep decompilation/downloads in ignored `.tools/`; commit the independently implemented behavior, short observations and test vectors, not the vendor source or APK.
