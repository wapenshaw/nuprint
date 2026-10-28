//! Pure printer protocol code: canonical rasters, framing, codecs and exchange plans.
//!
//! Nothing in this crate performs I/O, so every behavior is testable without Bluetooth.

pub mod bitmap;
pub mod driver;
pub mod iprint;

pub use bitmap::{BitmapError, MonoBitmap};
pub use driver::{
    CompletionPolicy, Density, DeviceInfo, DiagnosticQuery, ExchangePlan, FeedCapability, NotificationDecoder,
    PlanStep, PrintRequest, PrinterDriver, ProtocolError, ProtocolEvent, StatusFlag, StatusReport,
};
