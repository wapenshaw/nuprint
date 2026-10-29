# Engineering backlog

Open follow-up work, with resolved and outdated entries explicitly marked. Before acting on an item, check it against [AGENTS.md](../AGENTS.md), especially honest completion wording, never replaying partially sent jobs and the SC03h protocol invariants. This review records work; it does not implement the fixes.

Sources:

- **Simplify** — deliberately left out of the post-1.0.0 simplification pass (2026-10-13), because each item would change generated bindings, job-outcome handling or user-facing wording.
- **Rust review** / **Tauri review** — findings from reviewing the code on 2026-10-13 against the repository skills [`rust-best-practices`](../.claude/skills/rust-best-practices/SKILL.md) and [`tauri-v2`](../.claude/skills/tauri-v2/SKILL.md).

Priority is a suggestion: **P1** is worth doing next, **P2** is a clear improvement, **P3** is optional polish.

## Follow-up review — 2026-10-13

Reviewed the pre-release source of 2026-10-13 using both repository skills linked above, with the SC03h skill as the protocol reference. Scope: Rust protocol/transport/render/core/CLI, Tauri commands and configuration, React state/IPC, existing tests and backlog claims. No Bluetooth operations, physical prints, native UI changes or release builds were performed. **Reproduced** below means an offline probe demonstrated the stated behavior; **source-traced** means the affected path was inspected, with the proposed race/failure test still to add.

Validation:

- `cargo fmt --all --check`: passed.
- `cargo test --workspace --exclude nuprint-desktop --locked`: 138 tests passed, including PDF rendering and Python parity; no doctests exist.
- `cargo clippy --workspace --exclude nuprint-desktop --all-targets --all-features --locked -- -D warnings`: passed. Windows emitted incremental-cache access-denied notes.
- Full-workspace Clippy was attempted but blocked in the desktop build script by a file-in-use error while staging the Windows PDFium resource. The running app/processes were left alone; desktop compilation is **not validated by this review**.
- Frontend `npm run typecheck` and `npm test`: passed, 41 tests.
- Six temporary offline probes reproduced the behaviors described under R1–R4 and R6. They were removed after use so this change contains only backlog documentation. The scenarios below specify the regression tests to retain when fixing these issues. Existing passing tests do not cover these failures.

### New findings

#### R1 · P1 · Redacted diagnostics still export identifying log text — resolved

[`AppService::diagnostics_report`](../crates/nuprint-core/src/service.rs) redacts selected structured fields but copies `self.logs()` unchanged. `admit` logs the job title and printer name; inspection also logs the advertised name. Titles can contain a document filename or the first line of typed text. `app.pdf_error` also retains PDFium search paths from [`PdfEngine::load`](../crates/nuprint-render/src/pdf.rs), and nested error details are not sanitized.

**Fixed:** log text that names a printer, document or advertised device is wrapped in private markers where it is written (`private()` in `service.rs`). The screen, log file and `logs()` strip the markers; the diagnostics export replaces the marked text with `<redacted>` unless identifiers are requested, so it also works for printers and jobs removed since. `pdf_error` loses its folder names. A new log line that includes a name or title must use `private()`. Test: `redacted_diagnostics_leave_no_printer_name_or_title_anywhere`. A later review found three gaps, now closed: a title holding the end marker ended the span early, the rename-mismatch error quoted the new device name and an edited profile's name, and edited profile names and error details were exported as-is (`redacted_diagnostics_hide_names_in_errors_edits_and_odd_titles`). Original reproduction: save a mock printer named `SC03h-PRIVATE`, hold the queue, enqueue a job and serialize `diagnostics_report(false)`. The supposedly redacted report still contains that name in its log. Fix with a defined export schema/redaction policy for logs, errors and paths, preferably structured content-free log fields. Test sentinel names, document titles and local paths across the **whole** report, including logs for printers/jobs already removed; explicit identifier opt-in must remain intentional.

#### R2 · P1 · Job settings persist PDF passwords and source text — resolved

