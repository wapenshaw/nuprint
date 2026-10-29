//! The application service: one instance owns the queue, dispatcher, storage and all
//! Bluetooth activity for this process. The Tauri host and the CLI both drive it.

mod preview;
mod printers;
mod queue;

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use nuprint_protocol::CompletionPolicy;
use nuprint_render::{PdfEngine, RENDERER_VERSION};
use nuprint_transport::{Advertisement, Transport};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::api::*;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::jobs::classify;
use crate::log_writer::LogWriter;
use crate::profile::{ModelProfile, ProfileRegistry, ResolvedProfile};
use crate::render_queue::RenderQueue;
use crate::session::{LogFn, PrinterSession, SessionEvent, locate};
use crate::store::{JobRow, Store, now_ms};
use preview::{Snapshot, Source};

pub const MAX_QUEUE_JOBS: i64 = 100;
pub const MAX_RETAINED_BYTES: i64 = 256 * 1024 * 1024;
const MAX_LOG_LINES: usize = 500;
/// IDs of profiles edited on this computer start with this, followed by the printer's ID.
const CUSTOM_PROFILE_PREFIX: &str = "custom:";
const MAX_SNAPSHOTS: usize = 16;
const MAX_SOURCES: usize = 8;
/// How long shutdown and a diagnostics export wait for buffered log lines to reach the file.
const LOG_FLUSH_WAIT: Duration = Duration::from_secs(2);
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

struct ActiveJob {
    job_id: String,
    cancel: CancellationToken,
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
    pub async fn wait_reached(&self) {
        self.reached.acquire().await.expect("gate is never closed").forget();
    }

    /// Let the held dispatcher continue.
    pub fn release(&self) {
        self.release.add_permits(1);
    }
}

