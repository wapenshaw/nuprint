//! The application service: one instance owns the queue, dispatcher, storage and all
//! Bluetooth activity for this process. The Tauri host and the CLI both drive it.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use nuprint_protocol::{CompletionPolicy, Density, ExchangePlan, MonoBitmap, PrintRequest, PrinterDriver};
use nuprint_render::{
    ImageParams, PdfEngine, RENDERER_VERSION, RenderError, RenderOutput, RenderTarget, RenderWarning, SourceFormat,
    TextParams,
};
use nuprint_transport::{AdapterState, Advertisement, ServiceInfo, Transport};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::api::*;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::jobs::{classify, summarize};
use crate::profile::{ModelProfile, ProfileRegistry, ResolvedProfile};
use crate::session::{LogFn, PrinterSession, SessionEvent, locate};
use crate::store::{CustomProfileRow, JobRow, PrinterRow, Store, now_ms};

pub const MAX_QUEUE_JOBS: i64 = 100;
pub const MAX_RETAINED_BYTES: i64 = 256 * 1024 * 1024;
const MAX_LOG_LINES: usize = 500;
/// IDs of profiles edited on this computer start with this, followed by the printer's ID.
const CUSTOM_PROFILE_PREFIX: &str = "custom:";
const MAX_SNAPSHOTS: usize = 16;
const MAX_SOURCES: usize = 8;
const LOG_ROTATE_BYTES: u64 = 2_000_000;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);

/// Receives events for the UI or CLI. Must not block.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: AppEvent);
}

impl<F: Fn(AppEvent) + Send + Sync + 'static> EventSink for F {
    fn emit(&self, event: AppEvent) {
        self(event)
    }
}

pub struct ServiceConfig {
    /// Persistent data directory; `None` keeps state in memory (CLI one-shot use).
    pub data_dir: Option<PathBuf>,
    /// Directories searched for the PDFium library.
    pub pdfium_dirs: Vec<PathBuf>,
    pub version: String,
}

struct Source {
    format: SourceFormat,
    bytes: Arc<Vec<u8>>,
}

struct Snapshot {
    profile_id: String,
    profile_revision: u32,
    source_kind: &'static str,
    settings_json: String,
    /// (1-based source page, canonical bitmap)
    pages: Vec<(u32, MonoBitmap)>,
    png: Arc<Vec<u8>>,
}

/// The most recent entries by ID; the oldest is dropped beyond `cap`.
struct Recent<T> {
    cap: usize,
    items: Mutex<VecDeque<(String, Arc<T>)>>,
}

impl<T> Recent<T> {
    fn new(cap: usize) -> Self {
        Self { cap, items: Mutex::new(VecDeque::new()) }
    }

    fn insert(&self, id: String, value: T) {
        let mut items = self.items.lock().unwrap();
        if items.len() >= self.cap {
            items.pop_front();
        }
        items.push_back((id, Arc::new(value)));
    }

    fn get(&self, id: &str) -> Option<Arc<T>> {
        self.items.lock().unwrap().iter().find(|(key, _)| key == id).map(|(_, value)| value.clone())
    }
}

struct ActiveJob {
    job_id: String,
    cancel: CancellationToken,
}

struct Inner {
    config: ServiceConfig,
    registry: ProfileRegistry,
    transport: Arc<dyn Transport>,
    store: Mutex<Store>,
    events: Arc<dyn EventSink>,
    seq: AtomicU64,
    logs: Mutex<VecDeque<LogLine>>,
    log_file: Option<PathBuf>,
    pdf: Mutex<Option<AppResult<PdfEngine>>>,
    sources: Recent<Source>,
    snapshots: Recent<Snapshot>,
    latest_generation: Mutex<f64>,
    scans: Mutex<HashMap<String, CancellationToken>>,
    inspections: Mutex<HashMap<String, CancellationToken>>,
    discovered: Mutex<HashMap<String, Advertisement>>,
    wake: Notify,
    held: AtomicBool,
    active: Mutex<Option<ActiveJob>>,
    /// Serializes Bluetooth sessions across devices until adapter concurrency is measured.
    hardware: tokio::sync::Mutex<()>,
    /// What currently holds `hardware`, so a refusal can say what to wait for.
    hardware_use: Mutex<Option<HardwareUse>>,
    shutdown: CancellationToken,
    dispatcher: Mutex<Option<tokio::task::JoinHandle<()>>>,
    last_check: Mutex<Option<PrinterCheck>>,
    _temp: Option<tempfile::TempDir>,
}

#[derive(Clone)]
pub struct AppService {
    inner: Arc<Inner>,
}

/// The settings stored with a job: how it was made, without what it says. The text itself and a
/// PDF's password are never written to the database; a retry reprints the saved raster pages.
fn persisted_settings(spec: &SourceSpec) -> AppResult<String> {
    let mut value = serde_json::to_value(spec).map_err(AppError::internal)?;
    if let Some(object) = value.as_object_mut() {
        object.remove("text");
        object.remove("password");
    }
    serde_json::to_string(&value).map_err(AppError::internal)
}

/// Log text that identifies a printer, a document or a person is wrapped in these markers where it
/// is written. They are stripped for the screen and the log file, and the diagnostics export
/// replaces what they enclose unless identifiers were asked for. Redacting at the source keeps
/// working for printers and jobs that have since been removed.
const PRIVATE_START: char = '\u{1}';
const PRIVATE_END: char = '\u{2}';

fn private(text: &str) -> String {
    // A marker inside the text would end the span early and leave the rest of it unredacted.
    let text = text.replace([PRIVATE_START, PRIVATE_END], "");
    format!("{PRIVATE_START}{text}{PRIVATE_END}")
}

/// `text` with every marked span replaced (`Some`) or just unmarked (`None`).
fn unmark(text: &str, replacement: Option<&str>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for c in text.chars() {
        match c {
            PRIVATE_START => {
                inside = true;
                if let Some(r) = replacement {
                    out.push_str(r);
                }
            }
            PRIVATE_END => inside = false,
            _ if inside && replacement.is_some() => {}
            _ => out.push(c),
        }
    }
    out
}

fn source_kind_name(spec: &SourceSpec) -> &'static str {
    match spec {
        SourceSpec::Text(_) => "text",
        SourceSpec::Image { .. } => "image",
        SourceSpec::Pdf { .. } => "pdf",
        SourceSpec::TestPattern => "test_pattern",
    }
}

fn image_params(adjust: &ImageAdjust) -> ImageParams {
    ImageParams {
        tone: match adjust.tone {
            Tone::Dither => nuprint_render::Tone::Dither,
            Tone::Atkinson => nuprint_render::Tone::Atkinson,
            Tone::Threshold => nuprint_render::Tone::Threshold,
        },
        rotation: match adjust.rotation {
            Rotation::R0 => nuprint_render::Rotation::R0,
            Rotation::R90 => nuprint_render::Rotation::R90,
            Rotation::R180 => nuprint_render::Rotation::R180,
            Rotation::R270 => nuprint_render::Rotation::R270,
        },
        brightness: adjust.brightness,
        contrast: adjust.contrast,
    }
}

fn text_params(spec: &TextSpec) -> TextParams {
    TextParams {
        text: spec.text.clone(),
        size_px: spec.size_px,
        family: match spec.family {
            FontFamily::Sans => nuprint_render::FontFamily::Sans,
            FontFamily::Mono => nuprint_render::FontFamily::Mono,
        },
        bold: spec.bold,
        align: match spec.align {
            TextAlign::Left => nuprint_render::TextAlign::Left,
            TextAlign::Center => nuprint_render::TextAlign::Center,
            TextAlign::Right => nuprint_render::TextAlign::Right,
        },
        line_spacing: spec.line_spacing,
        margin_dots: spec.margin_dots,
        headings: spec.headings,
    }
}

fn warning_text(warning: &RenderWarning) -> String {
    match warning {
        RenderWarning::MissingGlyphs { chars } => {
            let list: String = chars.iter().take(12).collect();
            format!("The bundled fonts cannot print these characters; they will appear as boxes: {list}")
        }
        RenderWarning::BlankOutput => "Nothing would print: the preview has no black dots.".into(),
        RenderWarning::ContentScaledDown { factor } => {
            format!(
                "The page was scaled to {:.0}% to fit the paper width; small text may be unreadable.",
                factor * 100.0
            )
        }
    }
}

fn gatt_services(services: &[ServiceInfo]) -> Vec<GattService> {
    services
        .iter()
        .map(|s| GattService {
            uuid: s.uuid.to_string(),
            characteristics: s
                .characteristics
                .iter()
                .map(|c| {
                    let p = c.properties;
                    let props = [
                        (p.read, "read"),
                        (p.write, "write"),
                        (p.write_without_response, "write_without_response"),
                        (p.notify, "notify"),
                        (p.indicate, "indicate"),
                    ];
                    GattCharacteristic {
                        uuid: c.uuid.to_string(),
                        properties: props.iter().filter(|(on, _)| *on).map(|(_, n)| (*n).to_owned()).collect(),
                    }
                })
                .collect(),
        })
        .collect()
}

