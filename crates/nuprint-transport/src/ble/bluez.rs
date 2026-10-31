//! LE connections on BlueZ.
//!
//! btleplug connects through `org.bluez.Device1.Connect`, which picks the bearer itself. For a
//! dual-mode device, such as an SC03h that also offers Classic serial and audio profiles on the
//! same address, BlueZ picks Classic, finds no profile it can use and fails with "No more profiles
//! to connect to", so the GATT print service never appears (observed on BlueZ 5.87).
//!
//! BlueZ offers these only for a device it has heard over LE. A scan that also runs Classic inquiry,
//! as btleplug's does, can find the printer over Classic first, so an LE-only discovery runs
//! before connecting when BlueZ has not yet heard it over LE.
//!
//! BlueZ can be asked for LE in two ways, both offered only when its experimental features are
//! enabled (`Experimental = true` in `/etc/bluetooth/main.conf`): `org.bluez.Bearer.LE1.Connect`
//! from BlueZ 5.84, and the `PreferredBearer` device property from 5.80 (such as Ubuntu 25.10's
//! 5.83). Older BlueZ, such as Ubuntu 24.04's 5.72, has neither, so when neither is offered the
//! version decides whether to advise turning experimental features on or to say this BlueZ is too
//! old. The property is stored by BlueZ, so it is set only for the connect call and then restored.

use std::time::{Duration, Instant};

use dbus::arg::{PropMap, Variant};
use dbus::blocking::stdintf::org_freedesktop_dbus::Properties as _;
use dbus::blocking::{Connection, Proxy};
use tracing::{debug, warn};

use crate::TransportError;

const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const LE_BEARER: &str = "org.bluez.Bearer.LE1";
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RESOLVE_POLL: Duration = Duration::from_millis(100);
/// How long to listen for the printer's LE advertisement before connecting anyway.
const LE_DISCOVERY: Duration = Duration::from_secs(5);

/// What [`connect_le`] did.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum LeConnect {
    /// Connected over LE with services resolved.
    Connected,
    /// This BlueZ offers no way to ask for LE, so nothing was attempted. Carries BlueZ's version
    /// when it is older than [`FIRST_LE`] and so has no way to offer one at all.
    Unavailable { too_old: Option<Version> },
}

/// A BlueZ release, such as 5.72.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Version {
    major: u8,
    minor: u8,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// The first BlueZ that can be asked for LE: it added `PreferredBearer`.
const FIRST_LE: Version = Version { major: 5, minor: 80 };

/// Connect `device` (a btleplug peripheral id, the object path below `/org/bluez/`) over LE and
/// wait until BlueZ has resolved its services, as btleplug's own connect does.
///
/// # Errors
///
/// [`TransportError::Backend`] when BlueZ refuses the connection or D-Bus is unavailable, and
/// [`TransportError::Timeout`] when the services are not resolved within `timeout`.
pub(super) async fn connect_le(device: &str, timeout: Duration) -> Result<LeConnect, TransportError> {
    let path = format!("/org/bluez/{device}");
    tokio::task::spawn_blocking(move || connect_le_blocking(&path, timeout))
        .await
        .map_err(|error| TransportError::Backend(error.to_string()))?
}

fn connect_le_blocking(path: &str, timeout: Duration) -> Result<LeConnect, TransportError> {
    let deadline = Instant::now() + timeout;
    let connection = Connection::new_system().map_err(backend)?;
    let device = connection.with_proxy(BLUEZ, path, timeout);
    if !heard_over_le(&device) {
        discover_le(&connection, path, &device, timeout);
    }
    if !connect_bearer(&device)? && !connect_preferring_le(&device)? {
        let too_old = bluez_version(&connection, path, timeout).filter(|version| *version < FIRST_LE);
        debug!(?too_old, "BlueZ offers no LE connection");
        return Ok(LeConnect::Unavailable { too_old });
    }
    loop {
        if device.get::<bool>(DEVICE, "ServicesResolved").map_err(backend)? {
            return Ok(LeConnect::Connected);
        }
        if Instant::now() >= deadline {
            return Err(TransportError::Timeout("service discovery"));
        }
        std::thread::sleep(RESOLVE_POLL);
    }
}

