//! Typed frontend/backend contract. TypeScript declarations are generated from these
//! types by ts-rs (`cargo test -p nuprint-core export_bindings`) into
//! `apps/tauri/src/bindings/`. Never hand-edit the generated files.
//!
//! IDs are opaque strings. Times are UTC milliseconds since the Unix epoch.

pub use nuprint_protocol::{CompletionPolicy, Density, FeedCapability, StatusFlag};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::AppError;

// ---------------------------------------------------------------- app & profiles

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AppInfo {
    pub version: String,
    pub renderer_version: String,
    pub os: String,
    pub data_dir: Option<String>,
    pub pdf_available: bool,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_error: Option<String>,
    pub persistent: bool,
}

/// Support is tracked per profile revision × OS/backend (docs/printer-support.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SupportLevel {
    Candidate,
    Experimental,
    VerifiedBasic,
    ReleaseSupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProfileSummary {
    pub id: String,
    pub revision: u32,
    pub display_name: String,
    pub vendor: String,
    pub model: String,
    pub protocol_id: String,
    /// Effective level on this OS.
    pub support: SupportLevel,
    pub support_note: String,
    pub width_dots: u32,
    pub dots_per_mm: f32,
    pub max_height: u32,
    pub densities: Vec<Density>,
    pub default_density: Density,
    pub feed: FeedCapability,
    pub max_copies: u16,
    pub media: String,
    pub completion: CompletionPolicy,
}

