//! NuPrint application service shared by the Tauri desktop host and the CLI.
//!
//! Owns profile resolution, printer sessions, durable jobs and storage. It does not
//! depend on Tauri, a WebView or OS-specific APIs; transports supply Bluetooth.
//!
//! Start with [`AppService::start`] (needs a Tokio runtime) and shut it down with
//! [`AppService::shutdown`]. Three contracts are worth reading first:
//!
//! - **Profile revisions are immutable.** A job names the exact profile revision it was
//!   rendered and planned for; see [`profile::ProfileRegistry::get_revision`].
//! - **Cancellation has one owner per phase.** The store owns jobs that have not started, the
//!   dispatcher owns the one active job; see [`AppService::cancel_job`].
//! - **Confirmed and at-risk bytes differ.** `bytes_sent` counts confirmed writes, but whether a
//!   failure is uncertain depends on bytes that may have arrived; see [`jobs::classify`].

pub mod api;
pub mod error;
pub mod jobs;
mod log_writer;
pub mod profile;
pub mod service;
pub mod session;
pub mod store;

pub use error::{AppError, AppResult, ErrorCode};
pub use service::{AppService, EventSink, ServiceConfig, ShutdownOutcome};