[`render_preview` → `enqueue`](../crates/nuprint-core/src/service.rs) serializes the entire `SourceSpec` into `settings_json`. That includes `Pdf.password`, despite its [API contract](../crates/nuprint-core/src/api.rs) saying it is never persisted, and all of `TextSpec.text`. [`Store::drop_content`](../crates/nuprint-core/src/store.rs) removes raster files but leaves this JSON in the job row.

**Fixed:** jobs now store `persisted_settings`, which drops the typed text and the password (kind, sizes, page and adjustments stay); schema 3 strips both keys from existing rows (`json_remove`) and backs the old store up first as `state.v2.bak`. SQLite free pages, the WAL and that backup can still hold old bytes until they are overwritten or deleted, and password-protected PDFs remain supported (the password lives in memory only). Tests: `saved_job_settings_leave_out_passwords_and_typed_text`, `upgrading_from_schema_2_removes_saved_text_and_passwords`. Original finding: probes confirmed that serializing a PDF source includes a password sentinel, and that a completed mock text job retains its full text in SQLite after `content_retained` becomes false. Use a separate persisted settings type that excludes passwords and source content; retry already uses saved raster pages. Include migration cleanup for existing rows, document the limits of deleting data from existing SQLite/WAL/backups, and add an encrypted-PDF enqueue/restart test plus a completed-text retention test. This is distinct from the metadata title policy.

#### R3 · P1 · Queue persistence failures do not stop printing — resolved (progress updates excepted)

[`run_job` and `set_state`](../crates/nuprint-core/src/service.rs) log failed attempt inserts/state updates and continue into printer I/O. Attempt progress/outcome persistence errors are also discarded. The UI can receive a state that never reached SQLite. If durable state stays queued, [`next_queued`](../crates/nuprint-core/src/store.rs) can select it again; restart recovery can also mistake an attempted print for an unsent queued job.

**Fixed:** `Store::claim_queued` makes the `queued` → `connecting` transition and the attempt record in one transaction before any Bluetooth I/O. If it cannot be saved, nothing is sent, the job stays queued, and the queue pauses for this run (`pause_queue_for_storage_error`) instead of retrying in a loop. If the final state cannot be saved, the queue pauses and the job stays in its in-progress state, which a restart turns into Uncertain. Failures saving mid-transfer progress are logged only: the claimed in-progress state already protects against a replay. Tests: `a_job_that_cannot_be_recorded_is_never_sent` (SQLite trigger), `a_job_is_claimed_or_cancelled_never_both`. Original reproduction: on an isolated test database, add SQLite triggers using `RAISE(ABORT, ...)` to reject attempt inserts and active-state updates, then resume a held mock job. It still sends print bytes and reaches completed, with zero recorded attempts. Require a durable attempt/claim before sending; propagate persistence failures, stop dispatch and preserve uncertainty after any possible submission. Regression tests must inject failures before sending, during progress and at finalization, then restart and verify no automatic replay. Never report a durable transition as successful when its write failed.

#### R4 · P1 · A failed first write is treated as definitely unsent — resolved (cancellation and timeout remain unit-untested)

[`PrinterSession::send`](../crates/nuprint-core/src/session.rs) increments `print_bytes_submitted` only after a write returns success. If the first write times out or returns an error after delivery may have begun, [`classify`](../crates/nuprint-core/src/jobs.rs) sees zero and returns `Failed` rather than `Uncertain`. Cancellation during that write likewise produces `Cancelled` without the partial-output flag. A failed return does not establish that the printer received no data.

Reproduction: use the existing mock simulator with `disconnect_after_rows: Some(1)`. The write hook consumes a raster row and fails the first print chunk; the resulting job is `Failed` with zero recorded bytes. **Fixed:** `PrinterSession` now counts a print write that was started but not confirmed (`print_bytes_at_risk`), and `classify` uses it, so a failed first write is `Uncertain`; `bytes_sent` still shows only confirmed bytes. Test: `a_failed_first_print_write_is_uncertain_not_unsent`. Original note: track “submission may have started” separately from confirmed byte counts and preserve honest cancellation/uncertainty wording. Test first-write error, timeout and cancellation as well as failure before any print-write call. Actual delivery behavior on each OS remains unverified.

