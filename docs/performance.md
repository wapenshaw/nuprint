# Baseline performance measurements

Step 1 of the [Rust follow-up plan](rust-follow-up-plan.md): release-mode, hardware-free numbers that later steps (bounded rendering, blocking-work boundaries, a persistent log writer, reusable page plans) can be judged against. Everything here is CPU, memory and scheduling cost against the mock transport. **None of it says anything about Bluetooth throughput or print time**; the printer's pacing and the radio dominate those.

| | |
| --- | --- |
| Date | 2026-10-29 |
| Measured code | baseline `2799905` plus the harness commit `c75b47d` (`test(core): add a release benchmark harness`) |
| Toolchain | rustc 1.99.1 (2026-10-13), cargo 1.99.1, release profile |
| Machine | Intel Core i7-13700K (16 cores, 24 logical), 63.7 GB RAM, Windows 11 Pro 10.0.26300 |
| Power plan | Windows "Balanced" |
| PDFium | 151.0.7881.0 from `vendor/pdfium/win-x64/bin`, selected with `NUPRINT_PDFIUM_DIR` |
| Transport | `MockTransport` only: no Bluetooth, no paper |
| Platforms not measured | macOS, Linux |

Other work was running on the same machine (other agents building and testing in a shared target directory), so there is some noise. Two complete passes were made; the second pass's medians are shown beside the first's so run-to-run spread is visible. Raw output of both passes is in the ignored `output/` folder (`bench-<section>.md` and `bench-run2-<section>.md`).

## Method

The harness is [`crates/nuprint-core/examples/bench.rs`](../crates/nuprint-core/examples/bench.rs). It uses only the public `AppService` API, generated inputs (noise bitmaps encoded as PNG, a generated one-page PDF, repeated text) and a temporary data folder, and keeps no timing threshold anywhere near the test suite.

```powershell
$env:NUPRINT_PDFIUM_DIR = "vendor\pdfium\win-x64\bin"   # PDF cases only; release builds do not search vendor/
cargo run -p nuprint-core --example bench --release -- all
cargo run -p nuprint-core --example bench --release -- preview burst      # chosen sections
cargo run -p nuprint-core --example bench --release -- --quick all        # smoke test, few samples
```

Sections are `preview`, `burst`, `admission`, `exec`, `store`, `logging` and `shutdown`. The tables below come from one process per section, so the peak working set printed for a section is attributable to it.

- **Statistics.** Median, p95 (nearest rank), standard deviation (sd) and sample count `n` after warm-up calls. With `n` below about 20, p95 is effectively the maximum and is not a tail estimate.
- **Cold versus warm.** The first text render and the first PDF render in a fresh process are reported once, separately, and are not mixed into the warm rows.
- **Allocations.** A counting global allocator is enabled only around the `admission` and `exec` measurements. Counts include every thread, but the dispatcher is idle during admission (queue held).
- **Execution.** Runs on a current-thread Tokio runtime with paused virtual time, so pacing and ready-poll sleeps cost no real time. The wall time is the CPU work of planning, encoding, the mock exchange, status handling and persistence.
- **Memory.** Process working set and peak working set from `K32GetProcessMemoryInfo` (Windows) or `/proc/self/status` (Linux).
- **Async scheduling delay.** An OS thread spin-waits a precise 1 ms tick and sends it to a Tokio task; the delay is how long after the tick the task ran. (`tokio::time::sleep(1ms)` cannot be used on Windows: timers resolve to about 15.6 ms.)
- **Not instrumented.** Mutex wait and hold times inside the store are not measured because that needs a change to `service.rs`; call latency and scheduling delay stand in. Encoder call counts are read from the code, not counted. The Slint model-update path and a real `tracing` subscriber were not measured.

## Results

### Preview latency, one generation at a time

`render_preview` end to end: spawn to a blocking thread, render, PNG encode, hash, snapshot insert. Cold first call: text 1.43 ms (run 2: 1.35 ms), PDF page 7.33 ms (8.10 ms), so neither font nor PDFium initialisation is large next to the 5.7 ms warm PDF render.

