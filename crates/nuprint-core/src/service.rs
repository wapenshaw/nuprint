//! The application service: one instance owns the queue, dispatcher, storage and all
//! Bluetooth activity for this process. The Tauri host and the CLI both drive it.

mod diagnostics;
mod dispatch;
mod lifecycle;
mod logging;
mod preview;
mod printers;
mod queue;

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use nuprint_render::PdfEngine;
use nuprint_transport::{Advertisement, Transport};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::api::*;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::log_writer::LogWriter;
use crate::profile::{ModelProfile, ProfileRegistry};
use crate::render_queue::RenderQueue;
use crate::store::{Store, now_ms};
use dispatch::{ActiveJob, HardwareUse};
use preview::{Snapshot, Source};

pub const MAX_QUEUE_JOBS: i64 = 100;
pub const MAX_RETAINED_BYTES: i64 = 256 * 1024 * 1024;
/// IDs of profiles edited on this computer start with this, followed by the printer's ID.
const CUSTOM_PROFILE_PREFIX: &str = "custom:";
const MAX_SNAPSHOTS: usize = 16;
const MAX_SOURCES: usize = 8;

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
        let mut items = relock(&self.items);
        if items.len() >= self.cap {
            items.pop_front();
        }
        items.push_back((id, Arc::new(value)));
    }

    fn get(&self, id: &str) -> Option<Arc<T>> {
        relock(&self.items).iter().find(|(key, _)| key == id).map(|(_, value)| value.clone())
    }
}

/// Lock a mutex whose contents stay valid after any single operation on them (a cache, a map of
/// cancel tokens, a diagnostic holder, or the active-job slot, which is only ever replaced whole).
/// A panic while holding it cannot leave a half-applied change, so the poison flag is cleared
/// instead of panicking every later caller. Not for the job store: see `AppService::store`.
pub(crate) fn relock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        mutex.clear_poison();
        poisoned.into_inner()
    })
}

/// Run file I/O on a blocking thread, so a slow or network disk stalls neither the runtime's
/// workers nor a UI thread awaiting the result.
async fn blocking_io<T: Send + 'static>(work: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tokio::task::spawn_blocking(work).await.map_err(AppError::internal)?
}

/// Where a test may make the dispatcher panic. Compiled only with the `fault-injection` feature.
#[cfg(feature = "fault-injection")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    /// After the job is marked active, before it is claimed.
    BeforeClaim,
    /// After the durable claim, before any Bluetooth I/O.
    AfterClaim,
    /// While recording the outcome, holding the job store's lock.
    WhilePersisting,
    /// The dispatcher returns, without panicking, the next time it is woken outside shutdown.
    StopDispatcher,
}

/// Holds the dispatcher inside a job's cleanup, still owning Bluetooth, until released.
#[cfg(feature = "fault-injection")]
pub struct CleanupGate {
    reached: tokio::sync::Semaphore,
    release: tokio::sync::Semaphore,
}

#[cfg(feature = "fault-injection")]
impl Default for CleanupGate {
    fn default() -> Self {
        Self { reached: tokio::sync::Semaphore::new(0), release: tokio::sync::Semaphore::new(0) }
    }
}

#[cfg(feature = "fault-injection")]
impl CleanupGate {
    /// Resolves once the dispatcher is held at the gate.
    ///
    /// # Panics
    ///
    /// Never in practice: the gate's semaphore is never closed.
    pub async fn wait_reached(&self) {
        self.reached.acquire().await.expect("gate is never closed").forget();
    }

    /// Let the held dispatcher continue.
    pub fn release(&self) {
        self.release.add_permits(1);
    }
}

/// How [`AppService::shutdown`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownOutcome {
    /// The dispatcher and any active job ended. The service had not failed.
    Clean,
    /// Everything ended, but the service had failed (see [`ServiceFault`]).
    Faulted,
    /// The deadline passed. The dispatcher still owns the Bluetooth session and the data folder,
    /// and keeps them until its cleanup ends; call `shutdown` again to keep waiting.
    TimedOut,
}

struct Inner {
    config: ServiceConfig,
    registry: ProfileRegistry,
    transport: Arc<dyn Transport>,
    store: Mutex<Store>,
    events: Arc<dyn EventSink>,
    seq: AtomicU64,
    logs: Mutex<VecDeque<LogLine>>,
    log_writer: Option<Arc<LogWriter>>,
    pdf: Mutex<Option<AppResult<PdfEngine>>>,
    sources: Recent<Source>,
    snapshots: Recent<Snapshot>,
    renders: Arc<RenderQueue>,
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
    /// The supervisor, which owns the dispatcher's join handle.
    supervisor: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Set once, never cleared. Lock-free on purpose: reporting a failure must not depend on a
    /// mutex that the failed task may have poisoned.
    fault: OnceLock<ServiceFault>,
    #[cfg(feature = "fault-injection")]
    injected: Mutex<Option<FaultPoint>>,
    #[cfg(feature = "fault-injection")]
    gate: Mutex<Option<Arc<CleanupGate>>>,
    last_check: Mutex<Option<PrinterCheck>>,
    _temp: Option<tempfile::TempDir>,
}