#### R5 · P1 · Two desktop instances can own the same queue — resolved

[`AppService::start`](../crates/nuprint-core/src/service.rs) opens the configured store, runs restart recovery and starts a dispatcher unconditionally. Its hardware/active-job locks are process-local. The [desktop builder](../apps/tauri/src-tauri/src/lib.rs) has no single-instance guard, and the [store](../crates/nuprint-core/src/store.rs) has neither an exclusive owner lock nor an atomic queued-to-active claim.

**Fixed:** `Store::open` takes an exclusive `state.lock` (`File::try_lock`, released by the OS on exit or crash) before migrating or recovering, and refuses with `ErrorCode::DataDirInUse`. The desktop app then starts further windows in `window-2`…`window-8` folders next to the main one, each with its own saved printers and queue (title shows `NuPrint (2)`), so two windows for two printers work. They do not share printers or jobs, and two windows must not use the same physical printer at once. On macOS, a second copy needs `open -n`. Tests: `a_data_folder_has_one_owner_at_a_time`, `a_second_service_cannot_share_a_running_data_folder`. `AppService::shutdown` now also waits for the dispatcher to exit. Original finding: a second launch against the same data directory can run recovery over the first instance's live jobs and race its dispatcher; the never-replay guarantee is not protected across processes. Establish exclusive ownership of a persistent data directory before recovery, with a clear second-launch behavior. Test two services/processes sharing a temporary directory: only the owner may recover or dispatch, and a live job must not be relabeled by the second opener. This is about shared desktop state; the CLI currently uses its own in-memory store.

#### R6 · P2 · Connection checks overwrite concurrent printer edits — open, reproduced

[`check_printer`](../crates/nuprint-core/src/service.rs) loads a full `PrinterRow` before awaiting Bluetooth work, then upserts that old row with firmware/last-seen changes. A settings/profile edit made during the check can be lost; a deleted printer can also be recreated by the upsert.

Reproduction: delay mock connection by two seconds, start a check, set feed to zero after one second, then await the check. Saved feed returns to 12. Update only diagnostic columns on an existing row, or use a revision-aware merge. Cover settings changes, custom-profile save/reset and printer deletion while a check is pending.

#### R7 · P2 · Cancelling connection establishment bypasses disconnect cleanup — open, source-traced

[`PrinterSession::open`](../crates/nuprint-core/src/session.rs) and `inspect_device` cancel by dropping the `transport.connect` future. In [`BleTransport::connect`](../crates/nuprint-transport/src/ble.rs), the peripheral can already be connected while `BleLink::open` awaits service discovery/notifications. Dropping that future bypasses its error branch's explicit disconnect; `BleLink::drop` only aborts the notification forwarder.

Give connection acquisition cancellation-aware ownership and bounded cleanup, including an OS connection that finishes late. Add a staged transport test that cancels **after physical connection but before returning a Link**, then asserts disconnect before another action can take ownership. The existing delayed-connect mock test cancels before this phase and does not establish real BLE cleanup.

#### R8 · P2 · Cancelling a queued job races its dispatcher claim — resolved

[`dispatch_loop`](../crates/nuprint-core/src/service.rs) reads a queued row, releases the store lock and only then sets `active` in `run_job`. A concurrent `cancel_job` can see no active job and persist `Cancelled` in that gap. The dispatcher then uses its stale row, overwrites cancellation and proceeds. These operations can interleave across Tokio workers even though there is no explicit await between the read and `run_job`.

**Fixed:** claim and cancel are checked SQL transitions (`claim_queued`, `cancel_unstarted`), so exactly one wins; the dispatcher marks the job active before claiming, and `cancel_job` re-checks the active job after a failed cancel, so a cancel that loses the race still stops the transfer. Tests: `a_job_is_claimed_or_cancelled_never_both`, `cancelling_a_queued_job_prints_nothing`. A deterministic barrier between selection and claim is not injectable without a test hook; the store-level test covers both orders. Original proposal: make the claim and cancellation decision atomic, with a checked state transition rather than a detached row snapshot. Add a deterministic barrier test between selection and claim: cancelling there must result in zero print writes. Coordinate this with R3 and R5 rather than adding another independent lock.