| Case | n | median ms | p95 ms | sd ms | run 2 median ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| text, 5 chars | 40 | 0.030 | 0.047 | 0.007 | 0.025 |
| text, 2,000 chars | 20 | 1.12 | 1.21 | 0.069 | 1.16 |
| text, 3,000 chars (about 3,600 of the 4,000 allowed rows) | 10 | 1.73 | 1.88 | 0.059 | 1.73 |
| text, 20,000 chars (the accepted length; rejected as too tall, so this is the time to reject) | 5 | 3.40 | 3.52 | 0.074 | 3.58 |
| test pattern | 40 | 0.141 | 0.178 | 0.026 | 0.136 |
| image 384 x 164 | 30 | 0.986 | 1.06 | 0.025 | 1.03 |
| image 2000 x 2000 (4 MP) | 15 | 62.9 | 65.2 | 0.725 | 66.4 |
| image 6000 x 6000 (36 MP; the decoded-pixel limit is 40 MP) | 5 | 523 | 528 | 2.20 | 576 |
| PDF page, 3 lines | 20 | 5.72 | 5.98 | 0.153 | 6.04 |

Peak working set for the whole section was 236 MiB, set by the 36 MP decode. The maximum text length (20,000 characters) cannot be printed at the default size: the 4,000-row limit stops it, and only after the full layout has run.

### Preview bursts

Ascending generations fired together (or at 5 ms spacing), so only the newest is current. "Early stale" results were rejected within 2 ms (before any render). "Late stale" results rendered fully and were discarded afterwards. Counts are medians over the repetitions; peak working set is cumulative for the process, so read the increases.

| Burst | reps | latest ms | wall ms | ok | early stale | late stale | peak WS MiB | run 2 latest / wall ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| text 2,000 chars, 20 at once | 5 | 7.29 | 12.2 | 1 | 7 | 12 | 31 | 12.1 / 12.3 |
| text 2,000 chars, 100 at once | 5 | 40.6 | 45.9 | 1 | 40 | 59 | 31 | 49.8 / 53.3 |
| text 2,000 chars, 100 at 5 ms spacing | 3 | 1.32 | 1,544 | 100 | 0 | 0 | 31 | 1.76 / 1,546 |
| image 4 MP, 20 at once | 5 | 92.7 | 102 | 1 | 9 | 10 | 265 | 108 / 115 |
| image 4 MP, 100 at once | 3 | 231 | 298 | 1 | 57 | 42 | 1,063 | 209 / 277 |
| image 36 MP, 20 at once | 2 | 919 | 983 | 1 | 6 | 13 | 2,657 | 885 / 899 |

Peak working set for the burst process was 2,745 MiB (run 2: about 2,270 MiB at the 36 MP row). A single 36 MP render peaked at 236 MiB.

### Admission (`enqueue`) with the queue held

The caller blocks for the whole call: validation, one plan per copy and page, then the durable insert. n = 30 per row.

| Bitmap | Copies | Feed mm | median ms | p95 ms | max ms | allocs | MiB allocated | run 2 median ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 384 x 164 | 1 | 0 | 2.13 | 4.29 | 4.35 | 76 | 0.022 | 2.14 |
| 384 x 164 | 1 | 12 | 1.98 | 2.35 | 2.57 | 76 | 0.022 | 1.89 |
| 384 x 164 | 2 | 0 | 1.84 | 2.18 | 2.47 | 78 | 0.031 | 1.91 |
| 384 x 164 | 2 | 12 | 1.90 | 2.22 | 2.43 | 78 | 0.031 | 1.89 |
| 384 x 164 | 20 | 0 | 2.25 | 3.18 | 3.43 | 114 | 0.194 | 1.93 |
| 384 x 164 | 20 | 12 | 1.97 | 2.30 | 2.30 | 114 | 0.194 | 1.94 |
| 384 x 4000 | 1 | 0 | 2.66 | 4.84 | 5.46 | 76 | 0.432 | 2.57 |
| 384 x 4000 | 1 | 12 | 2.85 | 3.48 | 3.80 | 76 | 0.432 | 2.57 |
| 384 x 4000 | 2 | 0 | 2.72 | 3.02 | 3.32 | 78 | 0.646 | 3.03 |
| 384 x 4000 | 2 | 12 | 2.95 | 3.40 | 3.42 | 78 | 0.646 | 2.64 |
| 384 x 4000 | 20 | 0 | 6.25 | 7.23 | 8.02 | 114 | 4.50 | 5.86 |
| 384 x 4000 | 20 | 12 | 6.09 | 7.04 | 7.82 | 114 | 4.50 | 6.05 |

