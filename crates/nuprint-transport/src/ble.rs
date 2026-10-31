//! BLE/GATT backend over `btleplug` (WinRT, CoreBluetooth, BlueZ).
//!
//! `btleplug` implements central-mode BLE only; it does not provide Classic/SPP.
//! Discovery results are filtered by the application (core), not here.

#[cfg(target_os = "linux")]
mod bluez;

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use btleplug::api::{
    Central, CentralEvent, CentralState, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter,
    WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral, PeripheralId};
use futures::StreamExt;
use tokio::sync::{OnceCell, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::{
    AdapterState, Advertisement, CONSERVATIVE_WRITE_LEN, CharProps, CharacteristicInfo, Endpoint, Link, Notification,
    ServiceInfo, Transport, TransportError, TransportKind, WriteMode,
};

/// On Windows, a scan started just after one that was stopped within its first 100 ms or so can
/// stay silent, with no error, until it is stopped again: on hardware it heard nothing from any
/// device for 30 s, where a working scan hears its first device within a second. A scan that has
/// heard no advertisement by this point is restarted.
const SILENT_SCAN_RESTART: Duration = Duration::from_secs(2);
const SCAN_RESTART_PAUSE: Duration = Duration::from_millis(250);

/// What one adapter event means to a scan.
#[derive(Debug, PartialEq)]
enum ScanEvent {
    /// Advertising data from a device: proof that the scan hears.
    Advertised(PeripheralId),
    AdapterOff,
    /// Connections, disconnections and other adapter changes. They also arrive on a silent scan,
    /// for example a disconnect that follows a cancelled connection, so they prove nothing.
    Other,
}

fn classify(event: CentralEvent) -> ScanEvent {
    match event {
        CentralEvent::DeviceDiscovered(id)
        | CentralEvent::DeviceUpdated(id)
        | CentralEvent::RssiUpdate { id, .. }
        | CentralEvent::ServicesAdvertisement { id, .. }
        | CentralEvent::ServiceDataAdvertisement { id, .. }
        | CentralEvent::ManufacturerDataAdvertisement { id, .. } => ScanEvent::Advertised(id),
        CentralEvent::StateUpdate(CentralState::PoweredOff) => ScanEvent::AdapterOff,
        _ => ScanEvent::Other,
    }
}

fn map_err(error: btleplug::Error) -> TransportError {
    match error {
        btleplug::Error::PermissionDenied => TransportError::PermissionDenied,
        btleplug::Error::DeviceNotFound => TransportError::NotFound("Device".into()),
        btleplug::Error::NotConnected => TransportError::Disconnected,
        btleplug::Error::TimedOut(_) => TransportError::Timeout("operation"),
        other => TransportError::Backend(other.to_string()),
    }
}

/// Whether the OS lets this app use Bluetooth. Only macOS asks per app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code, reason = "only macOS reports a refusal"))]
enum Permission {
    Allowed,
    NotAsked,
    Denied,
}

fn permission() -> Permission {
    cfg_select! {
        target_os = "macos" => {
            use objc2_core_bluetooth::{CBManager, CBManagerAuthorization};
            // SAFETY: a class property with no preconditions, available since macOS 10.15.
            match unsafe { CBManager::authorization_class() } {
                CBManagerAuthorization::AllowedAlways => Permission::Allowed,
                CBManagerAuthorization::NotDetermined => Permission::NotAsked,
                _ => Permission::Denied,
            }
        }
        _ => Permission::Allowed,
    }
}

/// How long to wait for the user to answer the macOS permission prompt.
const PERMISSION_ANSWER_WAIT: Duration = Duration::from_secs(30);
/// How long CoreBluetooth may take to report its state after it starts.
const ADAPTER_READY_WAIT: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct BleTransport {
    adapter: OnceCell<Adapter>,
    /// Backend IDs seen during discovery in this process. IDs are revalidated per session, so
    /// after a panic the cache is kept rather than poisoning every later scan or connect.
    known: Mutex<HashMap<String, PeripheralId>>,
}

impl BleTransport {
    pub fn new() -> Self {
        Self::default()
    }

    async fn adapter(&self) -> Result<&Adapter, TransportError> {
        self.adapter
            .get_or_try_init(|| async {
                let manager = Manager::new().await.map_err(map_err)?;
                let adapters = manager.adapters().await.map_err(map_err)?;
                adapters.into_iter().next().ok_or(TransportError::AdapterAbsent)
            })
            .await
    }

    /// The adapter once it can be used. On macOS, opening it the first time shows the permission
    /// prompt, and a refused or unanswered prompt only shows as an unknown state; wait for the
    /// answer so a scan never runs, and silently finds nothing, without permission.
    async fn ensure_powered(&self) -> Result<&Adapter, TransportError> {
        if permission() == Permission::Denied {
            return Err(TransportError::PermissionDenied);
        }
        let adapter = self.adapter().await?;
        let started = tokio::time::Instant::now();
        loop {
            let permission = permission();
            match (permission, adapter.adapter_state().await) {
                (Permission::Denied, _) | (_, Err(btleplug::Error::PermissionDenied)) => {
                    return Err(TransportError::PermissionDenied);
                }
                (_, Ok(CentralState::PoweredOff)) => return Err(TransportError::AdapterOff),
                (_, Ok(CentralState::PoweredOn)) => return Ok(adapter),
                // Without a prompt to wait for, keep the backend's answer as it is.
                _ if !cfg!(target_os = "macos") => return Ok(adapter),
                (Permission::NotAsked, _) if started.elapsed() >= PERMISSION_ANSWER_WAIT => {
                    return Err(TransportError::PermissionPending);
                }
                (Permission::Allowed, _) if started.elapsed() >= ADAPTER_READY_WAIT => return Ok(adapter),
                _ => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
    }

    async fn advertisement(&self, adapter: &Adapter, id: &PeripheralId) -> Option<Advertisement> {
        let peripheral = adapter.peripheral(id).await.ok()?;
        let props = peripheral.properties().await.ok()??;
        let backend_id = id.to_string();
        self.known.lock().unwrap_or_else(PoisonError::into_inner).insert(backend_id.clone(), id.clone());
        let mut manufacturer_ids: Vec<u16> = props.manufacturer_data.keys().copied().collect();
        manufacturer_ids.sort_unstable();
        let address = if cfg!(target_os = "macos") { None } else { Some(props.address.to_string()) };
        Some(Advertisement {
            backend_id,
            local_name: props.local_name.or(props.advertisement_name),
            address,
            rssi: props.rssi,
            services: props.services,
            manufacturer_ids,
        })
    }

    async fn find_peripheral(&self, adapter: &Adapter, backend_id: &str) -> Result<Peripheral, TransportError> {
        let known = self.known.lock().unwrap_or_else(PoisonError::into_inner).get(backend_id).cloned();
        if let Some(id) = known
            && let Ok(peripheral) = adapter.peripheral(&id).await
        {
            return Ok(peripheral);
        }
        let peripherals = adapter.peripherals().await.map_err(map_err)?;
        peripherals
            .into_iter()
            .find(|p| p.id().to_string() == backend_id)
            .ok_or_else(|| TransportError::NotFound(backend_id.to_owned()))
    }
}

#[async_trait]
impl Transport for BleTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::BleGatt
    }

    async fn adapter_state(&self) -> AdapterState {
        match permission() {
            Permission::Denied => return AdapterState::PermissionDenied,
            // Opening the adapter makes macOS ask, so leave that until the user looks for printers.
            Permission::NotAsked if !self.adapter.initialized() => return AdapterState::PermissionNotAsked,
            Permission::NotAsked => return AdapterState::PermissionPending,
            Permission::Allowed => {}
        }
        let adapter = match self.adapter().await {
            Ok(adapter) => adapter,
            Err(TransportError::PermissionDenied) => return AdapterState::PermissionDenied,
            Err(TransportError::AdapterAbsent) => return AdapterState::Absent,
            Err(_) => return AdapterState::Unknown,
        };
        match adapter.adapter_state().await {
            Ok(CentralState::PoweredOn) => AdapterState::PoweredOn,
            Ok(CentralState::PoweredOff) => AdapterState::PoweredOff,
            Ok(_) => AdapterState::Unknown,
            Err(btleplug::Error::PermissionDenied) => AdapterState::PermissionDenied,
            Err(_) => AdapterState::Unknown,
        }
    }

    async fn scan(
        &self,
        duration: Duration,
        sink: mpsc::Sender<Advertisement>,
        cancel: CancellationToken,
    ) -> Result<(), TransportError> {
        let adapter = self.ensure_powered().await?;
        let mut events = adapter.events().await.map_err(map_err)?;
        adapter.start_scan(ScanFilter::default()).await.map_err(map_err)?;
        let deadline = tokio::time::sleep(duration);
        tokio::pin!(deadline);
        let silent = tokio::time::sleep(SILENT_SCAN_RESTART);
        tokio::pin!(silent);
        let mut heard = !cfg!(windows);
        let result = loop {
            tokio::select! {
                _ = &mut deadline => break Ok(()),
                _ = cancel.cancelled() => break Ok(()),
                _ = &mut silent, if !heard => {
                    warn!("no advertisements in {SILENT_SCAN_RESTART:?}; restarting the scan");
                    if let Err(error) = adapter.stop_scan().await {
                        warn!("stop_scan failed: {error}");
                    }
                    tokio::time::sleep(SCAN_RESTART_PAUSE).await;
                    if let Err(error) = adapter.start_scan(ScanFilter::default()).await {
                        break Err(map_err(error));
                    }
                    silent.as_mut().reset(tokio::time::Instant::now() + SILENT_SCAN_RESTART);
                }
                event = events.next() => {
                    let Some(event) = event else { break Ok(()) };
                    let id = match classify(event) {
                        ScanEvent::Advertised(id) => id,
                        ScanEvent::AdapterOff => break Err(TransportError::AdapterOff),
                        ScanEvent::Other => continue,
                    };
                    heard = true;
                    if let Some(adv) = self.advertisement(adapter, &id).await
                        && sink.send(adv).await.is_err()
                    {
                        break Ok(());
                    }
                }
            }
        };
        if let Err(error) = adapter.stop_scan().await {
            warn!("stop_scan failed: {error}");
        }
        result
    }

    async fn connect(
        &self,
        backend_id: &str,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Box<dyn Link>, TransportError> {
        let find = async {
            let adapter = self.ensure_powered().await?;
            self.find_peripheral(adapter, backend_id).await
        };
        // Nothing is connected yet, so stopping here needs no cleanup.
        let peripheral = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(TransportError::Cancelled),
            found = find => found?,
        };
        // Once `connect` has been called the device may be connected whatever happens next, even
        // when the call timed out, so every failure from here on disconnects before returning.
        let open = async {
            tokio::time::timeout(timeout, connect_peripheral(&peripheral, timeout))
                .await
                .map_err(|_| TransportError::Timeout("connection"))??;
            BleLink::open(backend_id.to_owned(), peripheral.clone(), timeout).await
        };
        let opened = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(TransportError::Cancelled),
            opened = open => opened,
        };
        match opened {
            Ok(link) => Ok(Box::new(link)),
            Err(error) => {
                let _ = tokio::time::timeout(Duration::from_secs(10), disconnect_peripheral(&peripheral)).await;
                Err(error)
            }
        }
    }
}