fn adapter_status(state: AdapterState) -> AdapterStatus {
    match state {
        AdapterState::PoweredOn => AdapterStatus::Ready,
        AdapterState::PoweredOff => AdapterStatus::PoweredOff,
        AdapterState::Absent => AdapterStatus::Absent,
        AdapterState::PermissionDenied => AdapterStatus::PermissionDenied,
        AdapterState::PermissionNotAsked => AdapterStatus::PermissionNotAsked,
        AdapterState::PermissionPending => AdapterStatus::PermissionPending,
        AdapterState::Unknown => AdapterStatus::Unknown,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HardwareUse {
    Printing,
    Checking,
    Inspecting,
}

/// The Bluetooth lock, recording what holds it until dropped.
struct HardwareClaim<'a> {
    _guard: tokio::sync::MutexGuard<'a, ()>,
    holder: &'a Mutex<Option<HardwareUse>>,
}

impl Drop for HardwareClaim<'_> {
    fn drop(&mut self) {
        // Runs before `_guard` is released, so it never clears a newer holder.
        *self.holder.lock().unwrap() = None;
    }
}

fn busy(holder: Option<HardwareUse>) -> AppError {
    let message = match holder {
        Some(HardwareUse::Printing) => "A print is in progress. Try again when it finishes.",
        Some(HardwareUse::Checking) => "A connection check is in progress. Try again when it finishes.",
        Some(HardwareUse::Inspecting) => "A device inspection is in progress. Try again when it finishes.",
        None => "Another Bluetooth operation is in progress. Try again when it finishes.",
    };
    AppError::new(ErrorCode::PrinterBusy, message)
}

impl AppService {
    /// Open storage, recover interrupted jobs and start the dispatcher.
    /// Must be called within a Tokio runtime.
    pub fn start(config: ServiceConfig, transport: Arc<dyn Transport>, events: Arc<dyn EventSink>) -> AppResult<Self> {
        let registry = ProfileRegistry::builtin()?;
        let (mut store, temp, log_file) = match &config.data_dir {
            Some(dir) => {
                fs::create_dir_all(dir.join("logs"))?;
                (Store::open(dir)?, None, Some(dir.join("logs").join("nuprint.log")))
            }
            None => {
                let temp = tempfile::tempdir()?;
                (Store::open_in_memory(temp.path())?, Some(temp), None)
            }
        };
        let (uncertain, held_jobs) = store.recover_after_restart()?;
        store.apply_retention(now_ms())?;
        // A stored profile that no longer validates is skipped: its printer then reports the
        // profile as unavailable instead of the app failing to start.
        for row in store.list_custom_profiles()? {
            if let Ok(profile) = ModelProfile::parse(&row.json) {
                let _ = registry.add_custom(profile);
            }
        }
        let held = store.get_pref::<bool>("queue_held")?.unwrap_or(false);
        let inner = Arc::new(Inner {
            config,
            registry,
            transport,
            store: Mutex::new(store),
            events,
            seq: AtomicU64::new(0),
            logs: Mutex::new(VecDeque::new()),
            log_file,
            pdf: Mutex::new(None),
            sources: Recent::new(MAX_SOURCES),
            snapshots: Recent::new(MAX_SNAPSHOTS),
            latest_generation: Mutex::new(f64::MIN),
            scans: Mutex::new(HashMap::new()),
            inspections: Mutex::new(HashMap::new()),
            discovered: Mutex::new(HashMap::new()),
            wake: Notify::new(),
            held: AtomicBool::new(held),
            active: Mutex::new(None),
            hardware: tokio::sync::Mutex::new(()),
            hardware_use: Mutex::new(None),
            shutdown: CancellationToken::new(),
            dispatcher: Mutex::new(None),
            last_check: Mutex::new(None),
            _temp: temp,
        });
        let service = Self { inner };
        service.log(
            LogLevel::Info,
            format!("NuPrint {} started on {}", service.inner.config.version, std::env::consts::OS),
        );
        if uncertain > 0 {
            service
                .log(LogLevel::Warn, format!("{uncertain} job(s) were interrupted by a restart and marked uncertain"));
        }
        if held_jobs > 0 {
            service.log(LogLevel::Info, format!("{held_jobs} queued job(s) were held after restart"));
        }
        let dispatcher = service.clone();
        let task = tokio::spawn(async move { dispatcher.dispatch_loop().await });
        *service.inner.dispatcher.lock().unwrap() = Some(task);
        Ok(service)
    }

    /// Stop accepting work, cancel the active transfer and wait (bounded) for its outcome
    /// to be persisted. Pending jobs stay queued/held for the next start.
    pub async fn shutdown(&self, wait: Duration) {
        self.inner.shutdown.cancel();
        if let Some(active) = self.inner.active.lock().unwrap().as_ref() {
            active.cancel.cancel();
        }
        let deadline = Instant::now() + wait;
        while self.inner.active.lock().unwrap().is_some() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // The dispatcher holds the service (and so the data folder's owner lock) until it exits.
        let task = self.inner.dispatcher.lock().unwrap().take();
        if let Some(task) = task {
            let _ = tokio::time::timeout(wait, task).await;
        }
    }

