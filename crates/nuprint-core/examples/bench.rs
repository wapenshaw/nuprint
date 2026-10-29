//! Release-mode, hardware-free measurement harness for `AppService` (follow-up plan, step 1).
//!
//! Run it in release mode, from the repository root. Nothing prints and no Bluetooth is used:
//! every transfer goes to the scriptable mock transport and every input is generated.
//!
//! ```text
//! # PDFium is only needed for the PDF cases; release builds do not search vendor/.
//! NUPRINT_PDFIUM_DIR=vendor/pdfium/win-x64/bin \
//!   cargo run -p nuprint-core --example bench --release -- all
//!
//! cargo run -p nuprint-core --example bench --release -- preview burst   # chosen sections
//! cargo run -p nuprint-core --example bench --release -- --quick all     # smoke test
//! ```
//!
//! Sections: `preview`, `burst`, `admission`, `exec`, `store`, `logging`, `shutdown`.
//! Run each section in its own process (a loop in the shell) when peak memory must be
//! attributed to it: the peak working set printed at the end of a section is process-wide.
//! Output is Markdown. Timings are wall-clock on a possibly busy machine, so compare runs on
//! the same machine and power plan, and never turn them into test thresholds. Counts of
//! allocations come from a counting global allocator that is enabled only around the
//! single-threaded `admission` and `exec` measurements.
//!
//! Only the public `AppService` API is used. Mutex wait/hold times inside the store are not
//! instrumented; the `store` section reports call latency and Tokio scheduling delay instead.

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use nuprint_core::api::*;
use nuprint_core::{AppResult, AppService, ErrorCode, ServiceConfig};
use nuprint_protocol::MonoBitmap;
use nuprint_protocol::iprint::{self, DIRECTION_RESPONSE, FrameDecoder, opcode};
use nuprint_transport::mock::{MockDevice, MockTransport, WriteHook};
use nuprint_transport::{
    Advertisement, CharProps, CharacteristicInfo, Endpoint, Notification, ServiceInfo, short_uuid,
};
use tokio::sync::mpsc;

const PROFILE: &str = "sc03h-iprint-384";
/// `limits.max_copies` of the SC03h profile.
const MAX_COPIES: u16 = 20;
/// The profile's default and maximum extra feed.
const DEFAULT_FEED: u16 = 12;

// ------------------------------------------------------------------ allocation counting

struct Counting;
static COUNT_ON: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

// SAFETY: forwards every call to `System`; the counters are plain atomics.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT_ON.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNT_ON.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNT_ON.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn count_start() {
    ALLOCS.store(0, Ordering::SeqCst);
    ALLOC_BYTES.store(0, Ordering::SeqCst);
    COUNT_ON.store(true, Ordering::SeqCst);
}

fn count_stop() -> (u64, u64) {
    COUNT_ON.store(false, Ordering::SeqCst);
    (ALLOCS.load(Ordering::SeqCst), ALLOC_BYTES.load(Ordering::SeqCst))
}

// ------------------------------------------------------------------ process memory

/// (current, peak) working set in bytes where the OS offers it cheaply.
#[cfg(windows)]
fn process_memory() -> Option<(u64, u64)> {
    #[repr(C)]
    struct Counters {
        cb: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged: usize,
        quota_paged: usize,
        quota_peak_nonpaged: usize,
        quota_nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
    }
    let mut c = Counters {
        cb: size_of::<Counters>() as u32,
        page_faults: 0,
        peak_working_set: 0,
        working_set: 0,
        quota_peak_paged: 0,
        quota_paged: 0,
        quota_peak_nonpaged: 0,
        quota_nonpaged: 0,
        pagefile: 0,
        peak_pagefile: 0,
    };
    // SAFETY: `c` is a correctly laid out PROCESS_MEMORY_COUNTERS with `cb` set.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    (ok != 0).then_some((c.working_set as u64, c.peak_working_set as u64))
}