#### R9 · P2 · Initial UI snapshot can outrun the event subscription — open, source-traced

[`AppProvider`](../apps/tauri/src/state/AppStore.tsx) starts `subscribe(...)` but does not await its completion before loading the initial queue snapshot. Events emitted after that snapshot but before native listener registration are lost; buffering only helps once registration is complete. Subscription rejection is silently swallowed, leaving an apparently live UI without updates.

Await listener readiness before snapshot loading, retain buffering/sequence reconciliation, and expose a subscription failure with recovery. Test a delayed/rejected subscribe promise and a job transition in the gap. Also preserve unlisten cleanup when the provider unmounts during registration.

#### R10 · P2 · Debounced feed saves can revert a newer darkness choice — open, source-traced

[`WorkspaceProvider::setFeedText`](../apps/tauri/src/state/WorkspaceStore.tsx) captures the current density in a 400 ms timer. `setDensity` immediately persists the new density but neither clears nor replaces that timer. Changing feed and then darkness within 400 ms lets the old timer write the previous density back; local controls can still display the newer choice until the printer is reselected.

Persist one current settings snapshot per printer, or cancel/rebuild pending saves when either field changes. Use fake timers to change feed then density within the debounce interval and assert the final saved values match both controls. Cover printer/profile switches and clearing the feed field with a pending save.

### Outdated entries reconciled