#[cfg(target_os = "linux")]
async fn connect_peripheral(peripheral: &Peripheral, timeout: Duration) -> Result<(), TransportError> {
    let bluez::LeConnect::Unavailable { too_old } = bluez::connect_le(&peripheral.id().to_string(), timeout).await?
    else {
        return Ok(());
    };
    match peripheral.connect().await.map_err(map_err) {
        Err(error) if bluez::chose_classic(&error) => Err(bluez::classic_error(too_old)),
        result => result,
    }
}

#[cfg(target_os = "linux")]
async fn disconnect_peripheral(peripheral: &Peripheral) -> Result<(), TransportError> {
    bluez::disconnect_le(&peripheral.id().to_string()).await;
    peripheral.disconnect().await.map_err(map_err)
}

#[cfg(not(target_os = "linux"))]
async fn disconnect_peripheral(peripheral: &Peripheral) -> Result<(), TransportError> {
    peripheral.disconnect().await.map_err(map_err)
}

#[cfg(not(target_os = "linux"))]
async fn connect_peripheral(peripheral: &Peripheral, _timeout: Duration) -> Result<(), TransportError> {
    peripheral.connect().await.map_err(map_err)
}

pub struct BleLink {
    backend_id: String,
    peripheral: Peripheral,
    services: Vec<ServiceInfo>,
    characteristics: HashMap<Endpoint, Characteristic>,
    notifications: Option<mpsc::Receiver<Notification>>,
    forwarder: JoinHandle<()>,
}