#[cfg(target_os = "linux")]
fn process_memory() -> Option<(u64, u64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kib = |key: &str| -> Option<u64> {
        status.lines().find(|l| l.starts_with(key))?.split_whitespace().nth(1)?.parse::<u64>().ok().map(|k| k * 1024)
    };
    Some((kib("VmRSS:")?, kib("VmHWM:")?))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn process_memory() -> Option<(u64, u64)> {
    None
}

fn print_memory(label: &str) {
    match process_memory() {
        Some((now, peak)) => println!(
            "\nProcess memory after {label}: working set {:.0} MiB, peak working set {:.0} MiB (process-wide).",
            now as f64 / 1048576.0,
            peak as f64 / 1048576.0
        ),
        None => println!("\nProcess memory is not reported on this OS."),
    }
}

// ------------------------------------------------------------------ statistics

struct Stats {
    n: usize,
    min: f64,
    median: f64,
    p95: f64,
    p99: f64,
    max: f64,
    sd: f64,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// Summarise samples (already in the reporting unit).
fn stats(mut v: Vec<f64>) -> Stats {
    assert!(!v.is_empty(), "no samples");
    v.sort_by(f64::total_cmp);
    let n = v.len();
    let mean = v.iter().sum::<f64>() / n as f64;
    let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
    Stats {
        n,
        min: v[0],
        median: percentile(&v, 0.5),
        p95: percentile(&v, 0.95),
        p99: percentile(&v, 0.99),
        max: v[n - 1],
        sd: var.sqrt(),
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1_000_000.0
}

fn num(v: f64) -> String {
    if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else if v >= 1.0 {
        format!("{v:.2}")
    } else {
        format!("{v:.3}")
    }
}

fn table_header(unit: &str, first: &str) {
    println!("\n| {first} | n | median {unit} | p95 {unit} | min {unit} | max {unit} | sd {unit} |");
    println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: |");
}

fn table_row(name: &str, s: &Stats) {
    println!(
        "| {name} | {} | {} | {} | {} | {} | {} |",
        s.n,
        num(s.median),
        num(s.p95),
        num(s.min),
        num(s.max),
        num(s.sd)
    );
}

// ------------------------------------------------------------------ options

struct Opts {
    quick: bool,
    sections: Vec<String>,
}

impl Opts {
    /// Samples scaled down under `--quick`.
    fn n(&self, full: usize) -> usize {
        if self.quick { full.div_ceil(5).max(3) } else { full }
    }
}

// ------------------------------------------------------------------ environment

fn status_device(name: &str) -> MockDevice {
    let wwr = CharProps { write_without_response: true, ..Default::default() };
    let n = CharProps { notify: true, ..Default::default() };
    let notify = |op: u8, payload: &[u8]| Notification {
        endpoint: Endpoint { service: short_uuid(0xAE30), characteristic: short_uuid(0xAE02) },
        data: iprint::frame_with_direction(op, DIRECTION_RESPONSE, payload).unwrap(),
    };
    MockDevice {
        advertisement: Advertisement {
            backend_id: format!("id-{name}"),
            local_name: Some(name.into()),
            address: Some("AA:BB:CC:DD:EE:FF".into()),
            rssi: Some(-50),
            services: vec![short_uuid(0xAF30)],
            manufacturer_ids: vec![],
        },
        services: vec![ServiceInfo {
            uuid: short_uuid(0xAE30),
            characteristics: vec![
                CharacteristicInfo { uuid: short_uuid(0xAE01), properties: wwr },
                CharacteristicInfo { uuid: short_uuid(0xAE02), properties: n },
                CharacteristicInfo { uuid: short_uuid(0xAE04), properties: n },
            ],
        }],
        max_write: 180,
        hook: Arc::new(move || {
            let mut decoder = FrameDecoder::new();
            let hook: WriteHook = Box::new(move |_endpoint, data| {
                let mut out = Vec::new();
                for frame in decoder.feed(data) {
                    match frame.opcode {
                        opcode::STATUS => out.push(notify(opcode::STATUS, &[0, 0x12, 0x27])),
                        opcode::DEVICE_INFO => {
                            out.push(notify(opcode::DEVICE_INFO, &hex::decode("810003332e302e343900").unwrap()));
                        }
                        _ => {}
                    }
                }
                Ok(out)
            });
            hook
        }),
    }
}

/// A service over the mock transport with a temporary (or in-memory) data folder.
struct Env {
    service: AppService,
    events: mpsc::UnboundedReceiver<(String, JobState)>,
    root: tempfile::TempDir,
    data_dir: Option<PathBuf>,
    printer: Option<SavedPrinter>,
}

impl Env {
    fn start(persistent: bool) -> Env {
        let root = tempfile::tempdir().expect("temp dir");
        let data_dir = persistent.then(|| root.path().join("data"));
        let (service, events) = Env::open(data_dir.clone()).expect("start service");
        Env { service, events, root, data_dir, printer: None }
    }

    fn open(data_dir: Option<PathBuf>) -> AppResult<(AppService, mpsc::UnboundedReceiver<(String, JobState)>)> {
        let (tx, events) = mpsc::unbounded_channel();
        let service = AppService::start(
            ServiceConfig { data_dir, pdfium_dirs: nuprint_render::pdfium_search_dirs(&[]), version: "bench".into() },
            Arc::new(MockTransport::new(vec![status_device("SC03h-BENCH")])),
            Arc::new(move |e: AppEvent| {
                if let AppEvent::JobChanged { job, .. } = e {
                    let _ = tx.send((job.id, job.state));
                }
            }),
        )?;
        Ok((service, events))
    }

    async fn save_printer(&mut self) -> SavedPrinter {
        let found = self.service.find_first_supported(Duration::from_secs(5)).await.expect("mock printer");
        let printer = self
            .service
            .save_printer(SavePrinterRequest {
                discovery_id: found.discovery_id,
                profile_id: PROFILE.into(),
                acknowledge_experimental: true,
            })
            .expect("save printer");
        self.printer = Some(printer.clone());
        printer
    }

    /// Import a generated image of the given size as a source.
    async fn image_source(&self, name: &str, width: u32, height: u32) -> String {
        let bitmap = noise_bitmap(width, height);
        let path = self.root.path().join(name);
        std::fs::write(&path, nuprint_render::encode_png(&bitmap)).unwrap();
        self.service.import_source(&path).await.expect("import generated image").source_id
    }

    async fn pdf_source(&self) -> AppResult<String> {
        let path = self.root.path().join("generated.pdf");
        std::fs::write(&path, minimal_pdf()).unwrap();
        Ok(self.service.import_source(&path).await?.source_id)
    }

    fn enqueue(&self, snapshot: &PreviewResult, copies: u16, feed_mm: u16, key: String) -> AppResult<JobSummary> {
        self.service.enqueue(EnqueueRequest {
            snapshot_id: snapshot.snapshot_id.clone(),
            printer_id: self.printer.as_ref().expect("printer saved").id.clone(),
            copies,
            density: Density::Normal,
            feed_mm,
            idempotency_key: key,
            title: "Bench".into(),
        })
    }

    /// Cancel every non-terminal job and (optionally) clear history.
    fn drain(&self, clear: bool) {
        for job in self.service.list_jobs().unwrap().jobs {
            if !job.state.is_terminal() {
                let _ = self.service.cancel_job(&job.id);
            }
        }
        if clear {
            self.service.clear_history().unwrap();
        }
    }
}

/// Deterministic, poorly compressible 1-bit image.
fn noise_bitmap(width: u32, height: u32) -> MonoBitmap {
    MonoBitmap::from_fn(width, height, |x, y| {
        let mut h = u64::from(x).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ u64::from(y).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 32;
        h & 3 == 0
    })
    .unwrap()
}

/// A one-page PDF with a few lines of text (Helvetica), with a correct cross-reference table.
fn minimal_pdf() -> Vec<u8> {
    let mut content = String::from(
        "BT /F1 18 Tf 14 110 Td 22 TL (Generated benchmark page) Tj T* (Line two of text) Tj T* (Line three) Tj ET",
    );
    content.push('\n');
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 216 144] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_owned(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes(),
    );
    out
}

static GENERATION: AtomicU64 = AtomicU64::new(1);

fn next_generation() -> f64 {
    GENERATION.fetch_add(1, Ordering::Relaxed) as f64
}

fn preview_request(source: SourceSpec) -> PreviewRequest {
    PreviewRequest { generation: next_generation(), profile_id: PROFILE.into(), source }
}

fn text_spec(text: String) -> SourceSpec {
    SourceSpec::Text(TextSpec { text, ..TextSpec::default() })
}

fn long_text(chars: usize, salt: usize) -> String {
    let mut s = String::with_capacity(chars + 80);
    let mut line = 0;
    while s.len() < chars {
        s.push_str(&format!("Line {line} of generated text {salt} the quick brown fox jumps over the lazy dog\n"));
        line += 1;
    }
    s.truncate(chars);
    s
}

fn plain_adjust() -> ImageAdjust {
    ImageAdjust { tone: Tone::default(), rotation: Rotation::default(), brightness: 0, contrast: 0 }
}

fn image_spec(source_id: &str) -> SourceSpec {
    SourceSpec::Image { source_id: source_id.into(), adjust: plain_adjust() }
}

fn runtime(workers: usize) -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().worker_threads(workers).enable_all().build().unwrap()
}

// ------------------------------------------------------------------ 1. preview

async fn preview_section(o: &Opts) {
    println!("\n## Preview latency (one generation at a time)");
    let mut env = Env::start(false);
    env.save_printer().await;
    let small = env.image_source("small.png", 384, 164).await;
    let mid = env.image_source("mid.png", 2000, 2000).await;
    // MAX_DECODED_PIXELS is 40,000,000; 6000 x 6000 = 36,000,000.
    let big = env.image_source("big.png", 6000, 6000).await;
    let pdf = env.pdf_source().await;

    // Cold: the very first renders in this process (font parsing, PDFium load).
    println!("\nCold first call in this process (single sample each, includes font/PDFium initialisation):\n");
    println!("| case | ms |\n| --- | ---: |");
    let t = Instant::now();
    let r = env.service.render_preview(preview_request(text_spec("Cold start".into()))).await;
    println!("| text (first render) | {}{} |", num(ms(t.elapsed())), if r.is_err() { " (error)" } else { "" });
    match &pdf {
        Ok(id) => {
            let t = Instant::now();
            let r = env
                .service
                .render_preview(preview_request(SourceSpec::Pdf {
                    source_id: id.clone(),
                    page: 1,
                    adjust: plain_adjust(),
                    password: None,
                }))
                .await;
            println!(
                "| PDF page (first render) | {}{} |",
                num(ms(t.elapsed())),
                if r.is_err() { " (error)" } else { "" }
            );
        }
        Err(e) => println!(
            "| PDF page (first render) | unavailable: {:?}: {} |",
            e.code,
            e.detail.clone().unwrap_or_default()
        ),
    }

    let long = long_text(2000, 0);
    let near_limit = long_text(3000, 0);
    let over_limit = long_text(nuprint_render::MAX_TEXT_CHARS, 0);
    let mut cases: Vec<(&str, SourceSpec, usize)> = vec![
        ("text, short (5 chars)", text_spec("Hello".into()), 40),
        ("text, 2,000 chars", text_spec(long), 20),
        ("text, 3,000 chars (about 3,600 of the 4,000 rows allowed)", text_spec(near_limit), 10),
        ("text, 20,000 chars (accepted length, rejected as too tall; time to reject)", text_spec(over_limit), 5),
        ("test pattern", SourceSpec::TestPattern, 40),
        ("image 384 x 164", image_spec(&small), 30),
        ("image 2000 x 2000 (4 MP)", image_spec(&mid), 15),
        ("image 6000 x 6000 (36 MP, near the 40 MP limit)", image_spec(&big), 5),
    ];
    if let Ok(id) = &pdf {
        cases.push((
            "PDF page, 3 lines (warm)",
            SourceSpec::Pdf { source_id: id.clone(), page: 1, adjust: plain_adjust(), password: None },
            20,
        ));
    }
    println!(
        "\nWarm latency of `render_preview` after 2 warm-up calls (includes spawn_blocking, PNG encode, hashing):"
    );
    table_header("ms", "case");
    for (name, spec, n) in cases {
        for _ in 0..2 {
            let _ = env.service.render_preview(preview_request(spec.clone())).await;
        }
        let mut samples = Vec::new();
        let mut last_err = None;
        let mut rejected = Vec::new();
        for _ in 0..o.n(n) {
            let t = Instant::now();
            match env.service.render_preview(preview_request(spec.clone())).await {
                Ok(_) => samples.push(ms(t.elapsed())),
                Err(e) => {
                    last_err = Some(e);
                    rejected.push(ms(t.elapsed()));
                }
            }
        }
        match (samples.is_empty(), last_err) {
            (false, _) => table_row(name, &stats(samples)),
            (true, Some(e)) => {
                table_row(name, &stats(rejected));
                println!("|  | (all rejected: {:?}) | | | | | |", e.code);
            }
            _ => {}
        }
    }
    env.service.shutdown(Duration::from_secs(2)).await;
}

// ------------------------------------------------------------------ 2. burst

struct BurstRun {
    wall: Duration,
    latest: Option<Duration>,
    ok: usize,
    stale_entry: usize,
    stale_late: usize,
    other: usize,
    peak_ws_mib: u64,
}

/// Fire `n` generations (ascending, so only the last is current) and observe the outcomes.
/// `stale_entry` counts stale results returned within 2 ms (rejected before rendering); a
/// slower stale result did its render and discarded it afterwards.
async fn burst(env: &Env, n: usize, spacing: Duration, make: &(dyn Fn(usize) -> SourceSpec + Sync)) -> BurstRun {
    let base = GENERATION.fetch_add(n as u64, Ordering::SeqCst);
    let start = Instant::now();
    let mut tasks = Vec::new();
    for i in 0..n {
        let service = env.service.clone();
        let request =
            PreviewRequest { generation: (base + i as u64) as f64, profile_id: PROFILE.into(), source: make(i) };
        tasks.push(tokio::spawn(async move {
            let t = Instant::now();
            let r = service.render_preview(request).await;
            (t.elapsed(), r.map(|_| ()).map_err(|e| e.code))
        }));
        if !spacing.is_zero() {
            tokio::time::sleep(spacing).await;
        }
    }
    let mut run =
        BurstRun { wall: Duration::ZERO, latest: None, ok: 0, stale_entry: 0, stale_late: 0, other: 0, peak_ws_mib: 0 };
    for (i, task) in tasks.into_iter().enumerate() {
        let (took, result) = task.await.unwrap();
        match result {
            Ok(()) => {
                run.ok += 1;
                if i == n - 1 {
                    run.latest = Some(took);
                }
            }
            Err(ErrorCode::StalePreview) if took < Duration::from_millis(2) => run.stale_entry += 1,
            Err(ErrorCode::StalePreview) => run.stale_late += 1,
            Err(_) => run.other += 1,
        }
    }
    run.wall = start.elapsed();
    run.peak_ws_mib = process_memory().map_or(0, |(_, peak)| peak / 1048576);
    run
}

async fn burst_section(o: &Opts) {
    println!("\n## Preview bursts (ascending generations fired together or at typing cadence)");
    let mut env = Env::start(false);
    env.save_printer().await;
    let mid = env.image_source("mid.png", 2000, 2000).await;
    let big = env.image_source("big.png", 6000, 6000).await;
    let _ = env.service.render_preview(preview_request(text_spec("warm".into()))).await;
    let _ = env.service.render_preview(preview_request(text_spec(long_text(2000, 1)))).await;

    println!(
        "\nEach row is the median of the repetitions. `latest` is the latency of the newest generation (the only \
         one the user sees). `late stale` results rendered fully and were discarded afterwards; `early stale` \
         were rejected within 2 ms. `peak WS` is the process peak working set so far (cumulative; read the increases). \
         A stale count near zero means newer requests replaced waiting ones before they rendered.
"
    );
    println!(
        "| case | reps | latest ms | wall ms | ok | early stale | late stale | other | peak WS MiB (cumulative) |"
    );
    println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
    type Maker = Box<dyn Fn(usize) -> SourceSpec + Sync>;
    let mid2 = mid.clone();
    let big2 = big.clone();
    let cases: Vec<(String, usize, Duration, Maker, usize)> = vec![
        ("text 2,000 chars, 20 at once".into(), 20, Duration::ZERO, Box::new(|i| text_spec(long_text(2000, i))), 5),
        ("text 2,000 chars, 100 at once".into(), 100, Duration::ZERO, Box::new(|i| text_spec(long_text(2000, i))), 5),
        (
            "text 2,000 chars, 100 at 5 ms spacing".into(),
            100,
            Duration::from_millis(5),
            Box::new(|i| text_spec(long_text(2000, i))),
            3,
        ),
        ("image 4 MP, 20 at once".into(), 20, Duration::ZERO, Box::new(move |_| image_spec(&mid2)), 5),
        (
            "image 4 MP, 100 at once".into(),
            100,
            Duration::ZERO,
            Box::new({
                let id = mid.clone();
                move |_| image_spec(&id)
            }),
            3,
        ),
        ("image 36 MP, 20 at once".into(), 20, Duration::ZERO, Box::new(move |_| image_spec(&big2)), 2),
    ];
    for (name, n, spacing, make, reps) in cases {
        let mut runs = Vec::new();
        for _ in 0..o.n(reps).min(reps) {
            runs.push(burst(&env, n, spacing, make.as_ref()).await);
            // Let evicted snapshots and decode buffers go before the next repetition.
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let latest: Vec<f64> = runs.iter().filter_map(|r| r.latest).map(ms).collect();
        let wall: Vec<f64> = runs.iter().map(|r| ms(r.wall)).collect();
        let med = |f: &dyn Fn(&BurstRun) -> usize| stats(runs.iter().map(|r| f(r) as f64).collect()).median;
        println!(
            "| {name} | {} | {} | {} | {} | {} | {} | {} | {} |",
            runs.len(),
            if latest.is_empty() { "n/a".into() } else { num(stats(latest).median) },
            num(stats(wall).median),
            med(&|r| r.ok),
            med(&|r| r.stale_entry),
            med(&|r| r.stale_late),
            med(&|r| r.other),
            med(&|r| r.peak_ws_mib as usize),
        );
    }
    env.service.shutdown(Duration::from_secs(2)).await;
}

// ------------------------------------------------------------------ 3. admission

async fn admission_section(o: &Opts) {
    println!("\n## Admission (`enqueue`: validation, planning per copy, durable insert) with the queue held");
    println!(
        "\nThe caller blocks for the whole call. Allocation counts come from the counting allocator and include \
         every thread, but the held queue leaves the dispatcher idle. Encoder calls are not observable through \
         the public API; reading `enqueue` + `admit` shows one `plan_job` per copy and page plus one per page \
         in `admit`."
    );
    let mut env = Env::start(true);
    env.save_printer().await;
    env.service.hold_queue().unwrap();
    println!(
        "\n| bitmap | copies | feed mm | n | median ms | p95 ms | min ms | max ms | allocs (median) | MiB allocated (median) |"
    );
    println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
    for height in [164u32, 4000] {
        let source = env.image_source(&format!("a{height}.png"), 384, height).await;
        for copies in [1u16, 2, MAX_COPIES] {
            for feed in [0u16, DEFAULT_FEED] {
                let preview = env.service.render_preview(preview_request(image_spec(&source))).await.unwrap();
                let n = o.n(30);
                let mut times = Vec::new();
                let mut allocs = Vec::new();
                let mut bytes = Vec::new();
                for i in 0..n + 3 {
                    let key = format!("adm-{height}-{copies}-{feed}-{i}");
                    count_start();
                    let t = Instant::now();
                    let r = env.enqueue(&preview, copies, feed, key);
                    let took = t.elapsed();
                    let (a, b) = count_stop();
                    r.expect("enqueue");
                    if i >= 3 {
                        times.push(ms(took));
                        allocs.push(a as f64);
                        bytes.push(b as f64 / 1048576.0);
                    }
                }
                env.drain(true);
                let (t, a, b) = (stats(times), stats(allocs), stats(bytes));
                println!(
                    "| 384 x {height} | {copies} | {feed} | {} | {} | {} | {} | {} | {} | {} |",
                    t.n,
                    num(t.median),
                    num(t.p95),
                    num(t.min),
                    num(t.max),
                    num(a.median),
                    num(b.median)
                );
            }
        }
    }
    env.service.shutdown(Duration::from_secs(2)).await;
    print_memory("admission");
}

// ------------------------------------------------------------------ 4. execution through the mock

fn exec_section(o: &Opts) {
    println!("\n## Execution through the mock transport (CPU cost, pacing removed)");
    println!(
        "\nRuns on a current-thread Tokio runtime with paused virtual time, so pacing and ready-poll sleeps cost \
         no real time: the wall time below is the CPU work of planning, encoding, the mock exchange, status \
         handling and persistence. It says nothing about how long a real printer takes. The mock keeps every \
         written byte in memory. A new service is used per case.\n"
    );
    println!(
        "| bitmap | copies | feed mm | n | median ms | p95 ms | min ms | max ms | allocs (median) | MiB allocated (median) | final state |"
    );
    println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |");
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().start_paused(true).build().unwrap();
    rt.block_on(async {
        for height in [164u32, 4000] {
            for copies in [1u16, 2, MAX_COPIES] {
                for feed in [0u16, DEFAULT_FEED] {
                    let mut env = Env::start(true);
                    env.save_printer().await;
                    let source = env.image_source(&format!("e{height}.png"), 384, height).await;
                    let preview = env.service.render_preview(preview_request(image_spec(&source))).await.unwrap();
                    let n = o.n(5);
                    let (mut times, mut allocs, mut bytes) = (Vec::new(), Vec::new(), Vec::new());
                    let mut state = JobState::Queued;
                    for i in 0..n + 1 {
                        count_start();
                        let t = Instant::now();
                        let job = env.enqueue(&preview, copies, feed, format!("exec-{i}")).expect("enqueue");
                        loop {
                            let (id, s) = tokio::time::timeout(Duration::from_hours(1), env.events.recv())
                                .await
                                .expect("job finishes")
                                .expect("event channel");
                            if id == job.id && s.is_terminal() {
                                state = s;
                                break;
                            }
                        }
                        let took = t.elapsed();
                        let (a, b) = count_stop();
                        if i >= 1 {
                            times.push(ms(took));
                            allocs.push(a as f64);
                            bytes.push(b as f64 / 1048576.0);
                        }
                    }
                    let (t, a, b) = (stats(times), stats(allocs), stats(bytes));
                    println!(
                        "| 384 x {height} | {copies} | {feed} | {} | {} | {} | {} | {} | {} | {} | {state:?} |",
                        t.n,
                        num(t.median),
                        num(t.p95),
                        num(t.min),
                        num(t.max),
                        num(a.median),
                        num(b.median)
                    );
                    env.service.shutdown(Duration::from_secs(5)).await;
                }
            }
        }
    });
    print_memory("execution");
}

// ------------------------------------------------------------------ 5. store contention

/// Async scheduling delay. An OS thread spin-waits a precise 1 ms tick (Windows timers only
/// resolve to about 15.6 ms, so `tokio::time::sleep(1ms)` cannot measure this) and sends the tick
/// time to a task; the delay is how long after the tick the task ran. Returns delays in ms once
/// `stop` is set and the queued ticks are drained. The ticker occupies one core while it runs.
fn heartbeat(stop: Arc<AtomicBool>) -> tokio::task::JoinHandle<Vec<f64>> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Instant>();
    std::thread::spawn(move || {
        let mut next = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            next += Duration::from_millis(1);
            while Instant::now() < next {
                std::hint::spin_loop();
            }
            if tx.send(Instant::now()).is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        let mut delays = Vec::new();
        while let Some(sent) = rx.recv().await {
            delays.push(ms(sent.elapsed()));
        }
        delays
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Direct,
    Blocking,
}

async fn list_call(service: &AppService, style: Style) {
    match style {
        Style::Direct => {
            let _ = service.list_jobs();
        }
        Style::Blocking => {
            let s = service.clone();
            let _ = tokio::task::spawn_blocking(move || s.list_jobs()).await;
        }
    }
}

async fn churn_call(env: &Arc<Env>, preview: &Arc<PreviewResult>, style: Style, key: String) {
    let work = {
        let (env, preview) = (env.clone(), preview.clone());
        move || {
            if let Ok(job) = env.enqueue(&preview, 1, 0, key) {
                let _ = env.service.cancel_job(&job.id);
                let _ = env.service.remove_job(&job.id);
            }
        }
    };
    match style {
        Style::Direct => work(),
        Style::Blocking => {
            let _ = tokio::task::spawn_blocking(work).await;
        }
    }
}

/// Completed jobs lose their print data, so unlike cancelled ones they do not count against the
/// 100-job cap: this is how a long history is built. Runs on a paused-time runtime (no pacing).
fn seed_history(count: usize) -> (tempfile::TempDir, Option<PathBuf>) {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().start_paused(true).build().unwrap();
    rt.block_on(async {
        let mut env = Env::start(true);
        env.save_printer().await;
        let source = env.image_source("h.png", 384, 164).await;
        let preview = env.service.render_preview(preview_request(image_spec(&source))).await.unwrap();
        for i in 0..count {
            let job = env.enqueue(&preview, 1, 0, format!("hist-{i}")).unwrap();
            loop {
                let (id, s) = env.events.recv().await.unwrap();
                if id == job.id && s.is_terminal() {
                    break;
                }
            }
        }
        env.service.shutdown(Duration::from_secs(5)).await;
        let Env { service, events, root, data_dir, .. } = env;
        drop((service, events));
        (root, data_dir)
    })
}

/// Cancel and remove every queued/held/cancelled job, keeping completed history.
fn clear_queue(env: &Env) {
    for job in env.service.list_jobs().unwrap().jobs {
        if !job.state.is_terminal() {
            let _ = env.service.cancel_job(&job.id);
        }
    }
    for job in env.service.list_jobs().unwrap().jobs {
        if job.state == JobState::Cancelled {
            let _ = env.service.remove_job(&job.id);
        }
    }
}

fn fill(env: &Env, preview: &PreviewResult, queued: usize, tag: &str) {
    for i in 0..queued {
        env.enqueue(preview, 1, 0, format!("{tag}-q{i}")).unwrap();
    }
}

/// Reopen a seeded data folder in the current runtime, with the queue held.
async fn resume(root: tempfile::TempDir, data_dir: Option<PathBuf>) -> Env {
    let (service, events) = Env::open(data_dir.clone()).expect("reopen seeded folder");
    let printer = service.list_printers().unwrap().into_iter().next();
    service.hold_queue().unwrap();
    Env { service, events, root, data_dir, printer }
}

fn list_latency(o: &Opts, name: &str, env: &Env) {
    for _ in 0..20 {
        let _ = env.service.list_jobs();
    }
    let mut samples = Vec::new();
    for _ in 0..o.n(200) {
        let t = Instant::now();
        let q = env.service.list_jobs().unwrap();
        samples.push(us(t.elapsed()));
        std::hint::black_box(q);
    }
    table_row(name, &stats(samples));
}

fn store_section(o: &Opts) {
    println!("\n## Store contention");
    let history = if o.quick { 60 } else { 300 };
    let (root, data_dir) = seed_history(history);
    let rt = runtime(2);
    rt.block_on(async {
        // Environment A: no history. Environment B: `history` completed jobs.
        let mut a = Env::start(true);
        a.save_printer().await;
        a.service.hold_queue().unwrap();
        let b = resume(root, data_dir).await;
        let source = a.image_source("s.png", 384, 1000).await;
        let preview_a = a.service.render_preview(preview_request(image_spec(&source))).await.unwrap();
        let source_b = b.image_source("s.png", 384, 1000).await;
        let preview_b = b.service.render_preview(preview_request(image_spec(&source_b))).await.unwrap();

        println!("\n### `list_jobs` latency by queue size (sequential, called directly from an async task)");
        println!(
            "\nThe call returns at most 200 jobs and reads every job's pages. Queued jobs are held (they keep \
             their print data, so at most 100 can exist); history is completed jobs ({history} here)."
        );
        table_header("us", "state");
        list_latency(o, "empty", &a);
        fill(&a, &preview_a, 20, "l20");
        list_latency(o, "20 queued", &a);
        fill(&a, &preview_a, 80, "l100");
        list_latency(o, "100 queued (the retained-job cap)", &a);
        list_latency(o, &format!("{history} completed (list capped at 200)"), &b);
        fill(&b, &preview_b, 100, "h100");
        list_latency(o, &format!("100 queued + {history} completed"), &b);

        // Concurrent phase on B: room for churn, a long history, 50 queued jobs.
        clear_queue(&b);
        fill(&b, &preview_b, 50, "c");
        let env = Arc::new(b);
        let preview = Arc::new(preview_b);
        let duration = if o.quick { Duration::from_millis(800) } else { Duration::from_secs(4) };
        println!(
            "\n### Concurrent calls and runtime scheduling delay\n\nTwo Tokio worker threads; state 50 queued + {history} completed \
             history. Each looper yields between calls. `heartbeat` is how long after a precise 1 ms OS-thread tick \
             a task ran (async scheduling delay). `direct` calls the sync API from the async task; \
             `blocking` wraps each call in `spawn_blocking`. A churn operation is enqueue + cancel + remove. \
             Duration {:.1} s per scenario.\n",
            duration.as_secs_f64()
        );
        println!("| scenario | heartbeat median ms | heartbeat p95 ms | heartbeat p99 ms | heartbeat max ms | list calls | list median us | list p95 us | list max us | churn ops |");
        println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
        let scenarios: [(&str, usize, usize, Style); 4] = [
            ("heartbeat only (baseline)", 0, 0, Style::Direct),
            ("4 list loops, direct", 4, 0, Style::Direct),
            ("4 list loops + 2 churn loops, direct", 4, 2, Style::Direct),
            ("4 list loops + 2 churn loops, blocking", 4, 2, Style::Blocking),
        ];
        for (si, (name, listers, churners, style)) in scenarios.into_iter().enumerate() {
            let stop = Arc::new(AtomicBool::new(false));
            let beat = heartbeat(stop.clone());
            let mut tasks = Vec::new();
            for _ in 0..listers {
                let (service, stop) = (env.service.clone(), stop.clone());
                tasks.push(tokio::spawn(async move {
                    let mut lat = Vec::new();
                    while !stop.load(Ordering::Relaxed) {
                        let t = Instant::now();
                        list_call(&service, style).await;
                        lat.push(us(t.elapsed()));
                        tokio::task::yield_now().await;
                    }
                    lat
                }));
            }
            let mut churn = Vec::new();
            for c in 0..churners {
                let (env, preview, stop) = (env.clone(), preview.clone(), stop.clone());
                churn.push(tokio::spawn(async move {
                    let mut ops = 0usize;
                    while !stop.load(Ordering::Relaxed) {
                        churn_call(&env, &preview, style, format!("churn-{si}-{c}-{ops}")).await;
                        ops += 1;
                        tokio::task::yield_now().await;
                    }
                    ops
                }));
            }
            tokio::time::sleep(duration).await;
            stop.store(true, Ordering::Relaxed);
            let mut lat = Vec::new();
            for t in tasks {
                lat.extend(t.await.unwrap());
            }
            let mut ops = 0;
            for c in churn {
                ops += c.await.unwrap();
            }
            let h = stats(beat.await.unwrap());
            let (calls, lm, lp, lx) = if lat.is_empty() {
                (0, "-".into(), "-".into(), "-".into())
            } else {
                let s = stats(lat);
                (s.n, num(s.median), num(s.p95), num(s.max))
            };
            println!(
                "| {name} | {} | {} | {} | {} | {calls} | {lm} | {lp} | {lx} | {ops} |",
                num(h.median),
                num(h.p95),
                num(h.p99),
                num(h.max)
            );
        }
        let env = Arc::try_unwrap(env).ok().expect("single owner");
        env.service.shutdown(Duration::from_secs(2)).await;
        a.service.shutdown(Duration::from_secs(2)).await;
    });
    print_memory("store");
}

// ------------------------------------------------------------------ 6. logging

fn log_message(i: usize) -> String {
    format!("Bench line {i}: transfer progress {} of 1000 chunks, status ready, no errors reported", i % 1000)
}

async fn log_run(label: &str, persistent: bool, lines: usize) {
    let env = Env::start(persistent);
    for i in 0..200 {
        env.service.log(LogLevel::Info, log_message(i));
    }
    let mut lat = Vec::with_capacity(lines);
    let total = Instant::now();
    for i in 0..lines {
        let t = Instant::now();
        env.service.log(LogLevel::Info, log_message(i));
        lat.push(us(t.elapsed()));
    }
    let total = total.elapsed();
    let files = env
        .data_dir
        .as_ref()
        .map(|d| {
            let size = |n: &str| std::fs::metadata(d.join("logs").join(n)).map(|m| m.len()).ok();
            format!(
                "nuprint.log {} B, nuprint.log.1 {}",
                size("nuprint.log").unwrap_or(0),
                size("nuprint.log.1").map_or("absent".into(), |s| format!("{s} B"))
            )
        })
        .unwrap_or_else(|| "no file (in-memory)".into());
    let worst: Vec<usize> = {
        let mut idx: Vec<usize> = (0..lat.len()).collect();
        idx.sort_by(|a, b| lat[*b].total_cmp(&lat[*a]));
        idx.truncate(3);
        idx
    };
    let s = stats(lat.clone());
    let p999 = percentile(
        &{
            let mut v = lat;
            v.sort_by(f64::total_cmp);
            v
        },
        0.999,
    );
    println!(
        "| {label} | {} | {} | {} | {} | {} | {} | {:.0} | {:?} | {files} |",
        s.n,
        num(s.median),
        num(s.p95),
        num(s.p99),
        num(p999),
        num(s.max),
        total.as_secs_f64() * 1000.0,
        worst
    );
    env.service.shutdown(Duration::from_secs(1)).await;
}

fn logging_section(o: &Opts) {
    println!("\n## Logging (`AppService::log`, per call, producer-side)");
    println!(
        "\nEach call appends to the in-memory ring, emits to the (counting) event sink, calls `tracing` (no \
         subscriber installed here, so that cost is a floor) and, with a data folder, stats, opens, appends and \
         closes `logs/nuprint.log`. Rotation happens at 2,000,000 bytes (about 20,000 of these lines). Lines are \
         about 95 bytes. `worst idx` are the indices of the three slowest calls.\n"
    );
    println!("| case | lines | median us | p95 us | p99 us | p99.9 us | max us | total ms | worst idx | files after |");
    println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |");
    let rt = runtime(2);
    rt.block_on(async {
        log_run("in-memory service (no file)", false, o.n(20_000)).await;
        log_run("file, ordinary volume", true, o.n(2_000)).await;
        log_run("file, burst crossing rotation", true, o.n(60_000)).await;

        // The same burst while a 1 ms heartbeat runs on the same runtime.
        let env = Env::start(true);
        let lines = o.n(60_000);
        let stop = Arc::new(AtomicBool::new(false));
        let quiet = {
            let stop = Arc::new(AtomicBool::new(false));
            let beat = heartbeat(stop.clone());
            tokio::time::sleep(Duration::from_millis(500)).await;
            stop.store(true, Ordering::Relaxed);
            stats(beat.await.unwrap())
        };
        let beat = heartbeat(stop.clone());
        let service = env.service.clone();
        let producer = tokio::spawn(async move {
            for i in 0..lines {
                service.log(LogLevel::Info, log_message(i));
                if i % 50 == 49 {
                    tokio::task::yield_now().await;
                }
            }
        });
        producer.await.unwrap();
        stop.store(true, Ordering::Relaxed);
        let busy = stats(beat.await.unwrap());
        println!(
            "\nHeartbeat delay on the shared runtime (2 workers): idle median {} ms, p99 {} ms, max {} ms; \
             during a {lines}-line burst logged from an async task (yielding every 50 lines) median {} ms, p99 {} ms, max {} ms.",
            num(quiet.median),
            num(quiet.p99),
            num(quiet.max),
            num(busy.median),
            num(busy.p99),
            num(busy.max)
        );
        env.service.shutdown(Duration::from_secs(1)).await;
    });
    print_memory("logging");
}

// ------------------------------------------------------------------ 7. shutdown

fn shutdown_section(o: &Opts) {
    println!("\n## Shutdown (`AppService::shutdown`)");
    println!(
        "\nReal time (not paused), so the pacing of a mock transfer is real. `restart` is how long after shutdown \
         returned a new service needed to open the same data folder (the owner lock is released when the last \
         service handle drops). Wait limit given to shutdown: 10 s.\n"
    );
    let rt = runtime(2);
    rt.block_on(async {
        // Idle.
        let (mut shut, mut restart) = (Vec::new(), Vec::new());
        for _ in 0..o.n(15) {
            let mut env = Env::start(true);
            env.save_printer().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            let t = Instant::now();
            env.service.shutdown(Duration::from_secs(10)).await;
            shut.push(ms(t.elapsed()));
            restart.push(reopen_delay(env).await);
        }
        table_header("ms", "case");
        table_row("idle: shutdown", &stats(shut));
        table_row("idle: restart after shutdown", &stats(restart));

        // Mock transfer active: a long job (20 copies of 384 x 4000), shut down once it is sending.
        let (mut shut, mut restart) = (Vec::new(), Vec::new());
        let mut states = Vec::new();
        for _ in 0..o.n(5).min(5) {
            let mut env = Env::start(true);
            env.save_printer().await;
            let source = env.image_source("t.png", 384, 4000).await;
            let preview = env.service.render_preview(preview_request(image_spec(&source))).await.unwrap();
            let job = env.enqueue(&preview, MAX_COPIES, DEFAULT_FEED, "shutdown".into()).unwrap();
            loop {
                let (id, s) = tokio::time::timeout(Duration::from_secs(60), env.events.recv())
                    .await
                    .expect("job starts sending")
                    .expect("event channel");
                if id == job.id && s == JobState::Sending {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            let t = Instant::now();
            env.service.shutdown(Duration::from_secs(10)).await;
            shut.push(ms(t.elapsed()));
            states.push(format!("{:?}", env.service.get_job(&job.id).map(|j| j.state)));
            restart.push(reopen_delay(env).await);
        }
        table_row("mock transfer active: shutdown", &stats(shut));
        table_row("mock transfer active: restart after shutdown", &stats(restart));
        states.sort();
        states.dedup();
        println!("\nFinal durable state of the interrupted job across samples: {}.", states.join(", "));
    });
}

/// Drop the environment and time how long reopening its data folder takes to succeed.
async fn reopen_delay(env: Env) -> f64 {
    let dir: Option<PathBuf> = env.data_dir.clone();
    let root_guard = env.root;
    drop((env.service, env.events));
    let t = Instant::now();
    loop {
        match Env::open(dir.clone()) {
            Ok((service, _events)) => {
                let took = ms(t.elapsed());
                service.shutdown(Duration::from_secs(2)).await;
                drop(root_guard);
                return took;
            }
            Err(_) if t.elapsed() < Duration::from_secs(5) => tokio::time::sleep(Duration::from_millis(5)).await,
            Err(e) => panic!("could not reopen the data folder: {e}"),
        }
    }
}

// ------------------------------------------------------------------ main

fn header() {
    println!("# NuPrint benchmark run");
    println!(
        "\n- crate nuprint-core {}, build {}",
        env!("CARGO_PKG_VERSION"),
        if cfg!(debug_assertions) { "DEBUG (numbers are not meaningful; use --release)" } else { "release" }
    );
    println!(
        "- os/arch {} {}, logical CPUs {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(0, |n| n.get())
    );
    println!("- NUPRINT_PDFIUM_DIR {}", std::env::var("NUPRINT_PDFIUM_DIR").unwrap_or_else(|_| "(unset)".into()));
    println!("- transport: MockTransport only; no Bluetooth, no paper");
}

fn main() {
    let mut opts = Opts { quick: false, sections: Vec::new() };
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--quick" => opts.quick = true,
            "all" => {
                opts.sections =
                    ["preview", "burst", "admission", "exec", "store", "logging", "shutdown"].map(String::from).to_vec()
            }
            s if !s.starts_with('-') => opts.sections.push(s.to_owned()),
            other => {
                eprintln!("unknown option {other}");
                std::process::exit(2);
            }
        }
    }
    if opts.sections.is_empty() {
        eprintln!("usage: bench [--quick] all | preview burst admission exec store logging shutdown");
        std::process::exit(2);
    }
    header();
    if opts.quick {
        println!("- QUICK mode: reduced samples, smoke test only");
    }
    for section in opts.sections.clone() {
        match section.as_str() {
            "preview" => {
                runtime(4).block_on(preview_section(&opts));
                print_memory("preview");
            }
            "burst" => {
                runtime(8).block_on(burst_section(&opts));
                print_memory("burst");
            }
            "admission" => runtime(2).block_on(admission_section(&opts)),
            "exec" => exec_section(&opts),
            "store" => store_section(&opts),
            "logging" => logging_section(&opts),
            "shutdown" => shutdown_section(&opts),
            other => {
                eprintln!("unknown section {other}");
                std::process::exit(2);
            }
        }
    }
}
