# Rust follow-up implementation plan

**Status: in progress; see [progress](#progress).** Prepared on 2026-10-29 against `202612a` on `chore/update-dependencies` and updated the same day for the front-end rework. The baseline is `5969001`: the 14 review-fix commits from `fa293c0` through `202612a`, then `b4e76fd` (the Slint app ships as a preview installer beside the Tauri app) and `5969001` (the front ends moved to `apps/tauri` and `apps/slint`; the Slint package is now `nuprint-slint`). The commits are local; this plan does not require pushing, opening a PR or starting CI.

The eight review fixes and the smaller completed follow-ups are the starting point. Do not redo the iPrint row-buffer/CRC optimization, batched queue query, enqueue page-copy removal, PDFium test enforcement or retention wording. The remaining work is to validate the changes in native applications, measure runtime contention, improve bounded execution and failure handling, and make the Rust APIs easier to maintain.

This plan is grounded in the current source and the `rust-best-practices` guidance on ownership, profiling, linting, errors and documentation, together with the repository's [SC03h skill](../.claude/skills/sc03h-printer/SKILL.md). After the rework, formatting, workspace Clippy, workspace tests, the frontend typecheck and its 41 tests, regenerated bindings and whitespace checks passed on Windows. On 2026-10-29 the owner ran the non-printing checks for the review fixes in the Slint app and the Tauri dev build on Windows and reported them working; no paper was used. Physical BLE cancellation at a known stage, the Slint installers on macOS and Linux, and any packaged build with a printer remain unverified.

**Scope and fixed constraints**

- Keep protocol encoding pure, BLE independent of vendor protocols, and core independent of Tauri, Slint and OS APIs.
- Preserve durable claim before Bluetooth I/O, checked cancellation, immutable profile revisions, one owner per data folder and conservative uncertainty. Never automatically replay an interrupted attempt.
- Preserve the exact SC03h model matcher, golden bytes, heat, speed, chunk cap, pacing, AE pause/resume, fresh status and final-copy feed rules. Read [protocol.md](protocol.md) before implementation touching these paths.
- Keep typed text and PDF passwords out of persistent jobs and diagnostic measurements. Preserve the documented 30-day metadata/first-page-preview policy.
- Measure performance changes in release builds. Module extraction, documentation and reliable error reporting do not need a speedup to justify them; they need clear ownership and behavior-preserving checks.
- Signing, packaging expansion, new models, profile import, system-printer integration and unrelated React backlog items are separate work. The Slint app (`apps/slint`) ships only as a preview installer; closing its feature gaps with the Tauri app is separate work, but every core change here must keep both front ends working.

**Delivery order and dependencies**

| Step | Deliverable | Depends on | Exit condition |
| --- | --- | --- | --- |
| 0 | Baseline validation record and corrected backlog references | Existing 14 commits | Native smoke-test results and remaining hardware gaps recorded honestly |
| 1 | Reproducible measurements | Baseline source | Release results for rendering, storage, planning and logging |
| 2 | Bounded rendering | Step 1 | Obsolete work cannot build an unbounded render backlog |
| 3 | Blocking-work boundaries | Step 1; coordinate with step 2 | No measured heavy filesystem/SQLite/encoding work on async or UI threads |
| 4 | Persistent log writer | Step 1; shared shutdown contract from step 3 | Bounded log buffering, reliable rotation and explicit flush/error behavior |
| 5 | Dispatcher supervision and recovery policy | Failure tests; complete recovery depends on step 3 lifecycle | Failure is visible; no orphan transfer or automatic replay |
| 6 | Reuse immutable page plans across copies | Step 1; step 3 execution boundary | Encoding/allocation scales with unique page variants, not copy count |
| 7 | Private module extraction | Ownership settled in steps 2–6 | Smaller responsibility-based modules with unchanged public contracts |
| 8 | Workspace lints and API examples | Independent; finalize after step 7 | Every member inherits selected lints; useful executable examples pass |

Steps 2–4 can be separate implementation tasks once measurements exist. Start dispatcher failure detection early; do not wait for all performance work to report a dead dispatcher. Lints and doctests can proceed independently. Keep commits focused and avoid combining mechanical file moves with behavior changes.

**0. Close validation gaps in the completed work**

1. Record the baseline SHA, toolchain, platform, PDFium location/version and exact checks in a short validation entry. Use temporary data folders and synthetic documents. Keep raw logs under ignored `output/`; commit only content-free results and conclusions.
2. Launch the Tauri app (`apps/tauri`) at its default window size. Check text/image/PDF previews, printer selection, Advanced save/reset and the queue/history preview. During a connection check, edit a setting and remove a test printer; confirm the result neither reverts the edit nor resurrects the row. Where hardware is unavailable, record the scenario as pending rather than treating compilation as execution evidence. *Owner-reported working on Windows (dev build), 2026-10-29.*
3. Launch the Slint app (`cargo run -p nuprint-slint`). Save an edited profile, edit it again to create another revision, reset it, and switch printers. Confirm the selected profile revision, preview limits and Print availability update without restart. Do not press Print during this smoke test. Extract and test the profile-selection helper if it can be separated naturally; retain the manual binding check. *Owner-reported working on Windows (dev build), 2026-10-29. The helper is tested (`the_selected_printer_gets_exactly_its_profile_revision` in `apps/slint/src/ctx.rs`).*
4. Check the Slint preview installers. On Windows the NSIS installer contains the app and PDFium, and the unpacked release app reports PDF available. The macOS DMGs and the `.deb` are first built by the release workflow; install each once and confirm launch, the Bluetooth permission prompt (macOS) and PDF availability before calling them working.
5. For a later authorized non-printing hardware session, exercise cancellation during connection establishment and GATT setup, followed by a normal Check connection. Record whether disconnect completed before the next session acquired hardware. Use a deterministic staged mock for exact timing; manual cancellation alone cannot prove which native stage was interrupted. A physical print remains a separate authorized test, starting with one short strip if needed.
6. Reconcile older backlog numbering: the 2026-10-13 R6/R7 entries correspond to the now-fixed connection-check and connection-cancellation issues. Mark the code fixes resolved with the implementing commits, while retaining the hardware-validation gap. Do not confuse those IDs with the later review's R6 WAL backup and R7 limits issues, or close unrelated historical entries.

Acceptance: the record distinguishes automated, native UI and hardware evidence; R8 has an actual Slint smoke test (owner-reported); unavailable platforms/devices remain explicitly pending. Baseline validation should not consume paper.

**1. Establish measurements before changing execution or allocation**

Primary locations: [`service.rs`](../crates/nuprint-core/src/service.rs), [`store.rs`](../crates/nuprint-core/src/store.rs), [`raster.rs`](../crates/nuprint-render/src/raster.rs), [`driver.rs`](../crates/nuprint-protocol/src/driver.rs), and the Slint event/model update paths in [`apps/slint/src/main.rs`](../apps/slint/src/main.rs).

Add a small reproducible release-mode benchmark/stress harness using the existing mock transport and generated, non-private inputs. Add benchmark dependencies only if the existing test/example infrastructure cannot provide repeatable results. Keep timing thresholds out of ordinary correctness tests.

| Workload | Cases | Measurements |
| --- | --- | --- |
| Preview | Short and maximum accepted text; small and near-limit decoded images; one PDF page; bursts of 20 then 100 generations | Latest-preview latency, work started/discarded, peak active renders, peak process memory |
| Admission and planning | 384 × 164 and 384 × 4000 bitmaps; 1, 2 and maximum permitted copies; zero/default feed | CPU time, allocation count/bytes, encoder calls, retained plan bytes, admission latency |
| Store contention | Empty queue, representative queue, configured maximum retained jobs; listing while admitting/cancelling | Mutex wait/hold duration, transaction duration, async heartbeat delay |
| Logging | Ordinary events and synthetic bursts around rotation size; slow/failing writer | Producer latency, buffered/dropped records, flush time, Slint UI update time |
| Shutdown/failure | Idle, render active, database write active, connection active, mock transfer active | Completion time, outstanding work, owner-lock lifetime, final durable job state |

Run warmups and repeated samples on the same machine, power settings, toolchain and fixtures. Separate cold PDF/font initialization from warm runs. Report median and p95, sample count, spread, wall time and memory; use enough samples for tail claims. Compare the fixed baseline with one change at a time. Set a workload-specific target before implementing each optimization, based on the measured problem. A repeatable reduction beyond measurement noise or an enforced resource bound is an acceptable result; an unmeasurable micro-optimization is not.

Instrument phase durations and lock waits without names, paths, source text or passwords. Use static operation names and sizes. Store benchmark output in ignored `output/`, and summarize the results in a versioned performance note. Do not infer improved BLE throughput from a faster CPU encoder: pacing and the physical device may dominate.

**2. Bound rendering and discard obsolete requests before expensive work**

Current behavior: `render_preview` calls `spawn_blocking` for rendering, but each incoming request can start independent work. The generation check after rendering avoids publishing some stale results; it does not bound wasted work. PNG encoding and bitmap statistics still run after the blocking task returns. PDFium already serializes access internally.

Implementation:

1. Add a core-owned render coordinator. Start with one active render and one replaceable pending preview per `AppService`, then measure whether two active jobs improve the supported workloads. Keep this an internal policy initially.
2. Preserve the current generation contract. A newer request replaces older pending work; callers whose pending request is replaced receive `StalePreview`. Audit equal-generation handling and frontend assumptions explicitly. Do not introduce a multi-editor identity scheme without a consumer that needs it.
3. Acquire execution capacity before dispatching blocking work, then recheck generation and shutdown. Do not enqueue an unlimited number of blocking closures that wait for a permit. Bound pending request ownership as well as active CPU jobs.
4. Keep the permit owned by the running closure until native rendering actually finishes. Dropping an async waiter must not release capacity while its blocking work continues. Already-running native calls may finish; prevent their stale results from entering snapshots or winning over the latest preview.
5. Render, encode PNG, compute hash/black ratio and produce an owned result in the same bounded worker operation. Move the resulting bitmap into the snapshot instead of cloning it solely for preview construction. Keep generation check and publication ordered so a newer accepted request cannot be overtaken during PNG generation/publication.
6. Include PDF loading/page-count work in a compatible bounded path; do not add another unbounded queue behind PDFium's mutex. Preserve in-memory-only password handling.

Tests: use a controlled renderer with barriers to hold one request, replace pending generations repeatedly, drop callers and initiate shutdown. Assert the concurrency/pending caps, newest-generation publication, exact snapshot identity and permit lifetime. Test render errors and panic/join errors without leaving permits stuck. Compare real text/image/PDF bitmap hashes and preview pixels with the baseline.

Acceptance: a burst retains only bounded scheduled work; the latest request eventually finishes; cancellation/shutdown does not falsely imply an in-progress native call stopped. Measure peak memory and latest-preview latency under the burst workload.

Optional measured subtask: remove the full RGBA copy in `composite_on_white` by borrowing an existing RGBA representation or consuming an owned image at the right boundary. Retain the exact alpha compositing and luma rounding for every supported image representation, EXIF orientation and PDF output. Add representative transparency/color-format equivalence cases. Keep this separate if measurements show little benefit.

**3. Move blocking work behind explicit ownership boundaries**

Current behavior: source import reads files synchronously; preview/export paths perform synchronous encoding or writes; `enqueue` validates/encodes/persists synchronously; all store calls use `Mutex<Store>`. Tauri commands being `async` does not make these operations nonblocking. Slint also calls some service operations on its UI thread.

Implementation has two increments:

1. Move independent file reads/writes and expensive CPU work to bounded worker operations. Source import must bound bytes actually read, not rely solely on metadata checked before a file changes. Capture owned paths/data and return results; never hold core locks while awaiting workers. Cover Tauri's asynchronous `nupreview` responder and Slint callbacks as well as command handlers. Snapshot PNGs already in memory should remain cheap.
2. Use measured store contention to choose the storage boundary. Prefer one dedicated worker owning `Store` and its SQLite connection if admission, retention and query operations materially block the runtime. Use a bounded request channel and typed results, with complete domain operations such as admission, claim, cancellation and profile revision save. If a smaller bounded blocking adapter suffices, document that decision and its concurrency cap rather than introducing a worker solely for style.

For either design:

- Keep each transaction and its required filesystem rollback in one owned operation. Preserve R2 publish-after-commit, R5 orphan cleanup, SQLite snapshot backup and `state.lock` ownership.
- Serialize profile revision allocation/save/publication correctly across the new async boundary. A completed database save must not become invisible merely because its caller stopped awaiting it.
- Return a definitive committed/not-committed result before claiming success or opening Bluetooth. A timed-out caller does not prove a queued database mutation was cancelled. Use idempotency keys and state lookup for admission retries; never dispatch a claim whose outcome is unknown.
- Define cancellation before acceptance versus after an operation starts. Once a durability-critical operation is accepted, finish it or record a definite failure. Do not drop it halfway through on caller cancellation.
- Preserve ordering between durable state and emitted events. Move only small immutable summaries across worker boundaries, and avoid synchronous callbacks that re-enter the store while its operation is active.
- Update Tauri, CLI and Slint call sites together if service methods become async. Keep synchronous helpers private where useful; never add nested runtime `block_on` to hide the change.
- At shutdown stop admission, resolve/cancel pending requests under the chosen semantics, finish critical writes and await workers. Keep the data-folder owner lock until work that can mutate its files has ended. Use one documented deadline budget rather than independently spending the whole timeout on every stage.

Tests: deterministically stall a file/store operation and show an independent Tokio heartbeat and cancel request remain responsive. Repeat admission failure cleanup, profile-save failure, concurrent claim/cancel, idempotent submission, WAL backup and restart/retention tests through the new boundary. Test caller cancellation after a committed admission, queued worker overload, worker failure and shutdown during a write.

Acceptance: expensive operations no longer occupy async/UI threads; accepted writes have unambiguous outcomes; no new replay path or ownership gap exists. Compare latency under contention and maintain the current durable-state tests.

**4. Use a persistent, bounded log writer**

Current behavior: `AppService::log` takes file metadata, may rename the file, opens it and writes for every line. Slint rebuilds the displayed log model on updates. Improve file handling first; measure UI batching separately.

Implementation:

1. Introduce a core-owned writer with a persistent file handle, tracked size and bounded input buffer. Keep the in-memory diagnostic ring and UI event content available without waiting on disk. Keep private markers in the diagnostic representation; remove/redact them only at the existing presentation/export boundaries.
2. Preserve sequence ordering across producers. Specify overload behavior: bounded buffering, an explicit aggregate dropped-record counter, and preferential retention of warnings/errors. Do not claim unlimited lossless logging while also promising nonblocking producers. Log loss must never change durable job state or suppress a user-visible terminal error.
3. Close/flush the current handle before rotation and define replacement of the previous rotated file, including Windows behavior. Handle read-only/full disks without recursive logging or a hot retry loop. Surface writer health through a bounded in-memory warning/diagnostic field if necessary.
4. Define flush points for normal shutdown and an explicit diagnostic export; document the possible loss of buffered file lines on abrupt process termination. Use the lifecycle budget from step 3. This log is not the queue's durability mechanism.
5. If measured Slint log updates are material, append/remove rows on its event thread and batch UI notifications rather than rebuilding the entire model for each line. Preserve ordering, retention cap and scrolling behavior.

Tests: concurrent producers, buffer saturation, rotation over an existing backup, write/flush failure, shutdown with pending entries and private-marker regression cases across the whole diagnostics report. Use a controllable slow writer for overload tests. Run native Slint checks if its model changes.

Acceptance: no open/metadata syscall per log record; bounded producer cost and memory under a slow sink; documented loss policy; rotation works on Windows and is checked on the other supported platforms before claiming support.

**5. Supervise the dispatcher; make recovery conditional on proven cleanup**

Current behavior: startup spawns `dispatch_loop`, and shutdown takes its `JoinHandle` but ignores the result. A panic can leave a claimed job, stale `active` state or poisoned shared locks without a clear service-health signal. Merely starting another loop is unsafe: dropping a panicked future is not proof that native Bluetooth work disconnected.

Increment A — detect and fail closed:

1. Add a supervisor that observes dispatcher termination during normal operation. Treat return during requested shutdown as normal; report unexpected return/panic as a fault. The health-reporting path must not depend solely on a potentially poisoned log/store mutex.
2. Stop accepting dispatch/resume while faulted, expose an actionable queue/service status and preserve pending jobs. Record which attempt was durably claimed. If its transfer phase is unknown, mark it uncertain when persistence is usable; otherwise leave the durable active state for restart recovery. Never requeue it automatically.
3. Audit mutexes by invariant: reconstructible caches may be discarded; store transactions, active-job state and profile publication need validated recovery or a structured failure. Do not replace every poisoned lock with `into_inner()` or a non-poisoning mutex.
4. Make shutdown observe supervisor and worker results. Preserve hardware and data-folder ownership until cleanup actually ends; reporting a deadline exceeded must not free resources still in use.

Increment B — limited recovery, only after lifecycle tests pass:

1. Establish a session/connection owner whose lifetime survives an individual job task's unwind and can perform bounded disconnect cleanup. Include native connection attempts that finish late. If cleanup cannot be established, keep the service faulted and require an app restart; do not release hardware for a replacement dispatcher.
2. Reconcile only the failed dispatcher's owned attempt and active state after cleanup. Do not run blanket startup recovery against a live service.
3. Permit a replacement dispatcher only when storage/profile invariants are valid and old work has ended. Keep the queue held pending explicit resume; the interrupted attempt remains uncertain and requires explicit retry. Start with at most one replacement per service lifetime to avoid panic loops.
4. If evidence does not justify in-process restart complexity, ship increment A with restart-the-app recovery and record increment B as deferred. Failure visibility and replay safety are the required outcome; automatic availability is secondary.

Tests: inject failure before claim, after claim/before connect, during connection setup, during an in-flight first print write, after bytes may have arrived, and while persisting completion. Assert no second hardware owner, no automatic replay, correct conservative state, bounded failure reporting and no repeated restart loop. Repeat with store unavailable, poisoned critical state and shutdown racing failure. Use mock write/connect counters and cleanup barriers; a passing no-hardware test is not physical BLE validation.

Acceptance: dispatcher death cannot silently strand the queue; recovery never assumes a failed write was unsent; unresolved cleanup prevents further hardware use. Document the chosen recovery policy and any API/binding changes.

**6. Reuse immutable page plans across copies**

Current behavior: `plan_pages` materializes an `ExchangePlan` for every page/copy, admission plans again for per-page metadata, and execution builds plans again from durable rasters. The row encoder is already optimized. The remaining opportunity is repeated planning and ownership of identical bytes.

Implementation:

1. Measure encoder call count and memory for maximum copies. Introduce an attempt-local prepared plan containing unique page variants and a lightweight ordered list of copy/page references. Prefer sharing whole immutable `ExchangePlan`s in core before changing the protocol API to share every byte buffer.
2. Prepare zero-feed variants and, when needed, one final-page variant with trailing feed. If final feed is zero, reuse the same variant. Preserve every plan step, including per-copy status checks, settling and completion policy; reuse does not mean concatenate only the print bytes.
3. Resolve the exact immutable profile revision and protocol version, validate every stored bitmap and construct all variants before connecting. Do not lazily discover invalid later pages after an earlier page has printed.
4. Derive total bytes and per-page metadata from the prepared plans using checked arithmetic. Avoid re-encoding inside `admit` while holding the store lock. Preserve confirmed bytes versus at-risk bytes and per-copy progress accounting.
5. Keep admission and execution validation separate: execution still verifies durable rasters and reconstructs its attempt-local plans after restart. Do not persist encoded wire buffers or add a global cache in the first increment. Cross-attempt caching would need explicit invalidation keys, size limits and retention rules and must justify its complexity separately.

Tests: byte-for-byte comparisons against existing Python vectors and baseline multi-copy mock traces; zero/default feed; all supported densities; cancellation/failure on a later copy; corrupted later page rejected before any connect; exact byte totals and final-copy feed placement. Include maximum permitted copies and a protocol-independent test driver whose non-data plan steps must not disappear.

Acceptance: encoder calls and retained byte buffers scale with unique page variants rather than copies. Golden bytes, status/flow sequencing, uncertainty and progress semantics remain unchanged. Reject the optimization if measurement shows no useful gain relative to added complexity.

**7. Split modules along the ownership established above**

Primary targets are `nuprint-core/src/service.rs` and `apps/slint/src/main.rs`. Avoid an upfront rewrite while render, storage and shutdown responsibilities are still changing.

1. Keep `AppService` as the public facade. Extract private service modules for previews/sources, printer/profile operations, admission/queue queries, dispatch/lifecycle and diagnostics/logging. Move supporting types with their owner; use narrow visibility rather than making internals public to solve module access.
2. Keep store transaction logic in the store layer and connection/transfer logic in session/transport. A module split must not relocate SC03h packet knowledge into core or Tauri.
3. Split Slint bootstrap, UI state/model conversion, event handling and feature callbacks only after its async call sites settle. Keep UI mutation on the event thread.
4. Move one responsibility per commit with no schema, public API, generated-binding or behavior changes in that commit. Do not merge render/API types or slim transport traits merely because older backlog entries suggest it; those are separate API decisions.

Acceptance: each module has a clear responsibility and smaller private surface; existing integration tests exercise the same public behavior; no new circular ownership or lock order appears. Run the Tauri/Slint smoke paths after callback moves. Line count reduction alone is not an acceptance criterion.

**8. Adopt selected workspace lints and executable API documentation**

Lints:

1. Re-measure candidate warnings against the pinned toolchain. Historical review counts are not the current baseline. Start with `redundant_clone`, `needless_collect`, `large_enum_variant` and `await_holding_lock`, plus existing default Clippy coverage and rustdoc broken-link checks.
2. Add root `[workspace.lints]` settings and `[lints] workspace = true` to all seven members, including the Tauri host (`nuprint-desktop`) and the Slint app (`nuprint-slint`). Use lower priority for groups than individual overrides. Confirm normal development and CI's `-D warnings` agree.
3. Resolve findings deliberately before enabling their gate. Use narrow, explained exceptions where justified. In particular, preserve collection of all PDF test thread handles before joining: turning it into a lazy spawn-then-join chain would remove the intended concurrency.
4. Add `missing_errors_doc` and `missing_panics_doc` in stages after documenting real contracts. Do not immediately enable all pedantic/nursery lints or blanket `missing_docs` across generated/macro-heavy APIs. Scope unsafe restrictions to crates without native boundaries; do not forbid the documented macOS FFI globally.

Documentation:

1. Add running doctests for canonical `MonoBitmap` bit order and bounds, a valid iPrint plan without I/O, text rendering and PNG output, and transport use with the scriptable mock where concise.
2. Document fallible APIs' errors and actual panic preconditions. Explain the runtime requirement and shutdown contract of `AppService::start`, profile revision identity, cancellation ownership and confirmed versus at-risk print bytes.
3. Keep examples deterministic and hardware-free. PDF examples that require a native library may be compile-checked and linked to the required-PDFium integration suite; do not count `no_run` as executed behavior or hide essential examples with `ignore`.
4. Prefer tests that establish one coherent contract over a mechanical one-assertion rule. Keep multi-field Python golden-vector comparisons intact.

Acceptance: all seven members inherit the lint policy, intentional exceptions state why, useful doctests run, rustdoc links resolve, and examples cannot accidentally print. Run `cargo test --workspace --doc --locked` explicitly and record the new executable-example count.

**Verification and completion checklist for each implementation slice**

- Run focused deterministic tests for the changed ownership/failure boundary, then the repository checks: formatting, workspace Clippy with all targets, workspace tests, and `git diff --check`. Include an all-features Clippy pass where the workspace feature combination is supported.
- Set `NUPRINT_REQUIRE_PDFIUM=1` for validation claiming PDF coverage, with PDFium installed. Restore the prior environment afterward. Release measurements must configure its library search path explicitly because debug-only vendor discovery does not apply.
- Run frontend typecheck/tests in `apps/tauri` when command signatures, events or generated bindings change. Inspect regenerated bindings and include them with the type change. Run native Tauri/Slint smoke checks for their changed flows; compilation is not a substitute. If Slint startup or resource lookup changes, rebuild its installer as in [building.md](building.md#slint-app) and check PDF availability.
- Run the CLI test with `--preview` for offline raster verification. Use existing Python fixtures unchanged unless reference behavior itself changes.
- For concurrency changes, use barriers, controlled faults and counter assertions rather than sleeps as primary correctness evidence. Keep latency benchmarks separate from timing-sensitive CI assertions.
- Update current-behavior docs only when the behavior lands. Record before/after measurements and explicitly unverified platforms/hardware. Document a rejected optimization as measured and deferred instead of implementing it to complete a checklist.
- Make each future commit independently reviewable under the repository's Conventional Commit rules. Do not amend the completed 14 commits merely to group follow-ups. New branches, when needed, use `<type>/<kebab-summary>`; publishing or starting remote workflows requires the corresponding instruction.

Completion means the selected slices meet their acceptance criteria and remaining items have an explicit measured or safety-based deferral. It does not mean every optimization must be implemented, every platform is hardware verified, or an in-process dispatcher restart must be shipped regardless of cleanup evidence.

## Progress

Updated as slices land. Steps 1, 2, 4, 5 (increment A) and the doctest half of step 8 started in parallel on 2026-10-29, each on its own branch from the baseline, and were merged one slice at a time. Steps 2 and 4 started before the step 1 baseline existed, so each was merged only after it clearly beat that baseline ([results](performance.md#measured-after-steps-2-and-4)).

| Step | Status | Commits |
| --- | --- | --- |
| 0 | Done: owner smoke tests, backlog reconciliation and validation entries recorded | `6ee98e3` |
| 1 | Done: release harness and baseline | `c85a4b3`, `15efde6` |
| 2 | Done: one active and one pending render; peak memory for a large-image burst fell about tenfold. The newest image can wait for the render in flight. The optional compositing subtask is done too: eight-bit pictures no longer get an RGBA copy, which roughly halves a 36 MP render's peak heap ([results](performance.md#compositing-pictures-onto-white-41a3d8c)) | `74b64f7`, `41a3d8c` |
| 3 | Done. File reads and writes, Tauri job-page previews and Slint reloads run on blocking threads; store calls stay behind the mutex, a measured decision ([why](performance.md#decisions-from-the-measurements)) | `99fc80f`, `d276d56`, `1a71a08` |
| 4 | Done: persistent writer; handing a line to the writer costs the caller about 0.4 µs, against about 115 µs for the old synchronous write. That is producer latency: a tight loop can outrun the writer, and lost lines are counted and reported ([results](performance.md#step-4-persistent-log-writer-cb0a401)). The Slint log now appends rows instead of rebuilding them, measured first ([results](performance.md#slint-activity-log-fa65a47)) | `cb0a401`, `da36b85`, `fa65a47` |
| 5 | Increment A done and tested, including a stopped dispatcher, invalid storage and a timed-out shutdown keeping the data folder. Increment B deferred: no session owner yet outlives a failed job task to prove the connection closed, so recovery stays "restart the app" | `0d9e58c`, `a69f437`, `044ceb4` |
| 6 | Measured and deferred: an extra copy costs about 5 ms of CPU against roughly 40 s of paced transfer ([why](performance.md#decisions-from-the-measurements)) | — |
| 7 | Done as pure moves: `service.rs` went from about 2,280 lines to 470, with previews, printers, queue, dispatch, logging, diagnostics and lifecycle in private `service/` modules; the Slint `main.rs` went from about 1,830 lines to 24, split into bootstrap, state, view, context, sync, preview, events and per-feature callbacks. The owner ran the non-printing smoke checks in both apps on Windows after the moves and reported them working | `9efe11a`…`350e79e`, `1ea8114`…`a8e1f83` |
| 8 | Done in stages. Doctests: 18 run, 1 compile-only. All seven members inherit `[workspace.lints]` (`await_holding_lock`, `large_enum_variant`, `needless_collect`, `redundant_clone`, rustdoc broken links), with one stated exception in the PDF concurrency test. `missing_errors_doc` and `missing_panics_doc` are enforced in protocol, render, transport, core and the Tauri host; the CLI and Slint crates are binaries the lint never checks | `acb4eb3`…`e178d5d`, `de2fd16`, `a7ed136`, `ae3a130`, `2a69630`, `1819f43` |

Poisoned-lock recovery now also covers the log ring (`da36b85`), the PDF engine slot (`bb6e479`), and the log writer and BLE device cache (`053a9f1`); no production lock in core or transport unwraps.

A review of the implemented steps on 2026-10-29 found four gaps, now fixed offline: a log write stuck on the disk could hang exit after shutdown reported success (`b5d9853`); lines the log file lost were missing from the in-memory log and diagnostics (`8533796`); connection and adapter events turned off the Windows silent-scan restart (`6e55590`; not rerun on hardware); and the Slint activity log dropped a line whose event arrived after a newer one (`b32f81c`, with the Tauri log now ordering such lines too, `b47fbe5`).

Still open after these slices, by design or for lack of hardware: step 0's macOS and Linux preview installers, which the release workflow builds first and which need a Mac and a Linux desktop to check; step 5 increment B; and step 6. The non-printing cancellation session ran on Windows with `crates/nuprint-cli/examples/cancel_connect.rs`: cancels while connecting and during service discovery all released Bluetooth, and it found and fixed a silent Windows scan after a cancelled search ([results](validation.md)).
