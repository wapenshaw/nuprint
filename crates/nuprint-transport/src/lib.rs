//! How bytes move: discovery, connection, endpoint writes and notifications.
//!
//! Transports never interpret vendor protocol bytes, heat values or status bits
//! (docs/printer-support.md). BLE/GATT through `btleplug` is the first backend;
//! Classic/SPP or USB would be separate implementations of the same traits.

#![warn(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
pub use uuid::Uuid;

pub mod ble;
pub mod mock;

/// Expand a 16-bit Bluetooth SIG short UUID (e.g. `0xAE30`).
pub const fn short_uuid(short: u16) -> Uuid {
    Uuid::from_u128(((short as u128) << 96) | 0x0000_0000_0000_1000_8000_0080_5f9b_34fb)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    BleGatt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterState {
    PoweredOn,
    PoweredOff,
    Absent,
    PermissionDenied,
    /// macOS has not asked yet; it asks the first time Bluetooth is used.
    PermissionNotAsked,
    /// macOS has asked and the user has not answered yet.
    PermissionPending,
    Unknown,
}

/// One discovery observation. `backend_id` is OS/backend-scoped: a MAC-like address on
/// Windows/Linux, an opaque UUID on macOS. It is not a portable identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Advertisement {
    pub backend_id: String,
    pub local_name: Option<String>,
    /// Hardware address where the OS exposes one (not on macOS).
    pub address: Option<String>,
    pub rssi: Option<i16>,
    pub services: Vec<Uuid>,
    pub manufacturer_ids: Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct Endpoint {
    pub service: Uuid,
    pub characteristic: Uuid,
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.service, self.characteristic)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct CharProps {
    pub read: bool,
    pub write: bool,
    pub write_without_response: bool,
    pub notify: bool,
    pub indicate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CharacteristicInfo {
    pub uuid: Uuid,
    pub properties: CharProps,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServiceInfo {
    pub uuid: Uuid,
    pub characteristics: Vec<CharacteristicInfo>,
}

impl ServiceInfo {
    pub fn characteristic(&self, uuid: Uuid) -> Option<&CharacteristicInfo> {
        self.characteristics.iter().find(|c| c.uuid == uuid)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteMode {
    WithResponse,
    WithoutResponse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub endpoint: Endpoint,
    pub data: Vec<u8>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TransportError {
    #[error("No Bluetooth adapter was found on this computer")]
    AdapterAbsent,
    #[error("Bluetooth is turned off")]
    AdapterOff,
    #[error("Bluetooth permission was denied for NuPrint")]
    PermissionDenied,
    #[error("Bluetooth permission has not been granted to NuPrint yet")]
    PermissionPending,
    #[error("{0} was not found nearby")]
    NotFound(String),
    #[error("Bluetooth {0} timed out")]
    Timeout(&'static str),
    #[error("Bluetooth disconnected")]
    Disconnected,
    #[error("Bluetooth operation cancelled")]
    Cancelled,
    #[error("Endpoint {0} is not available on this device")]
    EndpointMissing(String),
    /// BlueZ connected a dual-mode device over Classic, so its GATT services never appeared.
    #[error("BlueZ tried Classic Bluetooth instead of LE and its LE connection is unavailable")]
    ClassicConnection,
    #[error("Bluetooth error: {0}")]
    Backend(String),
}

/// Discovery and connection. Implementations must be safe to share across tasks.
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    fn kind(&self) -> TransportKind;

    async fn adapter_state(&self) -> AdapterState;

    /// Stream advertisements into `sink` until `duration` elapses or `cancel` fires.
    /// Repeated observations of one device may be sent; consumers de-duplicate.
    ///
    /// # Errors
    ///
    /// Adapter and permission errors as for [`Transport::connect`]. Cancellation is a normal
    /// end of the scan, not an error.
    async fn scan(
        &self,
        duration: Duration,
        sink: mpsc::Sender<Advertisement>,
        cancel: CancellationToken,
    ) -> Result<(), TransportError>;

    /// Connect and enumerate services. The caller owns the link exclusively and must
    /// call [`Link::disconnect`] in every outcome.
    ///
    /// `cancel` stops the attempt with [`TransportError::Cancelled`]. A device can already be
    /// connected while its services are still being discovered, so every failure, a timeout or
    /// cancellation included, disconnects before this returns. Await it to the end rather than
    /// dropping it, so the device is free again when it returns.
    ///
    /// # Errors
    ///
    /// Adapter and permission errors ([`TransportError::AdapterAbsent`],
    /// [`TransportError::AdapterOff`], [`TransportError::PermissionDenied`]),
    /// [`TransportError::NotFound`] for an unknown device, [`TransportError::Timeout`],
    /// [`TransportError::Cancelled`], and [`TransportError::Backend`] for other failures.
    async fn connect(
        &self,
        backend_id: &str,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Box<dyn Link>, TransportError>;
}

/// One connected session with a device.
#[async_trait]
pub trait Link: Send + Sync {
    fn backend_id(&self) -> &str;

    fn services(&self) -> &[ServiceInfo];

    /// Largest single write the backend will accept for `mode`. Backends without
    /// negotiated-size information report a conservative 20 bytes.
    fn max_write_len(&self, mode: WriteMode) -> usize;

    async fn subscribe(&self, endpoint: Endpoint) -> Result<(), TransportError>;

    /// The notification stream for this link; returns `None` after the first call.
    fn take_notifications(&mut self) -> Option<mpsc::Receiver<Notification>>;

    /// Write `data` to `endpoint`; it must not exceed [`Self::max_write_len`] for `mode`.
    ///
    /// # Errors
    ///
    /// [`TransportError::Disconnected`] once the link is gone, [`TransportError::EndpointMissing`]
    /// for an unknown characteristic, and [`TransportError::Backend`] for other failures,
    /// including an oversized write. A successful write only means the bytes were handed to the
    /// radio, not that the device acted on them.
    async fn write(&self, endpoint: Endpoint, data: &[u8], mode: WriteMode) -> Result<(), TransportError>;

    async fn is_connected(&self) -> bool;

    async fn disconnect(&self) -> Result<(), TransportError>;
}

/// The ATT default minus the 3-byte header.
pub const CONSERVATIVE_WRITE_LEN: usize = 20;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_uuid_expands() {
        assert_eq!(short_uuid(0xAE30).to_string(), "0000ae30-0000-1000-8000-00805f9b34fb");
        assert_eq!(short_uuid(0xAF30).to_string(), "0000af30-0000-1000-8000-00805f9b34fb");
    }
}
