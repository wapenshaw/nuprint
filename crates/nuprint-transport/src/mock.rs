//! Scriptable in-memory transport for offline tests. Never touches Bluetooth.
//!
//! A [`MockDevice`] describes one fake peripheral and a write hook that answers each write with
//! notifications (or an error). [`MockTransport::log`] records what the code under test did:
//!
//! ```
//! use std::sync::Arc;
//! use std::time::Duration;
//!
//! use nuprint_transport::mock::{MockDevice, MockTransport};
//! use nuprint_transport::{
//!     Advertisement, CharProps, CharacteristicInfo, Endpoint, Notification, ServiceInfo, Transport,
//!     WriteMode, short_uuid,
//! };
//! use tokio_util::sync::CancellationToken;
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let service = short_uuid(0xAE30);
//! let write = Endpoint { service, characteristic: short_uuid(0xAE01) };
//! let notify = Endpoint { service, characteristic: short_uuid(0xAE02) };
//! let properties = CharProps { write: true, notify: true, ..CharProps::default() };
//!
//! let device = MockDevice {
//!     advertisement: Advertisement {
//!         backend_id: "mock-1".into(),
//!         local_name: Some("Mock Printer".into()),
//!         address: None,
//!         rssi: None,
//!         services: vec![],
//!         manufacturer_ids: vec![],
//!     },
//!     services: vec![ServiceInfo {
//!         uuid: service,
//!         characteristics: vec![
//!             CharacteristicInfo { uuid: write.characteristic, properties },
//!             CharacteristicInfo { uuid: notify.characteristic, properties },
//!         ],
//!     }],
//!     max_write: 20,
//!     // Every write is answered with one notification echoing its length.
//!     hook: Arc::new(move || {
//!         Box::new(move |_endpoint, data| Ok(vec![Notification { endpoint: notify, data: vec![data.len() as u8] }]))
//!     }),
//! };
//! let transport = MockTransport::new(vec![device]);
//!
//! let mut link = transport.connect("mock-1", Duration::from_secs(1), &CancellationToken::new()).await?;
//! let mut notifications = link.take_notifications().expect("first call returns the stream");
//! link.subscribe(notify).await?;
//! link.write(write, b"hello", WriteMode::WithoutResponse).await?;
//! assert_eq!(notifications.recv().await.unwrap().data, vec![5]);
//! link.disconnect().await?;
//!
//! let log = transport.log();
//! assert_eq!(log.written_bytes(), b"hello");
//! assert_eq!((log.connects, log.disconnects), (1, 1));
//! # Ok(())
//! # }
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    AdapterState, Advertisement, Endpoint, Link, Notification, ServiceInfo, Transport, TransportError, TransportKind,
    WriteMode,
};

/// Called for every write; returns notifications to deliver or an error to raise.
pub type WriteHook = Box<dyn FnMut(Endpoint, &[u8]) -> Result<Vec<Notification>, TransportError> + Send>;

#[derive(Clone)]
pub struct MockDevice {
    pub advertisement: Advertisement,
    pub services: Vec<ServiceInfo>,
    pub max_write: usize,
    pub hook: Arc<dyn Fn() -> WriteHook + Send + Sync>,
}

#[derive(Debug, Default, Clone)]
pub struct MockLog {
    pub scans: u32,
    pub connects: u32,
    pub disconnects: u32,
    pub subscriptions: Vec<Endpoint>,
    pub writes: Vec<(Endpoint, Vec<u8>, WriteMode)>,
}

impl MockLog {
    pub fn written_bytes(&self) -> Vec<u8> {
        self.writes.iter().flat_map(|(_, data, _)| data.iter().copied()).collect()
    }
}

#[derive(Clone)]
pub struct MockTransport {
    devices: Arc<Vec<MockDevice>>,
    state: AdapterState,
    log: Arc<Mutex<MockLog>>,
    injectors: Arc<Mutex<Vec<mpsc::Sender<Notification>>>>,
    connect_delay: Duration,
    discovery_delay: Duration,
    hang_connection_state: bool,
}

impl MockTransport {
    pub fn new(devices: Vec<MockDevice>) -> Self {
        Self {
            devices: Arc::new(devices),
            state: AdapterState::PoweredOn,
            log: Arc::default(),
            injectors: Arc::default(),
            connect_delay: Duration::ZERO,
            discovery_delay: Duration::ZERO,
            hang_connection_state: false,
        }
    }

    pub fn with_state(mut self, state: AdapterState) -> Self {
        self.state = state;
        self
    }

    /// Wait this long before connecting.
    pub fn with_connect_delay(mut self, delay: Duration) -> Self {
        self.connect_delay = delay;
        self
    }

    /// Connect, then spend this long discovering services: a stop during that time finds the
    /// device already connected, as on real hardware.
    pub fn with_discovery_delay(mut self, delay: Duration) -> Self {
        self.discovery_delay = delay;
        self
    }

    /// Links never answer whether they are still connected.
    pub fn with_hanging_connection_state(mut self) -> Self {
        self.hang_connection_state = true;
        self
    }

    /// A snapshot of everything the transport and its links have been asked to do.
    ///
    /// # Panics
    ///
    /// If a thread panicked while recording into the log, which means a test has already failed.
    pub fn log(&self) -> MockLog {
        self.log.lock().unwrap().clone()
    }