About 2 ms is fixed cost (store transaction and bookkeeping). Allocation bytes grow by about 0.21 MiB per copy of a 4,000-row page, which is one full 48-byte-per-row plan (192 KB plus framing) per copy. The feed setting makes no measurable difference here. Retained plan bytes after admission were not measured: the plans are dropped when `enqueue` returns.

### Execution through the mock (CPU cost, pacing removed)

Enqueue to terminal event, a new service per row, n = 5 after one warm-up job. Every row ended `Completed`.

| Bitmap | Copies | Feed mm | median ms | p95 ms | allocs | MiB allocated | run 2 median ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 384 x 164 | 1 | 0 | 8.71 | 8.91 | 786 | 0.192 | 7.78 |
| 384 x 164 | 1 | 12 | 8.28 | 8.85 | 789 | 0.192 | 8.66 |
| 384 x 164 | 2 | 0 | 10.0 | 10.4 | 1,300 | 0.251 | 11.4 |
| 384 x 164 | 2 | 12 | 11.0 | 11.2 | 1,303 | 0.251 | 10.7 |
| 384 x 164 | 20 | 0 | 53.3 | 54.6 | 10,557 | 1.19 | 55.0 |
| 384 x 164 | 20 | 12 | 54.4 | 58.4 | 10,560 | 1.19 | 52.4 |
| 384 x 4000 | 1 | 0 | 16.0 | 16.6 | 10,596 | 3.73 | 13.8 |
| 384 x 4000 | 1 | 12 | 16.2 | 17.0 | 10,600 | 3.73 | 14.2 |
| 384 x 4000 | 2 | 0 | 24.4 | 25.2 | 20,911 | 4.85 | 18.7 |
| 384 x 4000 | 2 | 12 | 28.5 | 31.9 | 20,915 | 4.85 | 19.8 |
| 384 x 4000 | 20 | 0 | 124 | 147 | 206,586 | 28.9 | 102 |
| 384 x 4000 | 20 | 12 | 129 | 139 | 206,590 | 28.9 | 97.7 |

Allocation count and bytes are deterministic and scale linearly with copies (about 10,300 allocations and 1.3 MiB per extra 4,000-row copy). Wall time moved by up to 20% between passes, so differences smaller than that are noise. The whole process peaked at 54 MiB. The mock keeps every written byte, which adds to the allocation figures.

### Store contention

`list_jobs` returns at most 200 jobs plus their pages, called sequentially from an async task. Queued jobs are held and keep their print data (so at most 100 exist); history is 300 completed jobs. n = 200 per row.

| State | median µs | p95 µs | sd µs | run 2 median µs |
| --- | ---: | ---: | ---: | ---: |
| empty | 41.7 | 43.3 | 1.41 | 41.5 |
| 20 queued | 166 | 275 | 51.6 | 144 |
| 100 queued (the retained-job cap) | 701 | 1,166 | 203 | 627 |
| 300 completed (list capped at 200) | 1,618 | 2,649 | 486 | 1,393 |
| 100 queued + 300 completed | 2,000 | 2,957 | 564 | 1,452 |

Concurrent phase: two Tokio worker threads, state 50 queued + 300 completed, 4 s per scenario. Each looper yields between calls, so the loops saturate the store mutex; this is a stress shape, not a UI workload. A churn operation is enqueue + cancel + remove. "Direct" calls the sync API from the async task; "blocking" wraps each call in `spawn_blocking`.