impl BleLink {
    async fn open(backend_id: String, peripheral: Peripheral, timeout: Duration) -> Result<Self, TransportError> {
        tokio::time::timeout(timeout, peripheral.discover_services())
            .await
            .map_err(|_| TransportError::Timeout("service discovery"))?
            .map_err(map_err)?;
        let mut characteristics = HashMap::new();
        let mut services: Vec<ServiceInfo> = peripheral
            .services()
            .into_iter()
            .map(|service| {
                let mut chars: Vec<CharacteristicInfo> = service
                    .characteristics
                    .iter()
                    .map(|c| {
                        characteristics.insert(Endpoint { service: service.uuid, characteristic: c.uuid }, c.clone());
                        CharacteristicInfo {
                            uuid: c.uuid,
                            properties: CharProps {
                                read: c.properties.contains(CharPropFlags::READ),
                                write: c.properties.contains(CharPropFlags::WRITE),
                                write_without_response: c.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE),
                                notify: c.properties.contains(CharPropFlags::NOTIFY),
                                indicate: c.properties.contains(CharPropFlags::INDICATE),
                            },
                        }
                    })
                    .collect();
                chars.sort_by_key(|c| c.uuid);
                ServiceInfo { uuid: service.uuid, characteristics: chars }
            })
            .collect();
        services.sort_by_key(|s| s.uuid);
        let mut stream = peripheral.notifications().await.map_err(map_err)?;
        let (tx, rx) = mpsc::channel(512);
        let forwarder = tokio::spawn(async move {
            while let Some(n) = stream.next().await {
                let notification = Notification {
                    endpoint: Endpoint { service: n.service_uuid, characteristic: n.uuid },
                    data: n.value,
                };
                if tx.send(notification).await.is_err() {
                    break;
                }
            }
            debug!("notification stream ended");
        });
        Ok(Self { backend_id, peripheral, services, characteristics, notifications: Some(rx), forwarder })
    }

    fn characteristic(&self, endpoint: Endpoint) -> Result<&Characteristic, TransportError> {
        self.characteristics.get(&endpoint).ok_or_else(|| TransportError::EndpointMissing(endpoint.to_string()))
    }
}