| Earlier claim/item | Status at this review |
| --- | --- |
| “None of it is implemented” | **Outdated.** The backlog now distinguishes open, resolved and corrected entries. |
| Core chooses completion policy | **Resolved in behavior.** `PrinterDriver::plan_job` supplies `ExchangePlan.completion`; SC03h selects `StatusConfirmedReady`. No extra trait getter is required. |
| Add `ErrorCode::AmbiguousProfile` | **Enum already present; original addition obsolete.** It is not currently emitted. Registry validation rejects overlapping built-in prefixes and custom profiles do not participate in discovery. Revisit runtime ambiguity only if matching rules change. |
| Probe PDFium at startup | **Resolved.** The frontend calls `get_app_info`; `AppService::app_info` loads/caches the PDF engine result and returns availability/error information. |
| Workspace contains no `unsafe` | **Outdated.** macOS authorization in `ble.rs` and Dock icon updates in the desktop host use documented native `unsafe` calls. Scope any forbid policy to crates without these boundaries. |
| There are 28 app commands | **Outdated.** There are 36 registered commands; explicit app-command ACL work remains open. |
| Packaging is entirely unverified | **Partially outdated.** A macOS Apple silicon packaged debug app has a confirmed short-text print; signed release/installers and Linux remain unverified. See [validation](validation.md#macos-2026-10-13). |
| CLI can use edited profiles through the desktop database | **Incorrect.** CLI creates `ServiceConfig { data_dir: None, ... }` and has no profile-management/data-directory option. CLI access remains proposed. |
| Disconnect wording requires a separate owner-approval gate | **Outdated process note.** The phase-aware wording bug remains open; it can be fixed within an authorized bug-fix task under AGENTS.md. This review only records it. |

Earlier pedantic/missing-doc lint counts below are historical estimates from the first review, not refreshed measurements. The new Clippy pass used the normal configured lints with all features, not an added pedantic/missing-doc sweep.

## Restructuring

- **P2 · Merge the duplicate render/API types** (Simplify). `nuprint-render` and `nuprint-core::api` each define overlapping parameter types. Merging them regenerates `apps/tauri/src/bindings`, so commit the bindings in the same change.
- **P2 · Restructure job execution** (Simplify; `crates/nuprint-core/src/service.rs`). `execute_job` runs the plan; `run_job` and `jobs::classify` decide outcomes. Preserve uncertainty and restart rules, and address R3/R4 before cosmetic restructuring.
- **Resolved · Driver-owned completion policy** (previously P3). `PrinterDriver::plan_job` already supplies `ExchangePlan.completion`; see the reconciliation table above.
- **P3 · Slim the `Transport` trait** (Simplify; `crates/nuprint-transport/src/lib.rs`): reconsider `kind`, `backend_id`, the `max_write_len` parameter and `manufacturer_ids`.

## Performance

The Rust skill's rule is "don't guess, measure": profile a release build before and after each of these changes.

- **P2 · Keep blocking I/O off the async workers** (Simplify, refined by Rust review). Preview rendering and PDF page counting already run through `spawn_blocking`. These still run synchronously inside async Tauri commands, on Tokio worker threads:
  - `import_source` reads the whole file with `fs::read`;
  - preview PNG and diagnostics export use `fs::write`;
  - profile export writes synchronously, and job-preview responses load stored pages and encode PNGs on async workers;
  - every store call takes the SQLite mutex;
  - every log line reopens the log file.
  Move the file reads and writes behind `spawn_blocking` or `tokio::fs`, and keep a persistent log-file handle.
- **P2 · Remove the extra query per job** (Simplify; resolved): `list_jobs` issued one additional query per job; `Store::list_jobs_with_pages` now loads the pages in one query.
- **P3 · Use one transaction for `clear_history`** (Simplify).
- **P3 · Preview caching** (Simplify): measure whether decoding the same source again for each preview request costs enough to cache it.
- **P3 · Reconnect without scanning** (Simplify): reconnect to a saved printer without a fresh scan (`AppService::connect_session`). This needs hardware validation on each OS.
- **P3 · Avoid copying the PNG for every preview request** (Tauri review): `preview_response` in `apps/tauri/src-tauri/src/lib.rs` clones the whole PNG into each response body. Previews are small, so do this only if profiling shows it matters.

## Error handling and wording

- **P1 · Stop matching on error text** (Simplify; resolved). `run_job` checked `!e.message.contains("check")` to decide whether to add uncertain-job guidance, which missed "Check the paper" and repeated it. It now skips only `ErrorCode::OutcomeUncertain`, whose messages carry their own guidance. Test: `a_printer_that_stays_busy_is_uncertain_with_one_reminder`.
- **P1 · Phase-aware "Bluetooth disconnected" message** (Simplify; resolved). `TransportError::Disconnected` (`crates/nuprint-core/src/error.rs`) always said "Some paper may already have printed; check before retrying.", even before any print-write call. It now says "Bluetooth disconnected."; `run_job` adds the paper reminder once the job is uncertain, that is once print data may have arrived. Tests: `a_disconnect_before_print_data_does_not_warn_about_paper`, `disconnect_mid_transfer_is_uncertain_and_never_replayed`.
- **P1 · Define recovery policies for poisoned mutexes** (Rust review; still open). Production `Mutex::lock().unwrap()` calls remain in `service.rs`, `session.rs` and `ble.rs`. A panic can poison shared state and stop later commands or the dispatcher. Render engines and the profile registry already recover locks. Review each protected invariant before choosing recovery or a structured error; blindly recovering every lock or replacing it with a non-poisoning mutex does not repair corrupted state. Supervise dispatcher failure and preserve uncertain-job handling.
- **P3 · Document panic contracts and internal invariants** (Rust review; symbol corrected). Review `encode_png`, raster/pattern buffer assumptions and `iprint::command` (the earlier `iprint::frame` reference is stale). In particular, the public `command` helper accepts arbitrary payload slices and panics above `u16::MAX`; callers needing validation can use `frame_with_direction`. Document public preconditions and `# Panics` sections rather than calling every panic impossible. The earlier review counted 20 missing sections; that count was not refreshed.

## Lints and tooling

- **P2 · Declare workspace lints in `Cargo.toml`** (Rust review). Today the lint level comes only from the `-D warnings` command line in AGENTS.md. Suggested `[workspace.lints]`, with each crate opting in through `[lints] workspace = true`:
  - `rust`: `unsafe_code = "forbid"` only in crates without native unsafe boundaries (the old workspace-wide rationale is outdated), `future-incompatible = "warn"`, and `missing_docs = "warn"` per library crate once the docs item below lands;
  - `clippy`: `all = "deny"`, plus selected pedantic lints. Historical counts from the first review: `needless_pass_by_value` (6), `trivially_copy_pass_by_ref` (3), `redundant_closure_for_method_calls` (4), `match_same_arms` (2) and `wildcard_imports` (3). Re-measure before enabling these.
- **P2 · Checked integer conversions at the persistence boundary** (Rust review). The protocol crate has no lossy casts. Core uses `as` between `usize`, `u32` and `i64` for SQLite counts, sizes and timestamps: `store.rs`, `service.rs`, `jobs.rs`, and `now_ms` casting a `u128` to `i64`. Replace these with `try_from` and enable `cast_possible_truncation` and `cast_sign_loss` for core. In render, the float-to-integer casts follow bounds checks; keep them, and say so in a comment where the bound isn't adjacent.
- **P3 · Run clippy with `--all-features`** (Rust review). `nuprint-protocol` has a `ts` feature, and the skill's standard command checks all features. Add the flag to the AGENTS.md command, and to CI when there is one.
- **P3 · Justify the shared test-helper `allow`** (Rust review). `crates/nuprint-render/tests/common/mod.rs` uses `#![allow(dead_code)]`. That is correct for a module shared by several test binaries, where `#[expect]` would fail in binaries that use every helper, but it needs a one-line reason comment.

## Documentation

- **P3 · Public API docs, starting with the smallest crates** (Rust review). The earlier review reported roughly 700 undocumented public items and 85 missing `# Errors` sections; those counts were not refreshed. Start with `nuprint-protocol` and `nuprint-transport`, which define the extension points for new printer models, and add doc-test examples for `MonoBitmap` and frame building. The current test run confirms there are no doctests.
- **P3 · `#[must_use]` on pure constructors and getters** (Rust review). The earlier pedantic pass suggested about 67; re-measure and apply them where ignoring the result is a real mistake.

## Tests

- **P3 · Split tests that cover several behaviours** (Rust review). Test names already describe behaviour, as the skill asks. A few cover two behaviours in one test, for example `rejects_wrong_dimensions_and_bad_feed`, `unsupported_and_corrupt_input`, `retention_and_preferences`, `rotation_and_height_limit` and `empty_and_invalid_input`. Splitting them makes failures easier to diagnose. Keep the protocol golden-vector tests as they are, since they compare against Python output.

## Tauri hardening and distribution

- **P2 · Explicit permissions for app commands** (Tauri review; still open, command count updated). `capabilities/default.json` grants only event listen and unlisten, but app commands are not listed in any capability. Without an app manifest, all 36 registered app commands are available to local app windows/webviews by default. Declare them with `tauri_build::Attributes::new().app_manifest(AppManifest::new().commands(&[...]))` in `build.rs`, then grant the generated `allow-*` permissions in `default.json`. Test permitted calls from `main` and rejection from an unauthorized test window. This closes a defense-in-depth gap, not evidence of a current remote exploit; see [Tauri's app-command default and AppManifest guidance](https://v2.tauri.app/security/capabilities/).
- **P2 · Code signing and notarization** (Tauri review; open, evidence corrected). No release signing is configured. Plan signing together with release/install validation. The macOS packaged debug app has short-text hardware evidence, while signed/notarized release builds, Windows installers and Linux remain unverified; see [validation](validation.md). An auto-updater remains a separate product decision.

## Reviewed and already consistent with the skills

Recorded so later reviews don't repeat this work:

- **Tauri:**
  - `main.rs` only calls `run()`, and `lib.rs` holds the builder with `mobile_entry_point`.
  - The library declares all three crate types.
  - Every command is registered, is async, takes owned arguments and returns `Result<_, AppError>`; `AppError` serialises as a structured `{code, message, detail}`, which is better than the skill's string example.
  - State is accessed as `State<'_, AppService>`, matching `.manage()`.
  - The frontend imports from `@tauri-apps/api/core` and cleans up its listener.
  - The capability is scoped to the `main` window.
  - The CSP is strict and `freezePrototype` is on.
  - Native dialogs run in Rust, so no dialog or fs permission is granted.
  - Paths come from `app.path()`.
  - **Deliberate deviations:**
    - A single global event channel carries job progress. A per-command `Channel` doesn't fit queued background jobs.
    - The custom `nupreview` protocol serves snapshot PNGs from memory and retained job pages from the store. Source paths stay in Rust; `AppInfo.data_dir` is deliberately exposed for diagnostics, so the earlier blanket statement that no paths cross IPC was too broad.
- **Rust:**
  - **Corrected:** two macOS native call sites use `unsafe` with safety comments; there is no basis for the earlier blanket “no unsafe” claim.
  - `anyhow` is used only in the CLI binary; the libraries use `thiserror`.
  - `let … else` and `?` are used throughout.
  - The earlier review reported no `redundant_clone` or `needless_collect` findings; those extra lints were not separately re-run in this review.
  - Dynamic dispatch (`Arc<dyn Transport>`, `dyn PrinterDriver`, `EventSink`) appears only where runtime selection or the mock needs it.
  - There are no untracked `TODO` comments.
  - Rustfmt's import grouping (`group_imports`) still needs nightly rustfmt, so it is not adopted.

## Printer models

- **P2 · Next experimental profiles: GB01, GB02 and GT01** (model research, 2026-10-13). The 34 SC03h-identical profiles are in place as experimental. Next on [the shortlist](apk-printer-models.md#shortlist-for-first-experimental-profiles) are GB01, GB02 and GT01, which community projects confirm print with the same framing but which need their own APK energy, chunk, delay and feed values. Each needs:
  - its own profile, with an exact name prefix and no widened matcher;
  - offline tests;
  - a read-only **Check connection** on a real unit before it can go beyond experimental.
- **P3 · Reproducible catalog extraction.** The full-field APK extraction (`--no-replace-consts` decompile, overload-aware constructor parsing) lives only in the ignored `.tools/` folder. If the catalog needs regenerating for a new APK version, consider committing the parser. It reads a user-supplied decompile and contains no vendor code.

## Edited printer settings

- **P2 · Import an exported profile** (Advanced settings). Export writes a complete profile file, but nothing reads one back. An import would validate it exactly like an edit and apply it to one saved printer; promoting a tested file to a built-in profile stays a reviewed source change.
- **P3 · Command-line access** (Advanced settings; premise corrected). `nuprint` currently uses an in-memory store and cannot access desktop custom profiles. Consider `profile show/set/reset/export` or validated profile-file input. Any future `--data-dir` must respect persistent-store ownership (R5).
- **P3 · Form fields beyond the six common ones.** Only heat, speed, chunk size and chunk delay have their own boxes; everything else is edited as JSON. Add boxes when a setting proves to be tuned often.

## Release cleanup

- **Before the next release · Decide on the third-party skills.** All skills now live in `.claude/skills/`. The project's own `sc03h-printer` stays. `rust-best-practices` and `tauri-v2` are third-party copies kept for now, with their source and hashes recorded in `skills-lock.json`. Decide whether they stay in the repository; if they go, remove the lock file and the review links above too. If skills are installed with a tool again, use real copies, not Windows junctions: on 2026-10-13, deleting a junction-based install (`.grok/skills` linking to `.agents/skills`) emptied the folders it linked to, and the files had to be restored from git.

## Smaller items

- **Obsolete as written:** `ErrorCode::AmbiguousProfile` already exists. Runtime use is currently unnecessary because overlapping built-in prefixes are rejected; see the reconciliation table.
- Raster numbers duplicated inside the profile JSON (Simplify).
- Centralized logging (Simplify).
- SQLite `named_params`, an `ensure_ready` helper and `ErrorCode::as_str` (Simplify).
- **Resolved:** PDFium availability is probed through startup `get_app_info` and cached in `AppService`.
- Avoid re-encoding images at admission (Simplify).
