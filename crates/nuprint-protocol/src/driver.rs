//! Protocol-independent driver contract (docs/printer-support.md).
//!
//! A driver turns a canonical bitmap plus validated print settings into an ordered
//! [`ExchangePlan`] and interprets notification bytes as [`ProtocolEvent`]s. It never
//! performs I/O; the session in `nuprint-core` executes plans over a transport.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::MonoBitmap;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("{0}")]
    InvalidRaster(String),
    #[error("darkness preset '{0}' is not available for this printer")]
    UnsupportedDensity(String),
    #[error("Extra feed must be a whole number from {min} to {max} mm")]
    FeedOutOfRange { min: u16, max: u16 },
    #[error("packet payload of {0} bytes is too large")]
    PayloadTooLarge(usize),
}

/// Named heat/density presets. Profiles decide which are offered; there is no free heat value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Density {
    Light,
    Normal,
    Dark,
}

impl Density {
    pub fn as_str(self) -> &'static str {
        match self {
            Density::Light => "light",
            Density::Normal => "normal",
            Density::Dark => "dark",
        }
    }
}

impl std::str::FromStr for Density {
    type Err = ProtocolError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "light" => Ok(Density::Light),
            "normal" => Ok(Density::Normal),
            "dark" => Ok(Density::Dark),
            other => Err(ProtocolError::UnsupportedDensity(other.to_owned())),
        }
    }
}

/// One canonical page plus print-time settings. Feed applies after this page only when
/// `trailing_feed_mm` is non-zero; callers pass zero for all but the final page/copy.
#[derive(Debug, Clone, Copy)]
pub struct PrintRequest<'a> {
    pub bitmap: &'a MonoBitmap,
    pub density: Density,
    pub trailing_feed_mm: u16,
}

/// What evidence a model provides that a job finished (docs/technical-spec.md §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum CompletionPolicy {
    /// A fresh status reply after draining reports idle/ready.
    StatusConfirmedReady,
    /// The model acknowledges acceptance but not paper completion.
    AcknowledgedAcceptance,
    /// Write-only model: bounded drain, output unconfirmed.
    UnconfirmedTransfer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanStep {
    /// Send the status query and require a fresh, valid, non-blocking, idle report.
    RequireReady { timeout: Duration },
    /// Stream print bytes. The session chunks them, paces, honours flow control and
    /// checks for blocking faults between chunks. Frames may span chunks.
    SendPrintData { bytes: Vec<u8> },
    /// Fixed wait (e.g. letting the printer buffer drain).
    Settle(Duration),
    /// Query status until idle and writable, or fail as uncertain after `deadline`.
    PollUntilReady { interval: Duration, deadline: Duration, query_timeout: Duration },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangePlan {
    pub steps: Vec<PlanStep>,
    pub completion: CompletionPolicy,
}

impl ExchangePlan {
    pub fn print_bytes(&self) -> usize {
        self.steps.iter().map(|s| if let PlanStep::SendPrintData { bytes } = s { bytes.len() } else { 0 }).sum()
    }
}

/// Status fields. Unknown bits are retained in `raw` rather than guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum StatusFlag {
    PaperOut,
    CoverOpen,
    Overheated,
    LowPower,
    Charging,
    Printing,
}

impl StatusFlag {
    pub fn label(self) -> &'static str {
        match self {
            StatusFlag::PaperOut => "Out of paper",
            StatusFlag::CoverOpen => "Cover open",
            StatusFlag::Overheated => "Print head too hot",
            StatusFlag::LowPower => "Battery low",
            StatusFlag::Charging => "Charging",
            StatusFlag::Printing => "Printing",
        }
    }

    /// Faults that must stop a print before or during transfer.
    pub fn is_blocking(self) -> bool {
        matches!(self, StatusFlag::PaperOut | StatusFlag::CoverOpen | StatusFlag::Overheated | StatusFlag::LowPower)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusReport {
    pub raw: Vec<u8>,
    pub flags: Vec<StatusFlag>,
}

impl StatusReport {
    /// The first fault that must stop a print, if any.
    pub fn blocking_fault(&self) -> Option<StatusFlag> {
        self.flags.iter().copied().find(|f| f.is_blocking())
    }

    pub fn is_busy(&self) -> bool {
        self.flags.contains(&StatusFlag::Printing)
    }

    pub fn is_ready(&self) -> bool {
        self.blocking_fault().is_none() && !self.is_busy()
    }

    pub fn text(&self) -> String {
        if self.flags.is_empty() {
            "Ready".to_owned()
        } else {
            self.flags.iter().map(|f| f.label()).collect::<Vec<_>>().join(", ")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Best-effort firmware string parsed from the reply; `raw` is authoritative.
    pub firmware: Option<String>,
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolEvent {
    Status(StatusReport),
    FlowPause,
    FlowResume,
    DeviceInfo(DeviceInfo),
    /// Valid frame the driver does not interpret; kept for diagnostics.
    Unhandled {
        opcode: u8,
        payload: Vec<u8>,
    },
}

/// Reassembles fragmented notifications for one characteristic.
pub trait NotificationDecoder: Send {
    fn feed(&mut self, data: &[u8]) -> Vec<ProtocolEvent>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticQuery {
    pub name: &'static str,
    pub bytes: Vec<u8>,
    /// Best-effort queries do not fail diagnostics when unanswered.
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FeedCapability {
    pub min_mm: u16,
    pub max_mm: u16,
    pub default_mm: u16,
    /// Whether a non-default distance has physical calibration evidence.
    pub calibrated_reduced: bool,
}

impl FeedCapability {
    /// Whether `mm` is an accepted extra-feed distance.
    pub fn contains(&self, mm: u16) -> bool {
        (self.min_mm..=self.max_mm).contains(&mm)
    }
}

pub trait PrinterDriver: Send + Sync {
    fn protocol_id(&self) -> &'static str;
    fn protocol_version(&self) -> u32;
    fn new_decoder(&self) -> Box<dyn NotificationDecoder>;
    /// `None` for write-only models without a status query.
    fn status_query(&self) -> Option<Vec<u8>>;
    /// How long to wait for a fresh reply to [`Self::status_query`].
    fn status_timeout(&self) -> Duration;
    /// Non-printing queries used by diagnostics (status is issued separately).
    fn diagnostic_queries(&self) -> Vec<DiagnosticQuery>;
    fn densities(&self) -> Vec<Density>;
    fn feed(&self) -> FeedCapability;
    fn width_dots(&self) -> u32;
    fn max_height(&self) -> u32;
    /// Validate and encode completely before any transport is opened.
    fn plan_job(&self, request: &PrintRequest<'_>) -> Result<ExchangePlan, ProtocolError>;
}