| Scenario | heartbeat median ms | p95 ms | p99 ms | max ms | list calls | list median µs | list p95 µs | churn ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| heartbeat only | 0.010 | 0.032 | 0.051 | 0.127 | 0 | - | - | 0 |
| 4 list loops, direct | 1.14 | 3.41 | 4.58 | 6.21 | 2,071 | 3,873 | 5,436 | 0 |
| 4 list loops + 2 churn loops, direct | 1.60 | 6.29 | 8.39 | 11.7 | 1,185 | 2,863 | 5,112 | 431 |
| 4 list loops + 2 churn loops, blocking | 0.015 | 0.028 | 0.047 | 0.412 | 1,639 | 9,052 | 18,674 | 192 |

Run 2 agrees: direct with churn p95 6.07 ms and max 12.6 ms, blocking p99 0.048 ms and max 0.114 ms, blocking list median 7.7 ms and 211 churn operations.

### Logging (`AppService::log`)

Per call, producer side: ring buffer, event sink (a no-op here), `tracing` (no subscriber installed, so that cost is a floor), then with a data folder: stat, open, append and close of `logs/nuprint.log`. Rotation is at 2,000,000 bytes. Lines are about 95 bytes. n = number of lines.

| Case | lines | median µs | p95 µs | p99 µs | p99.9 µs | max µs | total ms | run 2 median / p95 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| no data folder (memory only) | 20,000 | 0.2 | 0.2 | 0.3 | 0.4 | 6.2 | 5 | 0.2 / 0.2 |
| file, ordinary volume | 2,000 | 115 | 212 | 289 | 396 | 480 | 257 | 115 / 208 |
| file, burst crossing rotation | 60,000 | 119 | 377 | 521 | 1,064 | 10,230 | 9,457 | 114 / 232 |

After the burst, `nuprint.log` held 483,184 B and `nuprint.log.1` 2,000,014 B (run 2 identical), so rotation fired as designed; it keeps one previous generation, and each rotation overwrites it. The slowest calls were not at the rotation boundaries, so rotation itself is not a visible latency cliff; the tail is ordinary file-system jitter. While a 60,000-line burst was logged from an async task that yielded every 50 lines, the heartbeat on a second worker stayed at its idle level (median 0.010 ms, max 0.124 ms), which shows the cost lands on the producing thread only.

### Shutdown

Real time (not paused), wait limit 10 s, new service per sample. "Restart" is the time after `shutdown` returned until a new service opened the same data folder, a proxy for how long the owner lock is held.

| Case | n | median ms | p95 ms | sd ms | run 2 median ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| idle: shutdown | 15 | 0.030 | 0.088 | 0.022 | 0.028 |
| idle: restart after shutdown | 15 | 1.71 | 2.48 | 0.217 | 1.68 |
| mock transfer active (20 x 4,000 rows, sending): shutdown | 5 | 61.8 | 62.1 | 0.525 | 62.0 |
| mock transfer active: restart after shutdown | 5 | 2.42 | 2.68 | 0.152 | 1.83 |

The interrupted job's durable final state was `Cancelled` in every sample. Shutdown with an active transfer takes about 62 ms, consistent with the 50 ms polling loop in `shutdown` plus the cancelled transfer's cleanup. The plan's render-active, database-write-active and connection-active shutdown cases were not measured.

## What this suggests

These are inferences from the numbers above, not verified outcomes. Set workload targets from them before changing code, as the plan requires.

### Step 2, bounded rendering

- Render work is not bounded. `spawn_blocking` allows up to 512 threads, and every request that passes the entry generation check renders in parallel. In a burst of 100 text previews fired at once, about 40 were rejected at entry and about 59 rendered completely and were discarded; only the newest returned. For 20 simultaneous 36 MP images, peak working set reached about 2.3 to 2.7 GB against 236 MiB for one, so memory grows with the number of in-flight decodes.
- Text at typing cadence is fine today (5 ms spacing: every call succeeded at about 1.3 ms). The risk is slow renders (images; PDF pages at larger sizes) issued faster than they finish, for example from a slider.
- A reasonable target: with at most two concurrent renders and a generation check before and during decode, the 20 x 36 MP burst should peak near two single-render footprints (a few hundred MiB) and the late-stale count should fall to near zero. The latest-preview latency should not exceed one render plus queueing (about 0.5 s for 36 MP); today it is 0.9 s.
- PDFium access is already serialised behind a mutex, so concurrent PDF renders gain nothing.