// ---------------------------------------------------------------- discovery & printers

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AdapterStatus {
    Ready,
    PoweredOff,
    Absent,
    PermissionDenied,
    /// macOS will ask for permission the first time NuPrint looks for printers.
    PermissionNotAsked,
    /// macOS has asked and is waiting for the user's answer.
    PermissionPending,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProfileMatch {
    pub profile_id: String,
    pub revision: u32,
    pub display_name: String,
    pub support: SupportLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DiscoveredPrinter {
    /// OS/backend-scoped identity from this scan; revalidated per session.
    pub discovery_id: String,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rssi: Option<i16>,
    pub advertised_services: Vec<String>,
    /// Name-based candidates only; a candidate is not permission to print.
    pub candidates: Vec<ProfileMatch>,
    /// Already saved as this printer ID.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_printer_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StartScanRequest {
    /// Seconds, 1–30. Default 8.
    #[ts(optional)]
    pub duration_s: Option<u32>,
    /// Also report devices that match no profile (diagnostic view).
    #[ts(optional)]
    pub include_unknown: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ScanStarted {
    pub scan_id: String,
    pub adapter: AdapterStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GattCharacteristic {
    pub uuid: String,
    pub properties: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GattService {
    pub uuid: String,
    pub characteristics: Vec<GattCharacteristic>,
}

/// Result of a read-only GATT inspection: no vendor writes are issued.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DeviceInspection {
    pub discovery_id: String,
    #[ts(optional)]
    pub name: Option<String>,
    pub services: Vec<GattService>,
    /// Profiles whose name rules AND required endpoints match.
    pub compatible: Vec<ProfileMatch>,
    /// Name candidates rejected by endpoint checks, with reasons.
    pub rejected: Vec<String>,
    pub max_write_len: u32,
}

/// A saved printer's editable profile settings (Printers → Advanced).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PrinterProfileDoc {
    pub printer_id: String,
    pub base_profile_id: String,
    pub base_display_name: String,
    /// The printer uses edited settings rather than the built-in ones.
    pub customized: bool,
    /// The built-in settings as JSON, for reset and comparison.
    pub base_json: String,
    /// The settings in use now as JSON.
    pub current_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SavePrinterProfileRequest {
    pub printer_id: String,
    /// The edited settings as a JSON object; see `PrinterProfileDoc`.
    pub json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SavePrinterRequest {
    pub discovery_id: String,
    pub profile_id: String,
    /// Required to save a profile whose support on this OS is experimental.
    pub acknowledge_experimental: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PrinterSettings {
    pub density: Density,
    pub feed_mm: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SavedPrinter {
    /// App-local ID; the cross-platform primary key.
    pub id: String,
    pub name: String,
    pub backend_id: String,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    pub profile_id: String,
    pub profile_revision: u32,
    pub profile_name: String,
    pub support: SupportLevel,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware: Option<String>,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_ms: Option<f64>,
    pub settings: PrinterSettings,
    /// False when the stored profile revision no longer exists in this build.
    pub profile_available: bool,
    /// Its settings were edited on this computer (Printers → Advanced).
    pub customized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PrinterCheck {
    pub printer_id: String,
    pub ready: bool,
    pub status_text: String,
    pub flags: Vec<StatusFlag>,
    pub raw_status_hex: String,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware: Option<String>,
    pub max_write_len: u32,
    pub services: Vec<GattService>,
    pub notifications: Vec<String>,
    pub checked_ms: f64,
}

// ---------------------------------------------------------------- sources & preview

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SourceKind {
    Image,
    Pdf,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SourceInfo {
    pub source_id: String,
    pub kind: SourceKind,
    /// File name only (never the full path).
    pub name: String,
    pub bytes: f64,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FontFamily {
    #[default]
    Sans,
    Mono,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Tone {
    /// Photo shading (Floyd–Steinberg).
    #[default]
    Dither,
    /// High-contrast shading (Atkinson).
    Atkinson,
    /// Crisp black/white for text and line art.
    Threshold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Rotation {
    #[default]
    #[serde(rename = "0")]
    R0,
    #[serde(rename = "90")]
    R90,
    #[serde(rename = "180")]
    R180,
    #[serde(rename = "270")]
    R270,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TextSpec {
    pub text: String,
    /// Pixels (dots), 12–96.
    pub size_px: u16,
    pub family: FontFamily,
    pub bold: bool,
    pub align: TextAlign,
    /// Multiplier, 0.8–3.0.
    pub line_spacing: f32,
    /// Dots, applied to all sides.
    pub margin_dots: u16,
    /// Lines starting with "# " render as headings.
    pub headings: bool,
}

impl Default for TextSpec {
    fn default() -> Self {
        Self {
            text: String::new(),
            size_px: 24,
            family: FontFamily::Sans,
            bold: false,
            align: TextAlign::Left,
            line_spacing: 1.2,
            margin_dots: 12,
            headings: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ImageAdjust {
    pub tone: Tone,
    pub rotation: Rotation,
    /// -100–100.
    pub brightness: i8,
    /// -100–100.
    pub contrast: i8,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum SourceSpec {
    Text(TextSpec),
    Image {
        source_id: String,
        adjust: ImageAdjust,
    },
    Pdf {
        source_id: String,
        /// 1-based.
        page: u32,
        adjust: ImageAdjust,
        /// Kept in memory only; never persisted.
        #[ts(optional)]
        password: Option<String>,
    },
    TestPattern,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PreviewRequest {
    /// Monotonic per view; older generations are rejected as stale.
    pub generation: f64,
    /// The profile whose dimensions the raster targets (from the selected printer).
    pub profile_id: String,
    pub source: SourceSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PreviewResult {
    pub generation: f64,
    pub snapshot_id: String,
    pub profile_id: String,
    pub profile_revision: u32,
    pub width: u32,
    pub height: u32,
    /// Raster length only (excludes feed).
    pub length_mm: f32,
    pub sha256: String,
    pub black_ratio: f32,
    pub warnings: Vec<String>,
    /// Path for the preview asset protocol: `convertFileSrc(preview_path, "nupreview")`.
    pub preview_path: String,
}

// ---------------------------------------------------------------- jobs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum JobState {
    Held,
    Queued,
    Connecting,
    Sending,
    WaitingForReady,
    Completed,
    TransferUnconfirmed,
    Accepted,
    Failed,
    Uncertain,
    Cancelled,
}

impl JobState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobState::Completed
                | JobState::TransferUnconfirmed
                | JobState::Accepted
                | JobState::Failed
                | JobState::Uncertain
                | JobState::Cancelled
        )
    }

    pub fn is_active(self) -> bool {
        matches!(self, JobState::Connecting | JobState::Sending | JobState::WaitingForReady)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Held => "held",
            JobState::Queued => "queued",
            JobState::Connecting => "connecting",
            JobState::Sending => "sending",
            JobState::WaitingForReady => "waiting_for_ready",
            JobState::Completed => "completed",
            JobState::TransferUnconfirmed => "transfer_unconfirmed",
            JobState::Accepted => "accepted",
            JobState::Failed => "failed",
            JobState::Uncertain => "uncertain",
            JobState::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "held" => JobState::Held,
            "queued" => JobState::Queued,
            "connecting" => JobState::Connecting,
            "sending" => JobState::Sending,
            "waiting_for_ready" => JobState::WaitingForReady,
            "completed" => JobState::Completed,
            "transfer_unconfirmed" => JobState::TransferUnconfirmed,
            "accepted" => JobState::Accepted,
            "failed" => JobState::Failed,
            "uncertain" => JobState::Uncertain,
            "cancelled" => JobState::Cancelled,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EnqueueRequest {
    pub snapshot_id: String,
    pub printer_id: String,
    /// 1–profile max (20 for SC03h).
    pub copies: u16,
    pub density: Density,
    pub feed_mm: u16,
    /// One per user submission action; repeated submissions return the same job.
    pub idempotency_key: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobSummary {
    pub id: String,
    pub title: String,
    pub state: JobState,
    /// Plain-language outcome, e.g. "Transfer complete · printer reports ready".
    pub state_text: String,
    pub printer_id: String,
    pub printer_name: String,
    pub profile_id: String,
    pub profile_revision: u32,
    pub created_ms: f64,
    pub updated_ms: f64,
    pub copies: u16,
    pub pages: u32,
    pub width: u32,
    /// Total raster rows across one copy.
    pub height: u32,
    pub density: Density,
    pub feed_mm: u16,
    /// Estimated paper: raster length × copies + final feed.
    pub paper_mm: f32,
    pub bytes_total: f64,
    pub bytes_sent: f64,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AppError>,
    /// Cancelled/failed after print bytes were submitted: paper may have moved.
    pub partial_output_possible: bool,
    pub content_retained: bool,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_of: Option<String>,
    pub revision: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct QueueSnapshot {
    pub held: bool,
    pub revision: f64,
    /// Event sequence number when this snapshot was taken: events with `seq <= seq`
    /// are already reflected in it.
    pub seq: f64,
    pub jobs: Vec<JobSummary>,
    /// Set when the queue worker failed. Nothing starts until NuPrint is restarted.
    #[ts(optional)]
    pub fault: Option<ServiceFault>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ServiceFaultKind {
    /// The queue worker panicked.
    DispatcherPanicked,
    /// The queue worker returned while the service was still running.
    DispatcherStopped,
    /// The saved queue failed validation after an internal failure.
    StorageInvalid,
}

/// A failure of NuPrint's own queue worker. The service refuses to start jobs and to use
/// Bluetooth until the app is restarted, because it cannot prove the failed worker released it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ServiceFault {
    pub kind: ServiceFaultKind,
    /// What happened and what to do, without printer names or job content.
    pub message: String,
    pub at_ms: f64,
    /// The job that was claimed when the worker failed, if any. It is marked uncertain.
    #[ts(optional)]
    pub job_id: Option<String>,
}

// ---------------------------------------------------------------- events & diagnostics

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Debug,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LogLine {
    pub seq: f64,
    pub at_ms: f64,
    pub level: LogLevel,
    pub message: String,
}

/// Single event channel `nuprint://event`. Events carry a sequence number; durable
/// state must be re-read with `list_jobs` after (re)subscribing.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum AppEvent {
    ScanDevice {
        scan_id: String,
        device: DiscoveredPrinter,
    },
    ScanFinished {
        scan_id: String,
        found: u32,
        #[ts(optional)]
        error: Option<AppError>,
    },
    JobChanged {
        seq: f64,
        job: JobSummary,
        queue_held: bool,
    },
    JobRemoved {
        seq: f64,
        job_id: String,
    },
    JobProgress {
        seq: f64,
        job_id: String,
        attempt_id: String,
        bytes_sent: f64,
        bytes_total: f64,
    },
    PrintersChanged {
        seq: f64,
    },
    /// The queue worker failed; see [`ServiceFault`]. Durable state is re-read with `list_jobs`.
    QueueFault {
        seq: f64,
        fault: ServiceFault,
    },
    Log {
        line: LogLine,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DiagnosticsReport {
    pub generated_ms: f64,
    pub app: AppInfo,
    pub adapter: AdapterStatus,
    pub profiles: Vec<ProfileSummary>,
    pub printers: Vec<SavedPrinter>,
    #[ts(optional)]
    pub last_check: Option<PrinterCheck>,
    pub recent_jobs: Vec<JobSummary>,
    pub log: Vec<LogLine>,
    /// Whether names/addresses/backend IDs are included (user-reviewed).
    pub includes_identifiers: bool,
}
