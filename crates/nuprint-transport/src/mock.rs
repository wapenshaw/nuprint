//! Scriptable in-memory transport for offline tests. Never touches Bluetooth.

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
}

impl MockTransport {
    pub fn new(devices: Vec<MockDevice>) -> Self {
        Self {
            devices: Arc::new(devices),
            state: AdapterState::PoweredOn,
            log: Arc::default(),
            injectors: Arc::default(),
            connect_delay: Duration::ZERO,
        }
    }

    pub fn with_state(mut self, state: AdapterState) -> Self {
        self.state = state;
        self
    }

    pub fn with_connect_delay(mut self, delay: Duration) -> Self {
        self.connect_delay = delay;
        self
    }

    pub fn log(&self) -> MockLog {
        self.log.lock().unwrap().clone()
    }

    /// Deliver an unsolicited notification to every open link (e.g. a delayed flow resume).
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

    async fn connect(&self, backend_id: &str, timeout: Duration) -> Result<Box<dyn Link>, TransportError> {
        self.check_state()?;
        if !self.connect_delay.is_zero() {
            tokio::time::timeout(timeout, tokio::time::sleep(self.connect_delay))
                .await
                .map_err(|_| TransportError::Timeout("connection"))?;
        }
        let device = self
            .devices
            .iter()
            .find(|d| d.advertisement.backend_id == backend_id)
            .ok_or_else(|| TransportError::NotFound(backend_id.to_owned()))?;
        self.log.lock().unwrap().connects += 1;
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
        *self.connected.lock().unwrap()
    }

    async fn disconnect(&self) -> Result<(), TransportError> {
        *self.connected.lock().unwrap() = false;
        self.log.lock().unwrap().disconnects += 1;
        Ok(())
    }
}