### Step 3, blocking-work boundaries

- A single store call is short: `list_jobs` takes 0.04 ms empty, 0.6 to 0.7 ms with 100 queued jobs and 1.4 to 2.0 ms with a 200-job result. `enqueue` takes about 2 ms and up to 6 ms for 20 copies of a 4,000-row page. At human-scale call rates that is unlikely to matter.
- Under sustained contention, direct calls from async tasks do delay other tasks measurably on a 2-worker runtime: p95 6 ms and max 12 ms against 0.03 ms and 0.13 ms idle. Wrapping calls in `spawn_blocking` removes that delay (p99 0.05 ms) but moves it into call latency (median 7.7 to 9 ms) and halves churn throughput, because the store is a single serialised mutex either way. So `spawn_blocking` protects the runtime and UI threads; it does not make the store faster.
- Whether this is worth doing depends on who calls the store: it matters for a frame-rate-sensitive UI thread that calls per event, not for a quiet runtime. The Slint event/model path was not measured, so that question stays open. A target such as "heartbeat p99 under 1 ms while four list loops and two churn loops run" is testable with this harness.
- Mutex hold and wait times need an instrumentation hook, which this step deliberately did not add to `service.rs`.
- Shutdown is cheap (0.03 ms idle, about 62 ms with a mock transfer, owner lock free about 2 ms later). The shared shutdown contract from this step can be adopted without concern for these paths; the 50 ms poll granularity is the only visible cost.

### Step 4, persistent log writer

- File logging costs about 115 µs per line on the calling thread (p95 210 to 380 µs, p99 about 500 µs, occasional 8 to 10 ms), against 0.2 µs for the in-memory path. The cost is stat + open + append + close every line. A job produces a handful of lines, so the steady cost is small, but diagnostic bursts of thousands of lines cost seconds of producer time (60,000 lines took 8 to 9.5 s), and any caller on a UI thread pays it directly.
- Rotation works and is not a latency cliff, but it keeps a single previous file and overwrites it. A persistent writer with a bounded channel could aim for a producer p99 of a few microseconds, with dropped-record counts and explicit flush on shutdown. The harness's logging section is the baseline for that comparison.
- Not measured: a real `tracing` subscriber, a failing or slow disk, and Slint's UI update cost per log event.

### Step 6, reusing page plans across copies

- Planning scales with copies: about 0.21 MiB allocated per 4,000-row copy at admission, and in execution about 10,300 allocations, 1.3 MiB and 5 ms of CPU per extra copy (a figure that includes the mock's own decoding). A 20-copy tall job costs about 100 to 125 ms of CPU and 29 MiB of allocation in total.
- Those costs are small beside the physical transfer. The SC03h profile sets 25 ms between chunks of at most 180 bytes plus 250 ms every 4,096 bytes; for one 4,000-row page (192 KB) that works out to roughly 40 s per copy (derived from the profile values, not measured). A saving of 5 ms per copy would not change print time.
- So step 6 is worth doing for allocation and retained-memory hygiene only if a target is set on bytes or allocation counts (for example, allocations scaling with unique page variants rather than copies), and the plan's rule applies: if that target is not worth the added complexity, record the optimisation as measured and deferred.

## Limits of these results

- One machine, one OS, two passes, with other processes active. Treat differences below about 20% as noise.
- Rows with `n` of 2 to 5 (36 MP images, bursts of large images, active-transfer shutdown) show the order of magnitude, not a tail.
- Allocation counts include the mock transport's own work. Memory is working set, not an allocator-level profile.
- PDF coverage is one generated three-line page.
- The harness binary used for these runs was built just before a one-line clippy fix to the PDF-unavailable message; the measured code paths are the same as `c75b47d`.