/// Whether BlueZ has heard the device over LE: it then has the LE bearer interface.
fn heard_over_le(device: &Proxy<'_, &Connection>) -> bool {
    device.get::<bool>(LE_BEARER, "Connected").is_ok()
}

/// Run an LE-only discovery until BlueZ hears the device over LE or [`LE_DISCOVERY`] passes.
/// Discovery filters belong to the D-Bus client that sets them, so btleplug's scans are unaffected.
fn discover_le(connection: &Connection, path: &str, device: &Proxy<'_, &Connection>, timeout: Duration) {
    let Some((adapter_path, _)) = path.rsplit_once('/') else { return };
    let adapter = connection.with_proxy(BLUEZ, adapter_path, timeout);
    let mut filter = PropMap::new();
    filter.insert("Transport".to_owned(), Variant(Box::new("le".to_owned())));
    if let Err(error) = adapter.method_call::<(), _, _, _>(ADAPTER, "SetDiscoveryFilter", (filter,)) {
        warn!("could not limit discovery to LE: {error}");
        return;
    }
    if let Err(error) = adapter.method_call::<(), _, _, _>(ADAPTER, "StartDiscovery", ()) {
        warn!("could not start LE discovery: {error}");
        return;
    }
    let deadline = Instant::now() + LE_DISCOVERY;
    while !heard_over_le(device) && Instant::now() < deadline {
        std::thread::sleep(RESOLVE_POLL);
    }
    debug!(heard = heard_over_le(device), "LE discovery finished");
    if let Err(error) = adapter.method_call::<(), _, _, _>(ADAPTER, "StopDiscovery", ()) {
        debug!("stopping LE discovery: {error}");
    }
}

/// BlueZ's version, from the adapter's `Modalias`. `None` when it cannot be read or `DeviceID` in
/// `main.conf` replaced BlueZ's own device ID.
fn bluez_version(connection: &Connection, path: &str, timeout: Duration) -> Option<Version> {
    let (adapter_path, _) = path.rsplit_once('/')?;
    let modalias = connection.with_proxy(BLUEZ, adapter_path, timeout).get::<String>(ADAPTER, "Modalias").ok()?;
    parse_modalias(&modalias)
}

/// BlueZ's default device ID is vendor 1D6B (Linux Foundation), product 0246 (BlueZ) and its
/// version as `major << 8 | minor`, so 5.72 is `usb:v1D6Bp0246d0548`.
fn parse_modalias(modalias: &str) -> Option<Version> {
    let (_, id) = modalias.split_once(':')?;
    let id = id.to_ascii_uppercase();
    let version = id.strip_prefix("V1D6BP0246D").filter(|version| version.len() == 4)?;
    let [major, minor] = u16::from_str_radix(version, 16).ok()?.to_be_bytes();
    Some(Version { major, minor })
}

/// Connect through `Bearer.LE1`; `false` when this BlueZ does not offer it.
fn connect_bearer(device: &Proxy<'_, &Connection>) -> Result<bool, TransportError> {
    match device.method_call::<(), _, _, _>(LE_BEARER, "Connect", ()) {
        Ok(()) => {}
        Err(error) if error.name() == Some("org.bluez.Error.AlreadyConnected") => {}
        Err(error) if missing(&error) => return Ok(false),
        Err(error) => return Err(backend(error)),
    }
    debug!("connected over the LE bearer");
    Ok(true)
}

/// Connect with LE as the preferred bearer; `false` when this BlueZ does not offer it.
fn connect_preferring_le(device: &Proxy<'_, &Connection>) -> Result<bool, TransportError> {
    let previous = match device.get::<String>(DEVICE, "PreferredBearer") {
        Ok(previous) => previous,
        Err(error) if missing(&error) => return Ok(false),
        Err(error) => return Err(backend(error)),
    };
    if previous != "le" {
        device.set(DEVICE, "PreferredBearer", "le".to_owned()).map_err(backend)?;
    }
    let connected = device.method_call::<(), _, _, _>(DEVICE, "Connect", ());
    if previous != "le"
        && let Err(error) = device.set(DEVICE, "PreferredBearer", previous)
    {
        warn!("could not restore the preferred bearer: {error}");
    }
    match connected {
        Ok(()) => {}
        Err(error) if error.name() == Some("org.bluez.Error.AlreadyConnected") => {}
        Err(error) => return Err(backend(error)),
    }
    debug!("connected with LE as the preferred bearer");
    Ok(true)
}