#[derive(Clone)]
pub struct AppService {
    inner: Arc<Inner>,
}

impl AppService {
    /// Open storage, recover interrupted jobs and start the dispatcher.
    ///
    /// The dispatcher is a spawned task, so this must be called from within a Tokio runtime
    /// (any flavor, with the time driver enabled). The runtime must outlive the service; the
    /// dispatcher dies with it. `events` is called from service tasks and must not block.
    ///
    /// Jobs that were active when the previous process ended are marked uncertain and are never
    /// replayed; queued jobs stay queued (or held) and resume. With a `data_dir`, one process
    /// owns the folder at a time through `state.lock`. Without one, state lives in a temporary
    /// directory for the life of the service.
    ///
    /// **Shutdown:** call [`Self::shutdown`] and await it before dropping the service or the
    /// runtime. Dropping without it neither cancels an active transfer nor releases the Bluetooth
    /// link in an orderly way, and the data folder stays locked until the dispatcher task is gone.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::DataDirInUse`] when another process holds the data folder, and
    /// [`ErrorCode::Internal`] or I/O-derived errors when the folder, database or built-in
    /// profiles cannot be opened.
    ///
    /// # Panics
    ///
    /// Panics when called outside a Tokio runtime.
    ///
    /// ```
    /// use std::sync::Arc;
    /// use std::time::Duration;
    ///
    /// use nuprint_core::api::AppEvent;
    /// use nuprint_core::{AppService, ServiceConfig};
    /// use nuprint_transport::mock::MockTransport;
    ///
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let service = AppService::start(
    ///     ServiceConfig { data_dir: None, pdfium_dirs: vec![], version: "doc-example".into() },
    ///     Arc::new(MockTransport::new(vec![])), // no Bluetooth: an in-memory transport
    ///     Arc::new(|_event: AppEvent| {}),
    /// )?;
    /// assert!(service.list_jobs()?.jobs.is_empty());
    /// service.shutdown(Duration::from_secs(2)).await; // always, before the runtime ends
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Outside a runtime it panics instead of returning an error:
    ///
    /// ```should_panic
    /// # use std::sync::Arc;
    /// # use nuprint_core::api::AppEvent;
    /// # use nuprint_core::{AppService, ServiceConfig};
    /// # use nuprint_transport::mock::MockTransport;
    /// let config = ServiceConfig { data_dir: None, pdfium_dirs: vec![], version: "doc-example".into() };
    /// let _ = AppService::start(config, Arc::new(MockTransport::new(vec![])), Arc::new(|_: AppEvent| {}));
    /// ```
    pub fn start(config: ServiceConfig, transport: Arc<dyn Transport>, events: Arc<dyn EventSink>) -> AppResult<Self> {
        let registry = ProfileRegistry::builtin()?;
        let (mut store, temp, log_path) = match &config.data_dir {
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
            log_writer: log_path.map(|path| Arc::new(LogWriter::to_file(path))),
            pdf: Mutex::new(None),
            sources: Recent::new(MAX_SOURCES),
            snapshots: Recent::new(MAX_SNAPSHOTS),
            renders: RenderQueue::new(),
            scans: Mutex::new(HashMap::new()),
            inspections: Mutex::new(HashMap::new()),
            discovered: Mutex::new(HashMap::new()),
            wake: Notify::new(),
            held: AtomicBool::new(held),
            active: Mutex::new(None),
            hardware: tokio::sync::Mutex::new(()),
            hardware_use: Mutex::new(None),
            shutdown: CancellationToken::new(),
            supervisor: Mutex::new(None),
            fault: OnceLock::new(),
            #[cfg(feature = "fault-injection")]
            injected: Mutex::new(None),
            #[cfg(feature = "fault-injection")]
            gate: Mutex::new(None),
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
        let worker = tokio::spawn(async move { dispatcher.dispatch_loop().await });
        let supervisor = service.clone();
        let task = tokio::spawn(async move { supervisor.supervise(worker).await });
        *relock(&service.inner.supervisor) = Some(task);
        if let Some(writer) = &service.inner.log_writer {
            // Weak: the writer thread must not keep the service (and its data folder lock) alive.
            let weak = Arc::downgrade(&service.inner);
            writer.set_notifier(Arc::new(move |level, message| {
                if let Some(inner) = weak.upgrade() {
                    AppService { inner }.record(level, message, false);
                }
            }));
        }
        Ok(service)
    }

    /// The saved queue. A thread that panicked while holding the lock leaves SQLite with at most
    /// a rolled-back transaction, so the guard is recovered only after checking that: valid
    /// storage clears the poison; storage that fails the check fails the service closed.
    fn store(&self) -> MutexGuard<'_, Store> {
        match self.inner.store.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                let guard = poisoned.into_inner();
                if guard.is_consistent() {
                    self.inner.store.clear_poison();
                } else {
                    self.fail_closed(ServiceFaultKind::StorageInvalid);
                }
                guard
            }
        }
    }

    /// The error for work refused because the service has failed, if it has.
    fn fault_error(&self) -> Option<AppError> {
        self.inner.fault.get().map(|fault| {
            AppError::new(ErrorCode::Internal, fault.message.clone()).with_detail(format!("{:?}", fault.kind))
        })
    }

    #[cfg(feature = "fault-injection")]
    /// Test seam: make the dispatcher panic once, when it next reaches `point`.
    pub fn inject_panic(&self, point: FaultPoint) {
        *relock(&self.inner.injected) = Some(point);
    }

    #[cfg(feature = "fault-injection")]
    fn take_injected(&self, point: FaultPoint) -> bool {
        let mut armed = relock(&self.inner.injected);
        (*armed == Some(point)).then(|| armed.take()).is_some()
    }

    #[cfg(feature = "fault-injection")]
    fn maybe_panic(&self, point: FaultPoint) {
        if self.take_injected(point) {
            panic!("injected dispatcher fault at {point:?}");
        }
    }

    /// Test seam: hold the dispatcher in its next job's cleanup until `gate` is released.
    #[cfg(feature = "fault-injection")]
    pub fn inject_cleanup_gate(&self, gate: Arc<CleanupGate>) {
        *relock(&self.inner.gate) = Some(gate);
    }

    #[cfg(feature = "fault-injection")]
    async fn hold_cleanup(&self) {
        let gate = relock(&self.inner.gate).take();
        if let Some(gate) = gate {
            gate.reached.add_permits(1);
            gate.release.acquire().await.expect("gate is never closed").forget();
        }
    }

    /// Test seam: a thread panics while holding the store lock with a transaction left open,
    /// the one state `Store::is_consistent` rejects.
    ///
    /// # Panics
    ///
    /// The injected panic happens on a helper thread that this call joins, so it does not
    /// propagate; the caller returns normally.
    #[cfg(feature = "fault-injection")]
    pub fn inject_store_corruption(&self) {
        let inner = self.inner.clone();
        let _ = std::thread::spawn(move || {
            let store = inner.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            store.inject_open_transaction();
            panic!("injected store fault");
        })
        .join();
    }

    /// Test seam: kill the supervisor task, as a runtime abort would.
    #[cfg(feature = "fault-injection")]
    pub fn abort_supervisor(&self) {
        if let Some(task) = relock(&self.inner.supervisor).as_ref() {
            task.abort();
        }
    }

    fn next_seq(&self) -> f64 {
        self.inner.seq.fetch_add(1, Ordering::Relaxed) as f64 + 1.0
    }

    fn emit(&self, event: AppEvent) {
        self.inner.events.emit(event);
    }

    // ------------------------------------------------------------ app & profiles

    fn pdf_engine(&self) -> AppResult<PdfEngine> {
        let mut pdf = relock(&self.inner.pdf);
        pdf.get_or_insert_with(|| {
            PdfEngine::load(&self.inner.config.pdfium_dirs).map_err(|e| {
                AppError::new(ErrorCode::PdfUnavailable, "PDF support is unavailable in this build.")
                    .with_detail(e.to_string())
            })
        })
        .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuprint_protocol::MonoBitmap;
    use std::time::Duration;

    #[tokio::test]
    async fn imports_and_saves_run_off_the_async_workers_with_the_same_results() {
        let dir = tempfile::tempdir().unwrap();
        let config = ServiceConfig { data_dir: None, pdfium_dirs: vec![], version: "test".into() };
        let transport = Arc::new(nuprint_transport::mock::MockTransport::new(vec![]));
        let service = AppService::start(config, transport, Arc::new(|_| {})).unwrap();

        let png = nuprint_render::encode_png(&MonoBitmap::new(8, 8).unwrap());
        let path = dir.path().join("dot.png");
        fs::write(&path, &png).unwrap();
        let info = service.import_source(&path).await.unwrap();
        assert_eq!((info.kind, info.bytes, info.name.as_str()), (SourceKind::Image, png.len() as f64, "dot.png"));

        let profile_id = service.profiles()[0].id.clone();
        let preview = service
            .render_preview(PreviewRequest { generation: 1.0, profile_id, source: SourceSpec::TestPattern })
            .await
            .unwrap();
        let saved = dir.path().join("preview.png");
        service.save_preview_png(&preview.snapshot_id, &saved).await.unwrap();
        assert_eq!(fs::read(&saved).unwrap(), *service.preview_png(&preview.snapshot_id).unwrap());

        let report = dir.path().join("report.json");
        service.export_diagnostics(&report, false).await.unwrap();
        assert!(fs::read_to_string(&report).unwrap().contains("\"includes_identifiers\""));
        service.shutdown(Duration::from_secs(5)).await;
    }
}
