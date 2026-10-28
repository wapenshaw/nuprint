//! NuPrint application service shared by the Tauri desktop host and the CLI.
//!
//! Owns profile resolution, printer sessions, durable jobs and storage. It does not
//! depend on Tauri, a WebView or OS-specific APIs; transports supply Bluetooth.

pub mod api;
pub mod error;
pub mod jobs;
pub mod profile;
pub mod service;
pub mod session;
pub mod store;

pub use error::{AppError, AppResult, ErrorCode};
pub use service::{AppService, EventSink, ServiceConfig};