#[cfg(feature = "fault-injection")]
macro_rules! fault_point {
    ($service:expr, $point:ident) => {
        $service.maybe_panic(FaultPoint::$point)
    };
}
#[cfg(not(feature = "fault-injection"))]
macro_rules! fault_point {
    ($service:expr, $point:ident) => {};
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
        *relock(self.holder) = None;
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

    /// Stop accepting work, cancel the active transfer and wait (bounded) for its outcome
    /// to be persisted and the dispatcher to end. Pending jobs stay queued/held for the next
    /// start.
    ///
    /// `wait` bounds each of two waits: the active job reaching a terminal state (cancelled, or
    /// uncertain if print bytes may already have been sent), then the dispatcher's supervisor
    /// exiting. A job that never settles is left for restart recovery, which marks it uncertain;
    /// it is never replayed. On [`ShutdownOutcome::TimedOut`] nothing is released: the dispatcher
    /// (which owns the Bluetooth session) and the supervisor keep the service, so the data folder
    /// stays locked until their cleanup really ends. Safe to call more than once.
    pub async fn shutdown(&self, wait: Duration) -> ShutdownOutcome {
        self.inner.shutdown.cancel();
        if let Some(active) = relock(&self.inner.active).as_ref() {
            active.cancel.cancel();
        }
        let deadline = Instant::now() + wait;
        while relock(&self.inner.active).is_some() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let task = relock(&self.inner.supervisor).take();
        if let Some(mut task) = task {
            match tokio::time::timeout(wait, &mut task).await {
                Err(_) => {
                    *relock(&self.inner.supervisor) = Some(task);
                    return ShutdownOutcome::TimedOut;
                }
                // The supervisor itself died; the dispatcher's result was never classified.
                Ok(Err(_)) => self.fail_closed(ServiceFaultKind::DispatcherStopped),
                Ok(Ok(())) => {}
            }
        }
        if relock(&self.inner.active).is_some() {
            return ShutdownOutcome::TimedOut;
        }
        // The dispatcher has ended; later lines stay in memory. On a timeout the file stays open,
        // because the dispatcher may still log while it cleans up.
        self.close_log_file().await;
        if self.inner.fault.get().is_some() { ShutdownOutcome::Faulted } else { ShutdownOutcome::Clean }
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

    // ------------------------------------------------------------ supervision

    /// Watch the dispatcher. It ends only at a requested shutdown; any other end is a failure.
    async fn supervise(self, worker: tokio::task::JoinHandle<()>) {
        let kind = match worker.await {
            Ok(()) if self.inner.shutdown.is_cancelled() => return,
            Err(e) if e.is_panic() => ServiceFaultKind::DispatcherPanicked,
            Err(_) if self.inner.shutdown.is_cancelled() => return,
            Ok(()) | Err(_) => ServiceFaultKind::DispatcherStopped,
        };
        self.fail_closed(kind);
    }

    /// Stop the service for good after a failure: record it (lock-free, first failure wins),
    /// hold the queue, and reconcile the claimed attempt. Never panics and never requeues.
    ///
    /// Bluetooth is not released afterwards. Dropping the failed task is not proof that the native
    /// connection ended, so `claim_hardware` refuses until the app restarts (increment B of the
    /// follow-up plan may allow a bounded in-process recovery).
    fn fail_closed(&self, kind: ServiceFaultKind) {
        // A dead dispatcher can no longer finish its job, so its slot is released here. On a
        // storage failure the dispatcher is still running and keeps its slot until it unwinds.
        let dispatcher_gone = kind != ServiceFaultKind::StorageInvalid;
        let job_id = {
            let mut active = relock(&self.inner.active);
            let id = active.as_ref().map(|a| {
                a.cancel.cancel();
                a.job_id.clone()
            });
            if dispatcher_gone {
                *active = None;
            }
            id
        };
        let fault = ServiceFault {
            kind,
            message: match kind {
                ServiceFaultKind::StorageInvalid => {
                    "NuPrint's saved queue could not be checked after an internal error. Restart NuPrint; \
                     nothing will be printed until then."
                }
                _ => {
                    "NuPrint's print queue stopped working. Nothing will be printed or connected until you \
                     restart NuPrint. Waiting jobs are kept. If a job was printing, check the paper before \
                     printing it again."
                }
            }
            .into(),
            at_ms: now_ms() as f64,
            job_id: job_id.clone(),
        };
        if self.inner.fault.set(fault.clone()).is_err() {
            return;
        }
        self.inner.held.store(true, Ordering::Relaxed);
        // The rest touches locks and callbacks that may be the reason for the failure.
        let _ = catch_unwind(AssertUnwindSafe(|| {
            // On a storage failure the caller may hold the poisoned store lock; leave the job for
            // the restart recovery rather than lock it again here.
            let note = match &job_id {
                Some(job_id) if dispatcher_gone => self.reconcile_failed_attempt(job_id),
                Some(_) => "the active job stays active and a restart will mark it uncertain",
                None => "no job was active",
            };
            self.log(LogLevel::Error, format!("Queue stopped ({kind:?}); {note}. Restart NuPrint."));
            self.emit(AppEvent::QueueFault { seq: self.next_seq(), fault });
        }));
    }

    /// Mark the attempt the failed dispatcher had claimed as uncertain, if storage can be saved.
    /// Otherwise it stays active and the next start marks it uncertain.
    fn reconcile_failed_attempt(&self, job_id: &str) -> &'static str {
        let result = {
            let mut store = self.store();
            if !store.is_consistent() {
                return "storage is unusable, so a restart will mark the job uncertain";
            }
            store.mark_active_uncertain(
                job_id,
                "NuPrint's print queue failed while this job was printing. Check the paper; it was not resent.",
            )
        };
        match result {
            Ok(Some(job)) => {
                self.emit_job(&job);
                "the job was marked uncertain"
            }
            Ok(None) => "the job had not started or had finished",
            Err(_) => "the job could not be saved, so a restart will mark it uncertain",
        }
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

    // ------------------------------------------------------------ logging

    pub fn log(&self, level: LogLevel, message: String) {
        self.record(level, message, true);
    }

    /// Add a line to the in-memory log and the UI. The file gets it too when `to_file`; the
    /// writer's own health notices pass false so a failing file cannot log about itself.
    fn record(&self, level: LogLevel, message: String, to_file: bool) {
        let at_ms = now_ms();
        // Everything that allocates happens before the lock; the markers stay in the ring.
        let plain = unmark(&message, None);
        let file_line = to_file.then(|| format!("{at_ms} {level:?} {plain}\n"));
        let line = {
            // A bounded ring that is only pushed and popped whole, so a panic elsewhere while it
            // was held cannot leave it half-changed; logging must keep working after a failure.
            let mut logs = relock(&self.inner.logs);
            // Sequence, ring order and file order are all decided under this lock.
            let seq = self.next_seq();
            if logs.len() >= MAX_LOG_LINES {
                logs.pop_front();
            }
            logs.push_back(LogLine { seq, at_ms: at_ms as f64, level, message });
            if let (Some(writer), Some(text)) = (&self.inner.log_writer, file_line) {
                writer.submit(level, text);
            }
            LogLine { seq, at_ms: at_ms as f64, level, message: plain }
        };
        match level {
            LogLevel::Error => tracing::error!("{}", line.message),
            LogLevel::Warn => tracing::warn!("{}", line.message),
            LogLevel::Info => tracing::info!("{}", line.message),
            LogLevel::Debug => tracing::debug!("{}", line.message),
        }
        self.emit(AppEvent::Log { line });
    }

    /// Wait (bounded) for buffered lines to be written. Without a data folder there is no file.
    async fn flush_log_file(&self) {
        if let Some(writer) = self.inner.log_writer.clone() {
            let _ = tokio::task::spawn_blocking(move || writer.flush(LOG_FLUSH_WAIT)).await;
        }
    }

    /// Write what is buffered and stop the file writer; later lines reach only the in-memory log.
    async fn close_log_file(&self) {
        if let Some(writer) = self.inner.log_writer.clone() {
            let _ = tokio::task::spawn_blocking(move || writer.close(LOG_FLUSH_WAIT)).await;
        }
    }

    fn logger(&self) -> LogFn {
        let service = self.clone();
        Arc::new(move |level, message| service.log(level, message))
    }

    pub fn logs(&self) -> Vec<LogLine> {
        relock(&self.inner.logs).iter().map(|l| LogLine { message: unmark(&l.message, None), ..l.clone() }).collect()
    }

    /// The log for a diagnostics export: identifying text is replaced unless `include_identifiers`.
    fn export_logs(&self, include_identifiers: bool) -> Vec<LogLine> {
        if include_identifiers {
            return self.logs();
        }
        let replacement = Some("<redacted>");
        relock(&self.inner.logs)
            .iter()
            .map(|l| LogLine { message: unmark(&l.message, replacement), ..l.clone() })
            .collect()
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

    /// Take the Bluetooth lock for a user action. Only an inspection is waited for, because it is
    /// short and bounded; behind a print or a check the action is refused as busy instead.
    async fn claim_hardware(&self, purpose: HardwareUse, cancel: &CancellationToken) -> AppResult<HardwareClaim<'_>> {
        // After a dispatcher failure the old Bluetooth session may still be open.
        if let Some(error) = self.fault_error() {
            return Err(error);
        }
        let guard = match self.inner.hardware.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                let holder = *relock(&self.inner.hardware_use);
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
        *relock(&self.inner.hardware_use) = Some(purpose);
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
        relock(&self.inner.discovered).insert(adv.backend_id.clone(), adv);
        PrinterSession::open(self.inner.transport.as_ref(), resolved, backend_id, self.logger(), cancel).await
    }

    // ------------------------------------------------------------ dispatcher

    async fn dispatch_loop(self) {
        loop {
            while !self.inner.held.load(Ordering::Relaxed)
                && !self.inner.shutdown.is_cancelled()
                && self.inner.fault.get().is_none()
            {
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
            #[cfg(feature = "fault-injection")]
            if self.take_injected(FaultPoint::StopDispatcher) {
                return;
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
        let active = relock(&self.inner.active);
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
        *relock(&self.inner.active) = Some(ActiveJob { job_id: queued.id.clone(), cancel: cancel.clone() });
        fault_point!(self, BeforeClaim);
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
            *relock(&self.inner.active) = None;
            return;
        };
        // Claim the job durably before any Bluetooth I/O. If it is no longer queued, or the claim
        // cannot be saved, nothing is sent.
        let attempt_id = Uuid::new_v4().to_string();
        let claimed = self.store().claim_queued(&queued.id, &attempt_id);
        let mut job = match claimed {
            Ok(Some(job)) => job,
            Ok(None) => {
                *relock(&self.inner.active) = None;
                return;
            }
            Err(e) => {
                self.log(LogLevel::Error, format!("Could not claim a job: {e}"));
                self.pause_queue_for_storage_error("the start of a job");
                *relock(&self.inner.active) = None;
                return;
            }
        };
        fault_point!(self, AfterClaim);
        self.emit_job(&job);
        let (result, sent) = self.execute_job(&mut job, &attempt_id, &cancel).await;
        #[cfg(feature = "fault-injection")]
        self.hold_cleanup().await;
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
            fault_point!(self, WhilePersisting);
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
        *relock(&self.inner.active) = None;
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
        let mut last_check = relock(&self.inner.last_check).clone();
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
        self.flush_log_file().await;
        let report = self.diagnostics_report(include_identifiers).await?;
        let json = serde_json::to_string_pretty(&report).map_err(AppError::internal)?;
        let path = path.to_path_buf();
        blocking_io(move || Ok(fs::write(path, json)?)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuprint_protocol::MonoBitmap;

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

#[cfg(test)]
#[path = "log_tests.rs"]
mod log_tests;