impl Drop for BleLink {
    fn drop(&mut self) {
        self.forwarder.abort();
    }
}

#[async_trait]
impl Link for BleLink {
    fn backend_id(&self) -> &str {
        &self.backend_id
    }

    fn services(&self) -> &[ServiceInfo] {
        &self.services
    }

    fn max_write_len(&self, _mode: WriteMode) -> usize {
        // ATT payload = MTU - 3. btleplug reports the default (23) until negotiation is known;
        // on Linux it reads BlueZ's per-characteristic `MTU` property, so a BlueZ that does not
        // expose one (older versions) also lands on the conservative size rather than guessing.
        let mtu = usize::from(self.peripheral.mtu());
        if mtu > 23 { mtu - 3 } else { CONSERVATIVE_WRITE_LEN }
    }

    async fn subscribe(&self, endpoint: Endpoint) -> Result<(), TransportError> {
        let characteristic = self.characteristic(endpoint)?;
        self.peripheral.subscribe(characteristic).await.map_err(map_err)
    }

    fn take_notifications(&mut self) -> Option<mpsc::Receiver<Notification>> {
        self.notifications.take()
    }

    async fn write(&self, endpoint: Endpoint, data: &[u8], mode: WriteMode) -> Result<(), TransportError> {
        let characteristic = self.characteristic(endpoint)?;
        let write_type = match mode {
            WriteMode::WithResponse => WriteType::WithResponse,
            WriteMode::WithoutResponse => WriteType::WithoutResponse,
        };
        self.peripheral.write(characteristic, data, write_type).await.map_err(map_err)
    }

    async fn is_connected(&self) -> bool {
        self.peripheral.is_connected().await.unwrap_or(false)
    }

    async fn disconnect(&self) -> Result<(), TransportError> {
        self.forwarder.abort();
        disconnect_peripheral(&self.peripheral).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_state_changes_are_not_advertisements() {
        assert_eq!(classify(CentralEvent::StateUpdate(CentralState::PoweredOn)), ScanEvent::Other);
        assert_eq!(classify(CentralEvent::StateUpdate(CentralState::Unknown)), ScanEvent::Other);
        assert_eq!(classify(CentralEvent::StateUpdate(CentralState::PoweredOff)), ScanEvent::AdapterOff);
    }

    /// Only Windows restarts a silent scan, and only there can a test build a device id.
    #[cfg(windows)]
    #[test]
    fn only_advertisements_stop_a_silent_scan_being_restarted() {
        let id = PeripheralId::from(btleplug::api::BDAddr::from([1, 2, 3, 4, 5, 6]));
        for event in [CentralEvent::DeviceConnected(id.clone()), CentralEvent::DeviceDisconnected(id.clone())] {
            assert_eq!(classify(event), ScanEvent::Other);
        }
        for event in [
            CentralEvent::DeviceDiscovered(id.clone()),
            CentralEvent::DeviceUpdated(id.clone()),
            CentralEvent::RssiUpdate { id: id.clone(), rssi: -60 },
            CentralEvent::ServicesAdvertisement { id: id.clone(), services: vec![] },
            CentralEvent::ServiceDataAdvertisement { id: id.clone(), service_data: HashMap::new() },
            CentralEvent::ManufacturerDataAdvertisement { id: id.clone(), manufacturer_data: HashMap::new() },
        ] {
            assert_eq!(classify(event), ScanEvent::Advertised(id.clone()));
        }
    }
}
