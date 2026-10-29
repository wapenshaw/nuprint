//! Plain mirrors of what the screens show, kept between UI callbacks and service events.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

use nuprint_core::api::*;
use serde::{Deserialize, Serialize};
use slint::{Rgba8Pixel, SharedPixelBuffer};

#[derive(Serialize, Deserialize, Default, Clone, Copy)]
pub(crate) struct Prefs {
    pub(crate) mode: i32,
    pub(crate) accent: i32,
    pub(crate) icon: i32,
}

#[derive(Clone)]
pub(crate) struct Status {
    pub(crate) text: String,
    pub(crate) tone: i32,
    pub(crate) action: Option<(&'static str, &'static str)>,
}

/// What the last connection check of one printer showed.
#[derive(Clone, Default)]
pub(crate) struct CheckView {
    pub(crate) ok: bool,
    pub(crate) line: String,
    pub(crate) flags: String,
    pub(crate) firmware: String,
    pub(crate) error: String,
    pub(crate) paper: bool,
}

/// Plain mirrors of what the screens show. Slint objects stay on the UI thread.
#[derive(Default)]
pub(crate) struct State {
    pub(crate) prefs: Prefs,
    pub(crate) data_dir: PathBuf,
    pub(crate) app_info: Option<AppInfo>,
    pub(crate) profiles: Vec<ProfileSummary>,
    pub(crate) printers: Vec<SavedPrinter>,
    pub(crate) selected: Option<String>,
    pub(crate) applied_printer: Option<String>,
    pub(crate) checks: HashMap<String, CheckView>,
    pub(crate) checking: Option<String>,
    pub(crate) last_check: Option<(String, PrinterCheck)>,
    pub(crate) adapter: Option<AdapterStatus>,
    pub(crate) found: Vec<DiscoveredPrinter>,
    pub(crate) scan: Option<String>,
    pub(crate) scan_error: String,
    pub(crate) image_src: Option<SourceInfo>,
    pub(crate) pdf_src: Option<SourceInfo>,
    pub(crate) preview: Option<PreviewResult>,
    pub(crate) preview_error: bool,
    pub(crate) req_seq: u64,
    pub(crate) generation: f64,
    pub(crate) feed_seq: u64,
    pub(crate) last_print: Option<Instant>,
    pub(crate) jobs: Vec<JobSummary>,
    pub(crate) held: bool,
    pub(crate) thumbs: HashMap<String, SharedPixelBuffer<Rgba8Pixel>>,
    pub(crate) tracked: Option<String>,
    pub(crate) status: Option<Status>,
    /// Sequence numbers of the activity log's rows, oldest first. Only the UI thread changes it.
    pub(crate) shown_logs: VecDeque<f64>,
    /// Log lines not on screen yet, and whether the UI thread is already due to add them.
    pub(crate) new_logs: Vec<LogLine>,
    pub(crate) logs_queued: bool,
    pub(crate) dialog_device: Option<String>,
    pub(crate) dialog_printer: Option<String>,
    pub(crate) save_candidates: Vec<ProfileMatch>,
    pub(crate) adv_doc: Option<PrinterProfileDoc>,
    pub(crate) adv_typing: HashMap<usize, String>,
}
