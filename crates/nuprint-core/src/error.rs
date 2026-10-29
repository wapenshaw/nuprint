//! Stable error categories (docs/technical-spec.md §8). Codes are part of the UI/CLI
//! contract; messages are readable recovery guidance; `detail` carries diagnostics.

use nuprint_protocol::{ProtocolError, StatusFlag};
use nuprint_render::RenderError;
use nuprint_transport::TransportError;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    PermissionDenied,
    AdapterAbsent,
    AdapterOff,
    DeviceNotFound,
    ServiceMismatch,
    UnsupportedProfile,
    AmbiguousProfile,
    ExperimentalNotAcknowledged,
    ConnectionFailed,
    StatusTimeout,
    PaperOut,
    CoverOpen,
    Overheated,
    LowPower,
    PrinterBusy,
    BufferTimeout,
    Disconnected,
    RenderInvalid,
    PdfUnavailable,
    PdfPasswordRequired,
    StalePreview,
    StorageFull,
    QueueFull,
    /// Another NuPrint already owns this data folder (and its job queue).
    DataDirInUse,
    OutcomeUncertain,
    Cancelled,
    InvalidInput,
    NotFound,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, thiserror::Error)]
#[error("{message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), detail: None }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    pub fn not_found(what: impl std::fmt::Display) -> Self {
        Self::new(ErrorCode::NotFound, format!("{what} was not found"))
    }

    pub fn cancelled() -> Self {
        Self::new(ErrorCode::Cancelled, "Cancelled")
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        Self::new(ErrorCode::Internal, "Something went wrong inside NuPrint").with_detail(error.to_string())
    }

    /// A blocking printer fault reported by status.
    pub fn from_fault(flag: StatusFlag) -> Self {
        let (code, message) = match flag {
            StatusFlag::PaperOut => (ErrorCode::PaperOut, "Printer is out of paper. Load paper, then try again."),
            StatusFlag::CoverOpen => (ErrorCode::CoverOpen, "Printer cover is open. Close it, then try again."),
            StatusFlag::Overheated => {
                (ErrorCode::Overheated, "Print head is too hot. Let the printer cool down before printing again.")
            }
            StatusFlag::LowPower => (ErrorCode::LowPower, "Printer battery is low. Charge it before printing."),
            StatusFlag::Printing | StatusFlag::Charging => {
                (ErrorCode::PrinterBusy, "Printer is already printing. Wait for it to finish.")
            }
        };
        Self::new(code, message)
    }
}

impl From<TransportError> for AppError {
    fn from(error: TransportError) -> Self {
        let detail = error.to_string();
        let (code, message) = match &error {
            TransportError::AdapterAbsent => {
                (ErrorCode::AdapterAbsent, "No Bluetooth adapter was found on this computer.")
            }
            TransportError::AdapterOff => (ErrorCode::AdapterOff, "Bluetooth is turned off. Turn it on and try again."),
            TransportError::PermissionDenied => (
                ErrorCode::PermissionDenied,
                "NuPrint is not allowed to use Bluetooth. Grant Bluetooth permission in system settings.",
            ),
            TransportError::PermissionPending => (
                ErrorCode::PermissionDenied,
                "NuPrint does not have Bluetooth access yet. If macOS is asking, choose Allow; otherwise allow \
                 NuPrint in Bluetooth privacy settings. Then find printers again.",
            ),
            TransportError::NotFound(_) => (
                ErrorCode::DeviceNotFound,
                "Printer was not found. Turn it on, keep it near the computer and close the phone app.",
            ),
            TransportError::Timeout(_) => (
                ErrorCode::ConnectionFailed,
                "The printer did not respond in time. Switch it off and on, close other printer apps and retry.",
            ),
            TransportError::Disconnected => (
                ErrorCode::Disconnected,
                "Bluetooth disconnected. Some paper may already have printed; check before retrying.",
            ),
            TransportError::Cancelled => return AppError::cancelled(),
            TransportError::EndpointMissing(_) => {
                (ErrorCode::ServiceMismatch, "This device does not offer the print service its profile requires.")
            }
            TransportError::Backend(_) => (ErrorCode::ConnectionFailed, "Bluetooth connection failed."),
        };
        AppError::new(code, message).with_detail(detail)
    }
}

impl From<ProtocolError> for AppError {
    fn from(error: ProtocolError) -> Self {
        AppError::new(ErrorCode::InvalidInput, error.to_string())
    }
}

impl From<RenderError> for AppError {
    fn from(error: RenderError) -> Self {
        let code = match error {
            RenderError::PdfUnavailable(_) => ErrorCode::PdfUnavailable,
            RenderError::PdfPasswordRequired => ErrorCode::PdfPasswordRequired,
            _ => ErrorCode::RenderInvalid,
        };
        AppError::new(code, error.to_string())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(error: rusqlite::Error) -> Self {
        if let rusqlite::Error::SqliteFailure(e, _) = &error
            && e.code == rusqlite::ErrorCode::DiskFull
        {
            return AppError::new(ErrorCode::StorageFull, "The disk is full; free space and try again.");
        }
        AppError::internal(error)
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::StorageFull {
            return AppError::new(ErrorCode::StorageFull, "The disk is full; free space and try again.");
        }
        AppError::internal(error)
    }
}