/// Drop the LE link to `device`. `Device1.Disconnect` can leave it up: with the printer paired over
/// Classic, BlueZ answered it with "No matching connection for device" and LE stayed connected
/// (observed on BlueZ 5.87). Best effort; the caller still calls `Device1.Disconnect`.
pub(super) async fn disconnect_le(device: &str) {
    let path = format!("/org/bluez/{device}");
    let result = tokio::task::spawn_blocking(move || {
        let connection = Connection::new_system()?;
        connection.with_proxy(BLUEZ, path, DISCONNECT_TIMEOUT).method_call::<(), _, _, _>(LE_BEARER, "Disconnect", ())
    })
    .await;
    match result {
        Ok(Ok(())) => debug!("disconnected the LE bearer"),
        Ok(Err(error)) if missing(&error) || error.name() == Some("org.bluez.Error.NotConnected") => {}
        Ok(Err(error)) => warn!("could not disconnect the LE bearer: {error}"),
        Err(error) => warn!("could not disconnect the LE bearer: {error}"),
    }
}

/// Whether BlueZ lacks the interface, method or property, as it does with experimental features off.
fn missing(error: &dbus::Error) -> bool {
    matches!(
        error.name(),
        Some(
            "org.freedesktop.DBus.Error.UnknownMethod"
                | "org.freedesktop.DBus.Error.UnknownInterface"
                | "org.freedesktop.DBus.Error.UnknownObject"
                | "org.freedesktop.DBus.Error.UnknownProperty"
                | "org.freedesktop.DBus.Error.InvalidArgs"
        )
    )
}

fn backend(error: dbus::Error) -> TransportError {
    TransportError::Backend(error.message().unwrap_or("D-Bus error").to_owned())
}

/// Whether a failed `Device1.Connect` is BlueZ choosing Classic for a dual-mode device.
pub(super) fn chose_classic(error: &TransportError) -> bool {
    matches!(error, TransportError::Backend(message)
        if message.contains("No more profiles to connect to") || message.contains("br-connection"))
}

/// The error for BlueZ choosing Classic: advice to turn experimental features on, unless this
/// BlueZ is too old for them to help.
pub(super) fn classic_error(too_old: Option<Version>) -> TransportError {
    match too_old {
        Some(version) => TransportError::BluezTooOld(version.to_string()),
        None => TransportError::ClassicConnection,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_classic_connection_failures() {
        assert!(chose_classic(&TransportError::Backend("No more profiles to connect to".into())));
        assert!(chose_classic(&TransportError::Backend("br-connection-profile-unavailable".into())));
        assert!(!chose_classic(&TransportError::Backend("Software caused connection abort".into())));
        assert!(!chose_classic(&TransportError::Disconnected));
    }

    #[test]
    fn reads_the_version_from_bluez_own_device_id() {
        let version = |major, minor| Some(Version { major, minor });
        assert_eq!(parse_modalias("usb:v1D6Bp0246d0548"), version(5, 72));
        assert_eq!(parse_modalias("usb:v1D6Bp0246d0557"), version(5, 87));
        assert_eq!(parse_modalias("bluetooth:v1d6bp0246d0550"), version(5, 80));
        assert!(version(5, 72) < Some(FIRST_LE) && version(5, 83) >= Some(FIRST_LE));
        assert_eq!(parse_modalias("bluetooth:v1234p5678dABCD"), None, "DeviceID set in main.conf");
        assert_eq!(parse_modalias("usb:v1D6Bp0246d548"), None);
        assert_eq!(parse_modalias(""), None);
    }

    #[test]
    fn old_bluez_is_not_told_to_turn_on_experimental_features() {
        let old = parse_modalias("usb:v1D6Bp0246d0548");
        assert!(matches!(classic_error(old), TransportError::BluezTooOld(version) if version == "5.72"));
        assert!(matches!(classic_error(None), TransportError::ClassicConnection));
    }
}