    fn store(&self) -> MutexGuard<'_, Store> {
        self.inner.store.lock().unwrap()
    }

    fn next_seq(&self) -> f64 {
        self.inner.seq.fetch_add(1, Ordering::Relaxed) as f64 + 1.0
    }

    fn emit(&self, event: AppEvent) {
        self.inner.events.emit(event);
    }

    // ------------------------------------------------------------ logging

    pub fn log(&self, level: LogLevel, message: String) {
        let line = LogLine { seq: self.next_seq(), at_ms: now_ms() as f64, level, message };
        {
            let mut logs = self.inner.logs.lock().unwrap();
            if logs.len() >= MAX_LOG_LINES {
                logs.pop_front();
            }
            logs.push_back(line.clone());
        }
        let line = LogLine { message: unmark(&line.message, None), ..line };
        match level {
            LogLevel::Error => tracing::error!("{}", line.message),
            LogLevel::Warn => tracing::warn!("{}", line.message),
            LogLevel::Info => tracing::info!("{}", line.message),
            LogLevel::Debug => tracing::debug!("{}", line.message),
        }
        if let Some(path) = &self.inner.log_file {
            if fs::metadata(path).is_ok_and(|m| m.len() > LOG_ROTATE_BYTES) {
                let _ = fs::rename(path, path.with_added_extension("1"));
            }
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "{} {:?} {}", line.at_ms as i64, line.level, line.message);
            }
        }
        self.emit(AppEvent::Log { line });
    }

    fn logger(&self) -> LogFn {
        let service = self.clone();
        Arc::new(move |level, message| service.log(level, message))
    }

    pub fn logs(&self) -> Vec<LogLine> {
        self.inner
            .logs
            .lock()
            .unwrap()
            .iter()
            .map(|l| LogLine { message: unmark(&l.message, None), ..l.clone() })
            .collect()
    }

    /// The log for a diagnostics export: identifying text is replaced unless `include_identifiers`.
    fn export_logs(&self, include_identifiers: bool) -> Vec<LogLine> {
        if include_identifiers {
            return self.logs();
        }
        let replacement = Some("<redacted>");
        self.inner
            .logs
            .lock()
            .unwrap()
            .iter()
            .map(|l| LogLine { message: unmark(&l.message, replacement), ..l.clone() })
            .collect()
    }

    // ------------------------------------------------------------ app & profiles

    fn pdf_engine(&self) -> AppResult<PdfEngine> {
        let mut pdf = self.inner.pdf.lock().unwrap();
        pdf.get_or_insert_with(|| {
            PdfEngine::load(&self.inner.config.pdfium_dirs).map_err(|e| {
                AppError::new(ErrorCode::PdfUnavailable, "PDF support is unavailable in this build.")
                    .with_detail(e.to_string())
            })
        })
        .clone()
    }

    pub fn app_info(&self) -> AppInfo {
        let pdf = self.pdf_engine();
        AppInfo {
            version: self.inner.config.version.clone(),
            renderer_version: RENDERER_VERSION.to_owned(),
            os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            data_dir: self.inner.config.data_dir.as_ref().map(|d| d.display().to_string()),
            pdf_available: pdf.is_ok(),
            pdf_error: pdf.err().and_then(|e| e.detail),
            persistent: self.inner.config.data_dir.is_some(),
        }
    }

    pub fn profiles(&self) -> Vec<ProfileSummary> {
        self.inner.registry.summaries()
    }

    pub async fn adapter_status(&self) -> AdapterStatus {
        adapter_status(self.inner.transport.adapter_state().await)
    }

    // ------------------------------------------------------------ discovery

    fn discovered_printer(&self, adv: &Advertisement) -> DiscoveredPrinter {
        let os = self.inner.registry.os();
        let candidates: Vec<ProfileMatch> =
            self.inner.registry.candidates(adv).into_iter().map(|p| p.as_match(os)).collect();
        let store = self.store();
        let saved_printer_id = candidates
            .iter()
            .find_map(|c| store.find_printer_by_backend(&adv.backend_id, &c.profile_id).ok().flatten().map(|p| p.id));
        DiscoveredPrinter {
            discovery_id: adv.backend_id.clone(),
            name: adv.local_name.clone(),
            address: adv.address.clone(),
            rssi: adv.rssi,
            advertised_services: adv.services.iter().map(|u| u.to_string()).collect(),
            candidates,
            saved_printer_id,
        }
    }

    pub async fn start_scan(&self, request: StartScanRequest) -> AppResult<ScanStarted> {
        let adapter = self.adapter_status().await;
        match adapter {
            AdapterStatus::PoweredOff => return Err(nuprint_transport::TransportError::AdapterOff.into()),
            AdapterStatus::Absent => return Err(nuprint_transport::TransportError::AdapterAbsent.into()),
            AdapterStatus::PermissionDenied => return Err(nuprint_transport::TransportError::PermissionDenied.into()),
            // The scan itself waits for the answer to the macOS permission prompt.
            AdapterStatus::Ready
            | AdapterStatus::PermissionNotAsked
            | AdapterStatus::PermissionPending
            | AdapterStatus::Unknown => {}
        }
        let duration = Duration::from_secs(u64::from(request.duration_s.unwrap_or(8).clamp(1, 30)));
        let include_unknown = request.include_unknown.unwrap_or(false);
        let scan_id = Uuid::new_v4().to_string();
        let cancel = self.inner.shutdown.child_token();
        self.inner.scans.lock().unwrap().insert(scan_id.clone(), cancel.clone());
        let service = self.clone();
        let id = scan_id.clone();
        tokio::spawn(async move {
            let (tx, mut rx) = tokio::sync::mpsc::channel(128);
            let transport = service.inner.transport.clone();
            let scan_cancel = cancel.clone();
            let scan = tokio::spawn(async move { transport.scan(duration, tx, scan_cancel).await });
            let mut seen: HashMap<String, (Option<String>, Option<i16>)> = HashMap::new();
            let mut found = 0u32;
            while let Some(adv) = rx.recv().await {
                let key = (adv.local_name.clone(), adv.rssi);
                let previous = seen.get(&adv.backend_id).cloned();
                let changed = match &previous {
                    None => true,
                    Some((name, rssi)) => *name != key.0 || rssi.zip(key.1).is_some_and(|(a, b)| (a - b).abs() >= 5),
                };
                if !changed {
                    continue;
                }
                seen.insert(adv.backend_id.clone(), key);
                let device = service.discovered_printer(&adv);
                if device.candidates.is_empty() && !include_unknown {
                    continue;
                }
                service.inner.discovered.lock().unwrap().insert(adv.backend_id.clone(), adv);
                found += u32::from(previous.is_none());
                service.emit(AppEvent::ScanDevice { scan_id: id.clone(), device });
            }
            let error = match scan.await {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(AppError::from(e)),
                Err(e) => Some(AppError::internal(e)),
            };
            service.inner.scans.lock().unwrap().remove(&id);
            service.log(LogLevel::Info, format!("Scan finished: {found} device(s)"));
            service.emit(AppEvent::ScanFinished { scan_id: id, found, error });
        });
        Ok(ScanStarted { scan_id, adapter })
    }

    pub fn stop_scan(&self, scan_id: &str) {
        if let Some(token) = self.inner.scans.lock().unwrap().get(scan_id) {
            token.cancel();
        }
    }

    /// Scan for one device by exact name, address or backend ID (CLI targeting).
    pub async fn find_device(&self, target: &str, timeout: Duration) -> AppResult<DiscoveredPrinter> {
        let cancel = self.inner.shutdown.child_token();
        let adv = locate(self.inner.transport.as_ref(), timeout, &cancel, |a| {
            a.backend_id.eq_ignore_ascii_case(target)
                || a.address.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(target))
                || a.local_name.as_deref() == Some(target)
        })
        .await?;
        self.inner.discovered.lock().unwrap().insert(adv.backend_id.clone(), adv.clone());
        Ok(self.discovered_printer(&adv))
    }

    /// First supported device seen during a scan (CLI default target).
    pub async fn find_first_supported(&self, timeout: Duration) -> AppResult<DiscoveredPrinter> {
        let cancel = self.inner.shutdown.child_token();
        let registry = &self.inner.registry;
        let adv =
            locate(self.inner.transport.as_ref(), timeout, &cancel, |a| !registry.candidates(a).is_empty()).await?;
        self.inner.discovered.lock().unwrap().insert(adv.backend_id.clone(), adv.clone());
        Ok(self.discovered_printer(&adv))
    }

    fn discovered(&self, discovery_id: &str) -> AppResult<Advertisement> {
        self.inner.discovered.lock().unwrap().get(discovery_id).cloned().ok_or_else(|| {
            AppError::new(ErrorCode::DeviceNotFound, "That device is no longer in the scan results. Scan again.")
        })
    }

    /// Take the Bluetooth lock for a user action. Only an inspection is waited for, because it is
    /// short and bounded; behind a print or a check the action is refused as busy instead.
    async fn claim_hardware(&self, purpose: HardwareUse, cancel: &CancellationToken) -> AppResult<HardwareClaim<'_>> {
        let guard = match self.inner.hardware.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                let holder = *self.inner.hardware_use.lock().unwrap();
                if holder != Some(HardwareUse::Inspecting) {
                    return Err(busy(holder));
                }
                tokio::select! {
                    guard = self.inner.hardware.lock() => guard,
                    _ = cancel.cancelled() => return Err(AppError::cancelled()),
                }
            }
        };
        Ok(self.hold_hardware(guard, purpose))
    }

    fn hold_hardware<'a>(&'a self, guard: tokio::sync::MutexGuard<'a, ()>, purpose: HardwareUse) -> HardwareClaim<'a> {
        *self.inner.hardware_use.lock().unwrap() = Some(purpose);
        HardwareClaim { _guard: guard, holder: &self.inner.hardware_use }
    }

    /// Connect to a previously seen device, rescanning for it if the OS lost track of it.
    async fn connect_session(
        &self,
        resolved: &ResolvedProfile,
        backend_id: &str,
        cancel: &CancellationToken,
    ) -> AppResult<PrinterSession> {
        let timeouts = &resolved.profile.transport.timeouts;
        let adv = locate(self.inner.transport.as_ref(), Duration::from_millis(timeouts.find_ms), cancel, |a| {
            a.backend_id == backend_id
        })
        .await?;
        // Revalidate identity: a device under this ID must still match the profile's name rules.
        // The message names neither the device nor an edited profile, because errors are logged
        // and kept with jobs.
        if let Some(name) = &adv.local_name
            && !resolved.profile.matches_name(name)
        {
            return Err(AppError::new(
                ErrorCode::UnsupportedProfile,
                "The device now advertises a different name, which does not match this printer's profile.",
            ));
        }
        self.inner.discovered.lock().unwrap().insert(adv.backend_id.clone(), adv);
        PrinterSession::open(self.inner.transport.as_ref(), resolved, backend_id, self.logger(), cancel).await
    }

    /// Read-only GATT inspection; no vendor protocol bytes are written.
    /// [`Self::cancel_inspections`] stops it when nobody is waiting for the result any more.
    pub async fn inspect_device(&self, discovery_id: &str) -> AppResult<DeviceInspection> {
        let id = Uuid::new_v4().to_string();
        let cancel = self.inner.shutdown.child_token();
        self.inner.inspections.lock().unwrap().insert(id.clone(), cancel.clone());
        let result = self.inspect(discovery_id, &cancel).await;
        self.inner.inspections.lock().unwrap().remove(&id);
        result
    }

    /// Cancel every running inspection; each releases Bluetooth once it has disconnected.
    pub fn cancel_inspections(&self) {
        for token in self.inner.inspections.lock().unwrap().values() {
            token.cancel();
        }
    }

    async fn inspect(&self, discovery_id: &str, cancel: &CancellationToken) -> AppResult<DeviceInspection> {
        let adv = self.discovered(discovery_id)?;
        let _hw = self.claim_hardware(HardwareUse::Inspecting, cancel).await?;
        let timeout = Duration::from_secs(25);
        let failed = |e: AppError| {
            if e.code != ErrorCode::Cancelled {
                self.log(LogLevel::Warn, format!("Inspection failed: {}", e.message));
            }
            e
        };
        // Find the device again first, as a print does: the OS can lose track of a device after
        // a disconnect (seen on macOS), leaving the handle from an earlier scan unable to connect.
        locate(self.inner.transport.as_ref(), Duration::from_secs(12), cancel, |a| a.backend_id == discovery_id)
            .await
            .map_err(failed)?;
        // Awaited to the end: on cancellation the transport disconnects before it returns.
        let link = self.inner.transport.connect(discovery_id, timeout, cancel).await.map_err(|e| failed(e.into()))?;
        let services = link.services().to_vec();
        let max_write_len = link.max_write_len(nuprint_transport::WriteMode::WithoutResponse) as u32;
        let _ = tokio::time::timeout(Duration::from_secs(10), link.disconnect()).await;
        let os = self.inner.registry.os();
        let mut compatible = Vec::new();
        let mut rejected = Vec::new();
        for profile in self.inner.registry.candidates(&adv) {
            match profile.profile.bind(&services) {
                Ok(_)
                    if profile.profile.notify_endpoints(&services).len()
                        >= profile.profile.transport.min_notify_subscriptions.max(1) =>
                {
                    compatible.push(profile.as_match(os))
                }
                Ok(_) => {
                    rejected.push(format!("{}: required status notifications missing", profile.profile.display_name))
                }
                Err(why) => rejected.push(format!("{}: {why}", profile.profile.display_name)),
            }
        }
        self.log(
            LogLevel::Info,
            format!(
                "Inspected {}: {} service(s)",
                private(adv.local_name.as_deref().unwrap_or("device")),
                services.len()
            ),
        );
        Ok(DeviceInspection {
            discovery_id: discovery_id.to_owned(),
            name: adv.local_name,
            services: gatt_services(&services),
            compatible,
            rejected,
            max_write_len,
        })
    }

    // ------------------------------------------------------------ printers

    fn saved_printer(&self, row: &PrinterRow) -> SavedPrinter {
        let os = self.inner.registry.os();
        let resolved = self.inner.registry.get_revision(&row.profile_id, row.profile_revision);
        let resolved = resolved.as_ref();
        SavedPrinter {
            id: row.id.clone(),
            name: row.name.clone(),
            backend_id: row.backend_id.clone(),
            address: row.address.clone(),
            profile_id: row.profile_id.clone(),
            profile_revision: row.profile_revision,
            profile_name: resolved.map_or_else(|| row.profile_id.clone(), |p| p.profile.display_name.clone()),
            support: resolved.map_or(SupportLevel::Candidate, |p| p.profile.support_on(os).0),
            firmware: row.firmware.clone(),
            last_seen_ms: row.last_seen_ms.map(|v| v as f64),
            settings: PrinterSettings { density: row.density, feed_mm: row.feed_mm },
            profile_available: resolved.is_some(),
            customized: row.base_profile_id.is_some(),
        }
    }

    pub fn list_printers(&self) -> AppResult<Vec<SavedPrinter>> {
        let rows = self.store().list_printers()?;
        Ok(rows.iter().map(|r| self.saved_printer(r)).collect())
    }

    fn printer_row(&self, printer_id: &str) -> AppResult<PrinterRow> {
        self.store().get_printer(printer_id)?.ok_or_else(|| AppError::not_found("Printer"))
    }

    pub fn save_printer(&self, request: SavePrinterRequest) -> AppResult<SavedPrinter> {
        let adv = self.discovered(&request.discovery_id)?;
        let os = self.inner.registry.os();
        let candidates = self.inner.registry.candidates(&adv);
        // A manual choice cannot bypass name/identity rules.
        let resolved = candidates.into_iter().find(|p| p.profile.id == request.profile_id).ok_or_else(|| {
            AppError::new(ErrorCode::UnsupportedProfile, "This device does not match the selected printer profile.")
        })?;
        let (support, note) = resolved.profile.support_on(os);
        match support {
            SupportLevel::Candidate => {
                return Err(AppError::new(ErrorCode::UnsupportedProfile, "Printing to this model is not supported."));
            }
            SupportLevel::Experimental if !request.acknowledge_experimental => {
                return Err(AppError::new(
                    ErrorCode::ExperimentalNotAcknowledged,
                    "This printer is experimental on this computer. Confirm to use it anyway.",
                )
                .with_detail(note));
            }
            _ => {}
        }
        let store = self.store();
        let name = adv.local_name.clone().unwrap_or_else(|| resolved.profile.display_name.clone());
        let row = match store.find_printer_by_backend(&adv.backend_id, &resolved.profile.id)? {
            Some(mut existing) => {
                existing.name = name;
                existing.address = adv.address.clone();
                // Saving again must not discard settings the user edited.
                if existing.base_profile_id.is_none() {
                    existing.profile_revision = resolved.profile.revision;
                }
                existing.last_seen_ms = Some(now_ms());
                existing
            }
            None => PrinterRow {
                id: Uuid::new_v4().to_string(),
                name,
                backend_kind: "ble_gatt".into(),
                backend_id: adv.backend_id.clone(),
                address: adv.address.clone(),
                profile_id: resolved.profile.id.clone(),
                profile_revision: resolved.profile.revision,
                base_profile_id: None,
                firmware: None,
                last_seen_ms: Some(now_ms()),
                density: resolved.profile.default_density,
                feed_mm: resolved.driver.feed().default_mm,
                created_ms: now_ms(),
            },
        };
        store.upsert_printer(&row)?;
        if store.get_pref::<String>("selected_printer")?.is_none() {
            store.set_pref("selected_printer", &row.id)?;
        }
        drop(store);
        self.log(LogLevel::Info, format!("Saved printer {} ({})", private(&row.name), resolved.profile.display_name));
        self.emit(AppEvent::PrintersChanged { seq: self.next_seq() });
        Ok(self.saved_printer(&row))
    }

    pub fn remove_printer(&self, printer_id: &str) -> AppResult<()> {
        let store = self.store();
        if !store.delete_printer(printer_id)? {
            return Err(AppError::not_found("Printer"));
        }
        if store.get_pref::<String>("selected_printer")?.as_deref() == Some(printer_id) {
            let next = store.list_printers()?.first().map(|p| p.id.clone());
            store.set_pref("selected_printer", &next)?;
        }
        drop(store);
        self.emit(AppEvent::PrintersChanged { seq: self.next_seq() });
        Ok(())
    }

    fn printer_profile(&self, row: &PrinterRow) -> AppResult<ResolvedProfile> {
        self.inner.registry.get_revision(&row.profile_id, row.profile_revision).ok_or_else(|| {
            AppError::new(ErrorCode::UnsupportedProfile, "This printer's profile is unavailable in this version.")
        })
    }

    /// The built-in profile a saved printer uses, or started from.
    fn base_profile(&self, row: &PrinterRow) -> AppResult<ResolvedProfile> {
        let base_id = row.base_profile_id.as_deref().unwrap_or(&row.profile_id);
        self.inner.registry.get(base_id).ok_or_else(|| {
            AppError::new(
                ErrorCode::UnsupportedProfile,
                "This printer's built-in profile is unavailable in this version.",
            )
        })
    }

    /// Keep a printer's darkness and feed valid after its profile changed.
    fn fit_settings(row: &mut PrinterRow, profile: &ResolvedProfile) {
        if !profile.driver.densities().contains(&row.density) {
            row.density = profile.profile.default_density;
        }
        let feed = profile.driver.feed();
        if !feed.contains(row.feed_mm) {
            row.feed_mm = feed.default_mm;
        }
    }

    /// The settings a user may edit for this printer, with the built-in ones for comparison.
    pub fn printer_profile_doc(&self, printer_id: &str) -> AppResult<PrinterProfileDoc> {
        let row = self.printer_row(printer_id)?;
        let current = self.printer_profile(&row)?;
        let base = self.base_profile(&row)?;
        let pretty = |p: &ModelProfile| serde_json::to_string_pretty(&p.tunable_doc()).map_err(AppError::internal);
        Ok(PrinterProfileDoc {
            printer_id: row.id.clone(),
            base_profile_id: base.profile.id.clone(),
            base_display_name: base.profile.display_name.clone(),
            customized: row.base_profile_id.is_some(),
            base_json: pretty(&base.profile)?,
            current_json: pretty(&current.profile)?,
        })
    }

    /// Store edited settings for one printer as a new revision of its own custom profile.
    /// Settings identical to the built-in ones go back to the built-in profile instead.
    /// Jobs already queued keep the revision they were rendered for.
    pub fn save_printer_profile(&self, request: SavePrinterProfileRequest) -> AppResult<SavedPrinter> {
        let doc: serde_json::Value = serde_json::from_str(&request.json)
            .map_err(|e| AppError::invalid(format!("The settings are not valid JSON: {e}")))?;
        // One store lock from choosing the revision to publishing it: two saves cannot pick the
        // same revision, and the registry only ever offers a revision that was stored.
        let mut store = self.store();
        let mut row = store.get_printer(&request.printer_id)?.ok_or_else(|| AppError::not_found("Printer"))?;
        let base = self.base_profile(&row)?;
        let custom_id = format!("{CUSTOM_PROFILE_PREFIX}{}", row.id);
        let revision =
            self.inner.registry.next_custom_revision(&custom_id).max(store.max_custom_revision(&custom_id)? + 1);
        let profile = ModelProfile::customized(&base.profile, &doc, custom_id, revision)?;
        if profile.tunable_doc() == base.profile.tunable_doc() {
            drop(store);
            return self.reset_printer_profile(&request.printer_id);
        }
        let json = serde_json::to_string(&profile).map_err(AppError::internal)?;
        let resolved = self.inner.registry.prepare_custom(profile)?;
        row.profile_id = resolved.profile.id.clone();
        row.profile_revision = revision;
        row.base_profile_id = Some(base.profile.id.clone());
        Self::fit_settings(&mut row, &resolved);
        store.save_custom_profile(
            &CustomProfileRow { id: resolved.profile.id.clone(), revision, json },
            &base.profile.id,
            &row,
        )?;
        self.inner.registry.publish_custom(resolved)?;
        drop(store);
        self.log(
            LogLevel::Info,
            format!("Saved edited settings for printer {} (revision {revision})", private(&row.name)),
        );
        self.emit(AppEvent::PrintersChanged { seq: self.next_seq() });
        Ok(self.saved_printer(&row))
    }

    /// Go back to the built-in settings. Edited revisions stay stored for jobs that name them.
    pub fn reset_printer_profile(&self, printer_id: &str) -> AppResult<SavedPrinter> {
        let store = self.store();
        let mut row = store.get_printer(printer_id)?.ok_or_else(|| AppError::not_found("Printer"))?;
        if row.base_profile_id.is_none() {
            drop(store);
            return Ok(self.saved_printer(&row));
        }
        let base = self.base_profile(&row)?;
        row.profile_id = base.profile.id.clone();
        row.profile_revision = base.profile.revision;
        row.base_profile_id = None;
        Self::fit_settings(&mut row, &base);
        store.upsert_printer(&row)?;
        drop(store);
        self.log(LogLevel::Info, format!("Printer {} uses the built-in settings again", private(&row.name)));
        self.emit(AppEvent::PrintersChanged { seq: self.next_seq() });
        Ok(self.saved_printer(&row))
    }

    /// The printer's settings as a complete profile file, for sharing. An edited profile gets a
    /// portable ID instead of this computer's printer ID.
    pub fn export_printer_profile(&self, printer_id: &str) -> AppResult<String> {
        let row = self.printer_row(printer_id)?;
        let mut profile = (*self.printer_profile(&row)?.profile).clone();
        if let Some(base) = &profile.derived_from {
            profile.id = format!("{base}-custom");
            profile.revision = 1;
        }
        serde_json::to_string_pretty(&profile).map_err(AppError::internal)
    }

    /// Write [`Self::export_printer_profile`] to a file the user chose.
    pub fn save_printer_profile_file(&self, printer_id: &str, path: &Path) -> AppResult<()> {
        fs::write(path, self.export_printer_profile(printer_id)?)?;
        Ok(())
    }

    /// Reject darkness, extra feed or copies the profile does not accept, before any work.
    pub fn check_job_options(&self, profile_id: &str, density: Density, feed_mm: u16, copies: u16) -> AppResult<()> {
        let resolved = self.inner.registry.get(profile_id).ok_or_else(|| AppError::not_found("Printer profile"))?;
        resolved.check_job_options(density, feed_mm, Some(copies))
    }

    pub fn update_printer_settings(&self, printer_id: &str, settings: PrinterSettings) -> AppResult<SavedPrinter> {
        // Read, check and write under one store lock, so a profile saved meanwhile is not undone.
        let store = self.store();
        let row = store.get_printer(printer_id)?.ok_or_else(|| AppError::not_found("Printer"))?;
        self.printer_profile(&row)?.check_job_options(settings.density, settings.feed_mm, None)?;
        if !store.update_printer_settings(&row.id, settings.density, settings.feed_mm)? {
            return Err(AppError::not_found("Printer"));
        }
        drop(store);
        let row = PrinterRow { density: settings.density, feed_mm: settings.feed_mm, ..row };
        self.emit(AppEvent::PrintersChanged { seq: self.next_seq() });
        Ok(self.saved_printer(&row))
    }

    pub fn selected_printer(&self) -> AppResult<Option<String>> {
        Ok(self.store().get_pref::<Option<String>>("selected_printer")?.flatten())
    }

    pub fn select_printer(&self, printer_id: Option<String>) -> AppResult<()> {
        if let Some(id) = &printer_id {
            self.printer_row(id)?;
        }
        self.store().set_pref("selected_printer", &printer_id)?;
        self.emit(AppEvent::PrintersChanged { seq: self.next_seq() });
        Ok(())
    }

    /// Connect and query status without printing.
    pub async fn check_printer(&self, printer_id: &str) -> AppResult<PrinterCheck> {
        let row = self.printer_row(printer_id)?;
        let resolved = self.printer_profile(&row)?.clone();
        let cancel = self.inner.shutdown.child_token();
        let _hw = self.claim_hardware(HardwareUse::Checking, &cancel).await?;
        self.log(LogLevel::Info, format!("Checking {}…", private(&row.name)));
        let failed = |e: AppError| {
            self.log(LogLevel::Warn, format!("Connection check failed: {}", e.message));
            e
        };
        let mut session = self.connect_session(&resolved, &row.backend_id, &cancel).await.map_err(failed)?;
        let result = session.diagnose(&cancel).await;
        let services = gatt_services(session.services());
        let firmware = session.device_info().and_then(|i| i.firmware);
        let notifications = session.raw_notifications();
        let max_write_len = session.max_write_len() as u32;
        session.close().await;
        let status = result.map_err(failed)?;
        // `row` is as it was before the wait; only the check's own findings are written back.
        self.store().record_printer_seen(&row.id, firmware.as_deref(), now_ms())?;
        let check = PrinterCheck {
            printer_id: row.id.clone(),
            ready: status.is_ready(),
            status_text: status.text(),
            flags: status.flags.clone(),
            raw_status_hex: hex::encode(&status.raw),
            firmware,
            max_write_len,
            services,
            notifications,
            checked_ms: now_ms() as f64,
        };
        self.log(LogLevel::Info, format!("Connection verified · {}", check.status_text));
        *self.inner.last_check.lock().unwrap() = Some(check.clone());
        self.emit(AppEvent::PrintersChanged { seq: self.next_seq() });
        Ok(check)
    }

    // ------------------------------------------------------------ sources & previews

    /// Copy a user-selected file into app-managed memory. Only the file name is kept.
    pub async fn import_source(&self, path: &Path) -> AppResult<SourceInfo> {
        let meta = fs::metadata(path)
            .map_err(|e| AppError::invalid("That file could not be opened.").with_detail(e.to_string()))?;
        if !meta.is_file() {
            return Err(AppError::invalid("Choose a file, not a folder."));
        }
        if meta.len() > nuprint_render::MAX_SOURCE_BYTES {
            return Err(
                RenderError::SourceTooLarge { bytes: meta.len(), limit: nuprint_render::MAX_SOURCE_BYTES }.into()
            );
        }
        let bytes = Arc::new(fs::read(path)?);
        let format = nuprint_render::sniff_format(&bytes).ok_or(RenderError::UnsupportedFormat)?;
        let name = path.file_name().map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
        let (kind, page_count) = if format == SourceFormat::Pdf {
            let engine = self.pdf_engine()?;
            let data = bytes.clone();
            let count = tokio::task::spawn_blocking(move || engine.page_count(&data, None))
                .await
                .map_err(AppError::internal)?;
            match count {
                Ok(count) => (SourceKind::Pdf, Some(count)),
                Err(RenderError::PdfPasswordRequired) => (SourceKind::Pdf, None),
                Err(e) => return Err(e.into()),
            }
        } else {
            (SourceKind::Image, None)
        };
        let info =
            SourceInfo { source_id: Uuid::new_v4().to_string(), kind, name, bytes: meta.len() as f64, page_count };
        self.inner.sources.insert(info.source_id.clone(), Source { format, bytes });
        Ok(info)
    }

    fn source(&self, source_id: &str) -> AppResult<Arc<Source>> {
        self.inner
            .sources
            .get(source_id)
            .ok_or_else(|| AppError::not_found("The opened file").with_detail("open it again"))
    }

    pub async fn render_preview(&self, request: PreviewRequest) -> AppResult<PreviewResult> {
        let stale = || AppError::new(ErrorCode::StalePreview, "A newer preview was requested");
        {
            let mut latest = self.inner.latest_generation.lock().unwrap();
            if request.generation < *latest {
                return Err(stale());
            }
            *latest = request.generation;
        }
        let resolved =
            self.inner.registry.get(&request.profile_id).ok_or_else(|| AppError::not_found("Printer profile"))?;
        let raster = &resolved.profile.raster;
        let target = RenderTarget {
            width_dots: raster.width_dots,
            dots_per_mm: raster.dots_per_mm,
            max_height: raster.max_height,
        };
        let source_kind = source_kind_name(&request.source);
        let settings_json = persisted_settings(&request.source)?;
        let spec = request.source.clone();
        let pages: Vec<(u32, RenderOutput)> = match spec {
            SourceSpec::Text(text) => {
                let params = text_params(&text);
                vec![(1, self.blocking(move || nuprint_render::render_text(&params, &target)).await?)]
            }
            SourceSpec::TestPattern => {
                vec![(1, self.blocking(move || Ok(nuprint_render::test_pattern(&target))).await?)]
            }
            SourceSpec::Image { source_id, adjust } => {
                let source = self.source(&source_id)?;
                if source.format == SourceFormat::Pdf {
                    return Err(AppError::invalid("This file is a PDF; choose a PDF page instead."));
                }
                let params = image_params(&adjust);
                vec![(1, self.blocking(move || nuprint_render::render_image(&source.bytes, &params, &target)).await?)]
            }
            SourceSpec::Pdf { source_id, page, adjust, password } => {
                let source = self.source(&source_id)?;
                if source.format != SourceFormat::Pdf {
                    return Err(AppError::invalid("This file is not a PDF."));
                }
                let engine = self.pdf_engine()?;
                let params = image_params(&adjust);
                let output = self
                    .blocking(move || engine.render_page(&source.bytes, password.as_deref(), page, &params, &target))
                    .await?;
                vec![(page, output)]
            }
        };
        if *self.inner.latest_generation.lock().unwrap() > request.generation {
            return Err(stale());
        }
        let (source_page, output) = &pages[0];
        let bitmap = output.bitmap.clone();
        let png = Arc::new(nuprint_render::encode_png(&bitmap));
        let warnings = pages.iter().flat_map(|(_, o)| o.warnings.iter().map(warning_text)).collect();
        let total = f64::from(bitmap.width()) * f64::from(bitmap.height());
        let snapshot_id = Uuid::new_v4().to_string();
        let result = PreviewResult {
            generation: request.generation,
            snapshot_id: snapshot_id.clone(),
            profile_id: resolved.profile.id.clone(),
            profile_revision: resolved.profile.revision,
            width: bitmap.width(),
            height: bitmap.height(),
            length_mm: bitmap.height() as f32 / target.dots_per_mm,
            sha256: bitmap.sha256_hex(),
            black_ratio: (bitmap.black_pixels() as f64 / total) as f32,
            warnings,
            preview_path: format!("{snapshot_id}.png"),
        };
        let snapshot = Snapshot {
            profile_id: resolved.profile.id.clone(),
            profile_revision: resolved.profile.revision,
            source_kind,
            settings_json,
            pages: vec![(*source_page, bitmap)],
            png,
        };
        self.inner.snapshots.insert(snapshot_id, snapshot);
        Ok(result)
    }

    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, RenderError> + Send + 'static,
    ) -> AppResult<T> {
        tokio::task::spawn_blocking(work).await.map_err(AppError::internal)?.map_err(AppError::from)
    }

    fn snapshot(&self, snapshot_id: &str) -> AppResult<Arc<Snapshot>> {
        self.inner
            .snapshots
            .get(snapshot_id)
            .ok_or_else(|| AppError::new(ErrorCode::StalePreview, "This preview has expired. Wait for it to refresh."))
    }

    /// Fails with `StalePreview` once the snapshot has been evicted.
    pub fn check_preview(&self, snapshot_id: &str) -> AppResult<()> {
        self.snapshot(snapshot_id).map(drop)
    }

    /// PNG of the canonical bitmap: exactly what would be transmitted.
    pub fn preview_png(&self, snapshot_id: &str) -> Option<Arc<Vec<u8>>> {
        self.snapshot(snapshot_id).ok().map(|s| s.png.clone())
    }

    /// A job's first page as a PNG, for the queue: the saved page while the job keeps its print
    /// data, otherwise the preview kept when it finished. `None` for an unknown job or one
    /// whose data was removed without a preview.
    pub fn job_page_png(&self, job_id: &str) -> Option<Vec<u8>> {
        let bitmap = {
            let store = self.store();
            let job = store.get_job(job_id).ok()??;
            if !job.content_retained {
                return store.thumbnail(job_id);
            }
            let page = store.pages(job_id).ok()?.into_iter().next()?;
            store.load_page(&page).ok()?
        };
        Some(nuprint_render::encode_png(&bitmap))
    }

    pub fn save_preview_png(&self, snapshot_id: &str, path: &Path) -> AppResult<()> {
        let snapshot = self.snapshot(snapshot_id)?;
        fs::write(path, snapshot.png.as_slice())?;
        Ok(())
    }

    // ------------------------------------------------------------ jobs

    fn job_summary(&self, job: &JobRow) -> AppResult<JobSummary> {
        let pages = self.store().pages(&job.id)?;
        Ok(summarize(job, &pages))
    }

    fn emit_job(&self, job: &JobRow) {
        if let Ok(summary) = self.job_summary(job) {
            let queue_held = self.inner.held.load(Ordering::Relaxed);
            self.emit(AppEvent::JobChanged { seq: self.next_seq(), job: summary, queue_held });
        }
    }

    /// One plan per copy and page in print order, as (copy, page index, plan). The extra
    /// feed is added once, after the last page of the last copy.
    fn plan_pages(
        driver: &dyn PrinterDriver,
        job: &JobRow,
        pages: &[(u32, MonoBitmap)],
    ) -> AppResult<Vec<(u16, usize, ExchangePlan)>> {
        let mut plans = Vec::with_capacity(usize::from(job.copies) * pages.len());
        for copy in 0..job.copies {
            for (index, (_, bitmap)) in pages.iter().enumerate() {
                let last = copy + 1 == job.copies && index + 1 == pages.len();
                let request =
                    PrintRequest { bitmap, density: job.density, trailing_feed_mm: if last { job.feed_mm } else { 0 } };
                plans.push((copy, index, driver.plan_job(&request)?));
            }
        }
        Ok(plans)
    }

    /// Atomically admit a job for the displayed raster snapshot.
    pub fn enqueue(&self, request: EnqueueRequest) -> AppResult<JobSummary> {
        if request.idempotency_key.trim().is_empty() || request.idempotency_key.len() > 128 {
            return Err(AppError::invalid("A submission key is required."));
        }
        let existing = self.store().find_job_by_key(&request.idempotency_key)?;
        if let Some(existing) = existing {
            return self.job_summary(&existing);
        }
        let snapshot = self.snapshot(&request.snapshot_id)?;
        let printer = self.printer_row(&request.printer_id)?;
        let resolved = self.printer_profile(&printer)?;
        if snapshot.profile_id != printer.profile_id || snapshot.profile_revision != printer.profile_revision {
            return Err(AppError::new(
                ErrorCode::StalePreview,
                "The preview was made for a different printer model. Wait for it to refresh, then print.",
            ));
        }
        resolved.check_job_options(request.density, request.feed_mm, Some(request.copies))?;
        let title: String = request.title.trim().chars().filter(|c| !c.is_control()).take(120).collect();
        let now = now_ms();
        let mut job = JobRow {
            id: Uuid::new_v4().to_string(),
            idempotency_key: request.idempotency_key,
            title: if title.is_empty() { "Untitled".into() } else { title },
            source_kind: snapshot.source_kind.into(),
            printer_id: printer.id.clone(),
            printer_name: printer.name.clone(),
            backend_id: printer.backend_id.clone(),
            profile_id: resolved.profile.id.clone(),
            profile_revision: resolved.profile.revision,
            protocol_id: resolved.driver.protocol_id().into(),
            protocol_version: resolved.driver.protocol_version(),
            state: JobState::Queued,
            created_ms: now,
            updated_ms: now,
            copies: request.copies,
            density: request.density,
            feed_mm: request.feed_mm,
            dots_per_mm: resolved.profile.raster.dots_per_mm,
            renderer_version: RENDERER_VERSION.into(),
            settings_json: snapshot.settings_json.clone(),
            content_retained: true,
            bytes_total: 0,
            bytes_sent: 0,
            error: None,
            partial_output: false,
            retry_of: None,
            revision: 0,
        };
        let plans = Self::plan_pages(resolved.driver.as_ref(), &job, &snapshot.pages)?;
        job.bytes_total = plans.iter().map(|(_, _, plan)| plan.print_bytes() as i64).sum();
        self.admit(job, snapshot.pages.clone())
    }

    fn admit(&self, mut job: JobRow, pages: Vec<(u32, MonoBitmap)>) -> AppResult<JobSummary> {
        let mut store = self.store();
        if store.count_jobs_with_content()? >= MAX_QUEUE_JOBS {
            return Err(AppError::new(
                ErrorCode::QueueFull,
                "The queue is full. Remove finished jobs or clear history.",
            ));
        }
        let new_bytes: i64 = pages.iter().map(|(_, b)| b.data().len() as i64).sum();
        if store.retained_content_bytes()? + new_bytes > MAX_RETAINED_BYTES {
            return Err(AppError::new(
                ErrorCode::QueueFull,
                "Saved print data is at its size limit. Clear history first.",
            ));
        }
        job.revision = store.next_revision();
        let driver = self.inner.registry.get_revision(&job.profile_id, job.profile_revision).map(|p| p.driver.clone());
        let per_page: Vec<i64> = pages
            .iter()
            .map(|(_, bitmap)| {
                let request = PrintRequest { bitmap, density: job.density, trailing_feed_mm: 0 };
                driver.as_ref().and_then(|d| d.plan_job(&request).ok()).map_or(0, |p| p.print_bytes() as i64)
            })
            .collect();
        store.insert_job(&job, &pages, &per_page)?;
        drop(store);
        self.log(
            LogLevel::Info,
            format!(
                "Queued \"{}\" for {} ({} cop{})",
                private(&job.title),
                private(&job.printer_name),
                job.copies,
                if job.copies == 1 { "y" } else { "ies" }
            ),
        );
        self.emit_job(&job);
        self.inner.wake.notify_one();
        self.job_summary(&job)
    }

    pub fn list_jobs(&self) -> AppResult<QueueSnapshot> {
        let store = self.store();
        let rows = store.list_jobs(200)?;
        let mut jobs = Vec::with_capacity(rows.len());
        for row in &rows {
            jobs.push(summarize(row, &store.pages(&row.id)?));
        }
        let seq = self.inner.seq.load(Ordering::SeqCst) as f64;
        Ok(QueueSnapshot {
            held: self.inner.held.load(Ordering::Relaxed),
            revision: store.revision() as f64,
            seq,
            jobs,
        })
    }

    pub fn get_job(&self, job_id: &str) -> AppResult<JobSummary> {
        let job = self.store().get_job(job_id)?.ok_or_else(|| AppError::not_found("Job"))?;
        self.job_summary(&job)
    }

    /// Acknowledge cancellation intent. An active job reaches its terminal state after
    /// the transfer task unwinds and Bluetooth cleanup completes.
    pub fn cancel_job(&self, job_id: &str) -> AppResult<()> {
        if self.cancel_active(job_id) {
            self.log(LogLevel::Info, "Cancelling transfer…".into());
            return Ok(());
        }
        let cancelled = self.store().cancel_unstarted(job_id)?;
        if let Some(job) = cancelled {
            self.emit_job(&job);
            return Ok(());
        }
        // Not queued or held any more. The dispatcher marks a job active before it claims it, so a
        // claim that just won the race is visible here.
        if self.cancel_active(job_id) {
            self.log(LogLevel::Info, "Cancelling transfer…".into());
            return Ok(());
        }
        let job = self.store().get_job(job_id)?.ok_or_else(|| AppError::not_found("Job"))?;
        if job.state.is_terminal() {
            Ok(())
        } else {
            Err(AppError::invalid("This job is finishing; it cannot be cancelled now."))
        }
    }

    pub fn retry_job(&self, job_id: &str, idempotency_key: String) -> AppResult<JobSummary> {
        let existing = self.store().find_job_by_key(&idempotency_key)?;
        if let Some(existing) = existing {
            return self.job_summary(&existing);
        }
        let (original, pages) = {
            let store = self.store();
            let original = store.get_job(job_id)?.ok_or_else(|| AppError::not_found("Job"))?;
            if !original.state.is_terminal() {
                return Err(AppError::invalid("Only finished jobs can be printed again."));
            }
            if !original.content_retained {
                return Err(AppError::invalid(
                    "This job's print data was not kept. Open the source and print it again.",
                ));
            }
            let pages = store.load_pages(job_id)?;
            (original, pages)
        };
        if self.inner.registry.get_revision(&original.profile_id, original.profile_revision).is_none() {
            return Err(AppError::new(
                ErrorCode::UnsupportedProfile,
                "This printer profile changed; print from the source again.",
            ));
        }
        let now = now_ms();
        let job = JobRow {
            id: Uuid::new_v4().to_string(),
            idempotency_key,
            state: JobState::Queued,
            created_ms: now,
            updated_ms: now,
            content_retained: true,
            bytes_sent: 0,
            error: None,
            partial_output: false,
            retry_of: Some(original.id.clone()),
            ..original
        };
        self.admit(job, pages)
    }

    pub fn remove_job(&self, job_id: &str) -> AppResult<()> {
        let mut store = self.store();
        let job = store.get_job(job_id)?.ok_or_else(|| AppError::not_found("Job"))?;
        if !job.state.is_terminal() {
            return Err(AppError::invalid("Cancel the job before removing it."));
        }
        store.delete_job(job_id)?;
        drop(store);
        self.emit(AppEvent::JobRemoved { seq: self.next_seq(), job_id: job_id.to_owned() });
        Ok(())
    }

    pub fn clear_history(&self) -> AppResult<()> {
        let mut store = self.store();
        let removed: Vec<String> =
            store.list_jobs(100_000)?.into_iter().filter(|j| j.state.is_terminal()).map(|j| j.id).collect();
        for id in &removed {
            store.delete_job(id)?;
        }
        drop(store);
        for job_id in removed {
            self.emit(AppEvent::JobRemoved { seq: self.next_seq(), job_id });
        }
        Ok(())
    }

    pub fn hold_queue(&self) -> AppResult<QueueSnapshot> {
        self.inner.held.store(true, Ordering::Relaxed);
        self.store().set_pref("queue_held", &true)?;
        self.log(LogLevel::Info, "Queue held; the current job (if any) continues".into());
        self.list_jobs()
    }

    pub fn resume_queue(&self) -> AppResult<QueueSnapshot> {
        self.inner.held.store(false, Ordering::Relaxed);
        let released = {
            let mut store = self.store();
            store.set_pref("queue_held", &false)?;
            let mut released = Vec::new();
            for mut job in store.list_jobs(100_000)?.into_iter().filter(|j| j.state == JobState::Held) {
                job.state = JobState::Queued;
                store.update_job(&mut job)?;
                released.push(job);
            }
            released
        };
        for job in &released {
            self.emit_job(job);
        }
        self.inner.wake.notify_one();
        self.list_jobs()
    }

    // ------------------------------------------------------------ dispatcher

    async fn dispatch_loop(self) {
        loop {
            while !self.inner.held.load(Ordering::Relaxed) && !self.inner.shutdown.is_cancelled() {
                let next = self.store().next_queued();
                match next {
                    Ok(Some(job)) => self.run_job(job).await,
                    Ok(None) => break,
                    Err(e) => {
                        self.log(LogLevel::Error, format!("Queue error: {e}"));
                        break;
                    }
                }
            }
            tokio::select! {
                _ = self.inner.shutdown.cancelled() => return,
                _ = self.inner.wake.notified() => {}
            }
        }
    }

    /// Record and announce a state. `false` means it could not be saved; the change is still
    /// announced so the screen shows what is happening, but the caller decides what to do.
    fn set_state(&self, job: &mut JobRow, state: JobState) -> bool {
        job.state = state;
        let saved = match self.store().update_job(job) {
            Ok(()) => true,
            Err(e) => {
                self.log(LogLevel::Error, format!("Could not save job state: {e}"));
                false
            }
        };
        self.emit_job(job);
        saved
    }

    /// Stop starting jobs, for this run only, when the queue can no longer be saved reliably.
    /// Printing on without a record could resend a job that was already tried.
    fn pause_queue_for_storage_error(&self, what: &str) {
        self.inner.held.store(true, Ordering::Relaxed);
        self.log(LogLevel::Error, format!("Queue paused: {what} could not be saved. Restart NuPrint to continue."));
    }

    fn cancel_active(&self, job_id: &str) -> bool {
        let active = self.inner.active.lock().unwrap();
        match active.as_ref() {
            Some(active) if active.job_id == job_id => {
                active.cancel.cancel();
                true
            }
            _ => false,
        }
    }

    async fn run_job(&self, queued: JobRow) {
        let cancel = self.inner.shutdown.child_token();
        // Active before claimed: a cancel that misses the queued row still finds the job here.
        *self.inner.active.lock().unwrap() = Some(ActiveJob { job_id: queued.id.clone(), cancel: cancel.clone() });
        let hardware = tokio::select! {
            guard = self.inner.hardware.lock() => Some(self.hold_hardware(guard, HardwareUse::Printing)),
            _ = cancel.cancelled() => None,
        };
        let Some(hardware) = hardware else {
            // Cancelled while waiting for Bluetooth; nothing was claimed or sent.
            let cancelled = self.store().cancel_unstarted(&queued.id);
            match cancelled {
                Ok(Some(job)) => self.emit_job(&job),
                Ok(None) => {}
                Err(e) => self.log(LogLevel::Error, format!("Could not save the cancellation: {e}")),
            }
            *self.inner.active.lock().unwrap() = None;
            return;
        };
        // Claim the job durably before any Bluetooth I/O. If it is no longer queued, or the claim
        // cannot be saved, nothing is sent.
        let attempt_id = Uuid::new_v4().to_string();
        let claimed = self.store().claim_queued(&queued.id, &attempt_id);
        let mut job = match claimed {
            Ok(Some(job)) => job,
            Ok(None) => {
                *self.inner.active.lock().unwrap() = None;
                return;
            }
            Err(e) => {
                self.log(LogLevel::Error, format!("Could not claim a job: {e}"));
                self.pause_queue_for_storage_error("the start of a job");
                *self.inner.active.lock().unwrap() = None;
                return;
            }
        };
        self.emit_job(&job);
        let (result, sent) = self.execute_job(&mut job, &attempt_id, &cancel).await;
        drop(hardware);
        // `sent` counts bytes that may have arrived, which can exceed what was confirmed written;
        // `job.bytes_sent` keeps the confirmed count.
        let (state, partial) = classify(&result, sent);
        job.partial_output = partial;
        job.error = match (&result, state) {
            (Err(_), JobState::Cancelled) if partial => Some(AppError::new(
                ErrorCode::Cancelled,
                "Cancelled. Data already in the printer may still print; check the paper before printing again.",
            )),
            (Err(_), JobState::Cancelled) => None,
            // An OutcomeUncertain error already says what to check.
            (Err(e), JobState::Uncertain) if e.code != ErrorCode::OutcomeUncertain => Some(AppError {
                code: e.code,
                message: format!("{} Some paper may already have printed; check before retrying.", e.message),
                detail: e.detail.clone(),
            }),
            (Err(e), _) => Some(e.clone()),
            (Ok(_), _) => None,
        };
        {
            let store = self.store();
            if let Err(e) = store.finish_attempt(&attempt_id, state, result.as_ref().err().map(|e| e.code)) {
                self.log(LogLevel::Error, format!("Could not record the attempt outcome: {e}"));
            }
            let _ = store.add_event(&attempt_id, "outcome", state.as_str());
        }
        // If this is lost, the job stays in its in-progress state and a restart marks it uncertain.
        if !self.set_state(&mut job, state) {
            self.pause_queue_for_storage_error("the result of a job");
        }
        if state == JobState::Completed {
            // The print data is deleted; a preview of the first page and the details stay until
            // the job is removed or its 30 days are up.
            let thumbnail = self.job_page_png(&job.id);
            let mut store = self.store();
            if let Err(e) = store.drop_content(&mut job, thumbnail.as_deref()) {
                self.log(LogLevel::Warn, format!("Could not delete completed job data: {e}"));
            }
            drop(store);
            self.emit_job(&job);
        }
        match &result {
            Ok(_) => {
                self.log(LogLevel::Info, format!("\"{}\": {}", private(&job.title), crate::jobs::state_text(&job)))
            }
            Err(e) => self.log(LogLevel::Error, format!("\"{}\": {}", private(&job.title), e.message)),
        }
        *self.inner.active.lock().unwrap() = None;
    }

    /// Returns the outcome and the number of print bytes that may have reached the printer.
    async fn execute_job(
        &self,
        job: &mut JobRow,
        attempt_id: &str,
        cancel: &CancellationToken,
    ) -> (Result<CompletionPolicy, AppError>, usize) {
        let Some(resolved) = self.inner.registry.get_revision(&job.profile_id, job.profile_revision) else {
            return (
                Err(AppError::new(
                    ErrorCode::UnsupportedProfile,
                    "This job's printer profile is not in this version; print it again from the source.",
                )),
                0,
            );
        };
        // Load, verify and encode every page before opening Bluetooth.
        let pages = self.store().load_pages(&job.id);
        let plans = match pages.and_then(|pages| Self::plan_pages(resolved.driver.as_ref(), job, &pages)) {
            Ok(plans) => plans,
            Err(e) => return (Err(e), 0),
        };
        let completion = plans.last().map_or(CompletionPolicy::StatusConfirmedReady, |(_, _, p)| p.completion);
        let total: usize = plans.iter().map(|(_, _, p)| p.print_bytes()).sum();
        job.bytes_total = total as i64;
        let mut session = match self.connect_session(&resolved, &job.backend_id, cancel).await {
            Ok(session) => session,
            Err(e) => return (Err(e), 0),
        };
        let mut base = 0usize;
        let mut last_emit = Instant::now() - PROGRESS_INTERVAL;
        let mut result = Ok(completion);
        let job_id = job.id.clone();
        for (copy, index, plan) in &plans {
            let outcome = {
                let job = &mut *job;
                let mut on_event = |event: SessionEvent| match event {
                    SessionEvent::Phase(state) => {
                        if job.state != state {
                            self.set_state(job, state);
                        }
                    }
                    SessionEvent::Progress { sent, total: _ } => {
                        let now = Instant::now();
                        let done = base + sent;
                        if now.duration_since(last_emit) >= PROGRESS_INTERVAL || done == total {
                            last_emit = now;
                            self.emit(AppEvent::JobProgress {
                                seq: self.next_seq(),
                                job_id: job_id.clone(),
                                attempt_id: attempt_id.to_owned(),
                                bytes_sent: done as f64,
                                bytes_total: total as f64,
                            });
                        }
                    }
                };
                session.execute(plan, cancel, &mut on_event).await
            };
            job.bytes_sent = session.print_bytes_submitted() as i64;
            let _ = self.store().update_attempt(
                attempt_id,
                job.state,
                *index as u32,
                u32::from(*copy),
                job.bytes_sent,
                session.last_status().map(|s| hex::encode(s.raw)).as_deref(),
            );
            base += plan.print_bytes();
            if let Err(e) = outcome {
                result = Err(e);
                break;
            }
        }
        let at_risk = session.print_bytes_at_risk();
        session.close().await;
        (result, at_risk)
    }

    // ------------------------------------------------------------ diagnostics

    pub async fn diagnostics_report(&self, include_identifiers: bool) -> AppResult<DiagnosticsReport> {
        let mut app = self.app_info();
        let mut printers = self.list_printers()?;
        let mut last_check = self.inner.last_check.lock().unwrap().clone();
        let mut recent_jobs = self.list_jobs()?.jobs;
        let mut profiles = self.profiles();
        recent_jobs.truncate(20);
        if !include_identifiers {
            app.data_dir = app.data_dir.map(|_| "<app data directory>".into());
            for (i, p) in printers.iter_mut().enumerate() {
                p.name = format!("Printer {}", i + 1);
                p.backend_id = "<redacted>".into();
                p.address = None;
                // Edited settings carry a name the owner typed.
                if p.customized {
                    p.profile_name = "<redacted>".into();
                }
            }
            for profile in &mut profiles {
                if profile.id.starts_with(CUSTOM_PROFILE_PREFIX) {
                    profile.display_name = "<redacted>".into();
                }
            }
            if let Some(check) = &mut last_check {
                check.printer_id = "<redacted>".into();
            }
            // PDFium search failures name local folders.
            app.pdf_error = app.pdf_error.map(|_| "PDFium could not be loaded (folder names removed).".into());
            for job in &mut recent_jobs {
                job.title = "<redacted>".into();
                job.printer_name = "<redacted>".into();
                // Backend details can name the device or its address; the code and message stay.
                if let Some(error) = &mut job.error {
                    error.detail = None;
                }
            }
        }
        Ok(DiagnosticsReport {
            generated_ms: now_ms() as f64,
            app,
            adapter: self.adapter_status().await,
            profiles,
            printers,
            last_check,
            recent_jobs,
            log: self.export_logs(include_identifiers),
            includes_identifiers: include_identifiers,
        })
    }

    pub async fn export_diagnostics(&self, path: &Path, include_identifiers: bool) -> AppResult<()> {
        let report = self.diagnostics_report(include_identifiers).await?;
        let json = serde_json::to_string_pretty(&report).map_err(AppError::internal)?;
        fs::write(path, json)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_job_settings_leave_out_passwords_and_typed_text() {
        let adjust = ImageAdjust { tone: Tone::Dither, rotation: Rotation::R0, brightness: 0, contrast: 0 };
        let pdf = SourceSpec::Pdf { source_id: "s".into(), page: 2, adjust, password: Some("hunter2".into()) };
        let saved = persisted_settings(&pdf).unwrap();
        assert!(!saved.contains("hunter2") && !saved.contains("password"), "{saved}");
        assert!(saved.contains(r#""page":2"#), "{saved}");

        let text = SourceSpec::Text(TextSpec {
            text: "private note".into(),
            size_px: 24,
            family: FontFamily::Sans,
            bold: false,
            align: TextAlign::Left,
            line_spacing: 1.0,
            margin_dots: 8,
            headings: false,
        });
        let saved = persisted_settings(&text).unwrap();
        assert!(!saved.contains("private note"), "{saved}");
        assert!(saved.contains("size_px"), "{saved}");
    }
}