    /// Deliver an unsolicited notification to every open link (e.g. a delayed flow resume).
    ///
    /// # Panics
    ///
    /// If a thread panicked while opening or closing a link, which means a test has already failed.
    pub async fn inject(&self, notification: Notification) {
        let senders = self.injectors.lock().unwrap().clone();
        for sender in senders {
            let _ = sender.send(notification.clone()).await;
        }
    }

    fn check_state(&self) -> Result<(), TransportError> {
        match self.state {
            AdapterState::PoweredOff => Err(TransportError::AdapterOff),
            AdapterState::Absent => Err(TransportError::AdapterAbsent),
            AdapterState::PermissionDenied => Err(TransportError::PermissionDenied),
            _ => Ok(()),
        }
    }
}

#[async_trait]
impl Transport for MockTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::BleGatt
    }

    async fn adapter_state(&self) -> AdapterState {
        self.state
    }

    async fn scan(
        &self,
        _duration: Duration,
        sink: mpsc::Sender<Advertisement>,
        cancel: CancellationToken,
    ) -> Result<(), TransportError> {
        self.check_state()?;
        self.log.lock().unwrap().scans += 1;
        for device in self.devices.iter() {
            if cancel.is_cancelled() {
                break;
            }
            if sink.send(device.advertisement.clone()).await.is_err() {
                break;
            }
        }
        Ok(())
    }

    async fn connect(
        &self,
        backend_id: &str,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Box<dyn Link>, TransportError> {
        self.check_state()?;
        if !self.connect_delay.is_zero() {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(TransportError::Cancelled),
                waited = tokio::time::timeout(timeout, tokio::time::sleep(self.connect_delay)) => {
                    waited.map_err(|_| TransportError::Timeout("connection"))?;
                }
            }
        }
        let device = self
            .devices
            .iter()
            .find(|d| d.advertisement.backend_id == backend_id)
            .ok_or_else(|| TransportError::NotFound(backend_id.to_owned()))?;
        self.log.lock().unwrap().connects += 1;
        if !self.discovery_delay.is_zero() {
            let failed = tokio::select! {
                biased;
                _ = cancel.cancelled() => Some(TransportError::Cancelled),
                waited = tokio::time::timeout(timeout, tokio::time::sleep(self.discovery_delay)) => {
                    waited.err().map(|_| TransportError::Timeout("service discovery"))
                }
            };
            if let Some(error) = failed {
                // Connected already, so the attempt cleans up as the BLE backend does.
                self.log.lock().unwrap().disconnects += 1;
                return Err(error);
            }
        }
        let (tx, rx) = mpsc::channel(1024);
        self.injectors.lock().unwrap().push(tx.clone());
        Ok(Box::new(MockLink {
            backend_id: backend_id.to_owned(),
            services: device.services.clone(),
            max_write: device.max_write,
            hook: Mutex::new((device.hook)()),
            tx,
            rx: Some(rx),
            connected: Mutex::new(true),
            hang_connection_state: self.hang_connection_state,
            log: self.log.clone(),
        }))
    }
}

pub struct MockLink {
    backend_id: String,
    services: Vec<ServiceInfo>,
    max_write: usize,
    hook: Mutex<WriteHook>,
    tx: mpsc::Sender<Notification>,
    rx: Option<mpsc::Receiver<Notification>>,
    connected: Mutex<bool>,
    hang_connection_state: bool,
    log: Arc<Mutex<MockLog>>,
}

#[async_trait]
impl Link for MockLink {
    fn backend_id(&self) -> &str {
        &self.backend_id
    }

    fn services(&self) -> &[ServiceInfo] {
        &self.services
    }

    fn max_write_len(&self, _mode: WriteMode) -> usize {
        self.max_write
    }

    async fn subscribe(&self, endpoint: Endpoint) -> Result<(), TransportError> {
        self.log.lock().unwrap().subscriptions.push(endpoint);
        Ok(())
    }

    fn take_notifications(&mut self) -> Option<mpsc::Receiver<Notification>> {
        self.rx.take()
    }

    async fn write(&self, endpoint: Endpoint, data: &[u8], mode: WriteMode) -> Result<(), TransportError> {
        if !*self.connected.lock().unwrap() {
            return Err(TransportError::Disconnected);
        }
        if data.len() > self.max_write {
            return Err(TransportError::Backend(format!("write of {} exceeds {}", data.len(), self.max_write)));
        }
        self.log.lock().unwrap().writes.push((endpoint, data.to_vec(), mode));
        let reaction = (self.hook.lock().unwrap())(endpoint, data);
        match reaction {
            Ok(notifications) => {
                for n in notifications {
                    let _ = self.tx.try_send(n);
                }
                Ok(())
            }
            Err(TransportError::Disconnected) => {
                *self.connected.lock().unwrap() = false;
                Err(TransportError::Disconnected)
            }
            Err(error) => Err(error),
        }
    }

    async fn is_connected(&self) -> bool {
        if self.hang_connection_state {
            std::future::pending::<()>().await;
        }
        *self.connected.lock().unwrap()
    }

    async fn disconnect(&self) -> Result<(), TransportError> {
        *self.connected.lock().unwrap() = false;
        self.log.lock().unwrap().disconnects += 1;
        Ok(())
    }
}
