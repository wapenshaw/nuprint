# NuPrint on Linux

How NuPrint reaches a printer through BlueZ, the Linux Bluetooth stack, why it needs BlueZ's experimental features, and what was tested. The [user guide](user-guide.md#installing) has the short version; [validation](validation.md#linux-2026-10-31) has the full hardware record.

## Setup

Turn on BlueZ's experimental features once, as root, then restart Bluetooth:

```bash
sudo sed -i 's/^#\?Experimental = .*/Experimental = true/' /etc/bluetooth/main.conf
sudo systemctl restart bluetooth
```

The line sits in the `[General]` section; Fedora and Ubuntu ship it commented out as `#Experimental = false`. No desktop (GNOME or KDE) offers a switch for it. NuPrint never edits this file itself: it is system-wide Bluetooth configuration, and it turns on BlueZ's experimental D-Bus interfaces for every application, not just NuPrint.

Do not pair the printer in system Bluetooth settings. NuPrint doesn't need pairing (see [pairing](#pairing)).

## Why plain BlueZ fails

**Observed on hardware** (SC03h-3DE8, Fedora 44, BlueZ 5.87). The SC03h is a dual-mode device: on one public address it offers the LE print service (advertised as `af30`, served as AE30) and Classic Bluetooth profiles (serial port, headset, A2DP, AVRCP, hands-free, phonebook).

btleplug connects through `org.bluez.Device1.Connect`, which lets BlueZ pick the bearer. For this printer BlueZ picks Classic: the printer refuses A2DP (`Connection refused (111)`), BlueZ answers "No more profiles to connect to", and the GATT print service never appears. The app showed this as "Bluetooth connection failed."

Neither of these changes that:

- Removing the pairing. BlueZ still learns the Classic side, because btleplug's scan always uses `Transport::Auto`, which runs Classic inquiry alongside the LE scan.
- Discovering the printer with an LE-only `bluetoothctl` scan and then calling `Device1.Connect`. BlueZ still connected over Classic and paired with it ("Just Works", without a prompt).

Windows and macOS are not affected: their Bluetooth APIs connect to LE devices over LE.

## How NuPrint connects

**Implemented** in `crates/nuprint-transport/src/ble/bluez.rs` (Linux only; Windows and macOS are unchanged). NuPrint calls BlueZ over D-Bus directly for what btleplug does not expose:

1. **LE discovery if needed.** BlueZ offers LE connection only for a device it has heard over LE (one that has the `org.bluez.Bearer.LE1` interface). A scan can find the printer by Classic inquiry first, leaving only `Bearer.BREDR1`. If so, NuPrint runs an LE-only discovery (`SetDiscoveryFilter` with `Transport = le`) for up to 5 seconds. Discovery filters belong to the D-Bus client that sets them, so btleplug's scans are unaffected.
2. **`Bearer.LE1.Connect`** where BlueZ offers it. This connects over LE for this connection only and stores nothing.
3. **Otherwise `PreferredBearer`.** NuPrint sets the device's `PreferredBearer` property to `le`, calls `Device1.Connect` and restores the previous value (normally `last-used`) whether or not the connection succeeded.
4. NuPrint then waits for `ServicesResolved`, as btleplug's own connect does.
5. **Neither available** (experimental features off, or BlueZ before 5.80): NuPrint falls back to btleplug's plain connect. It works only when the adapter is LE-only (`ControllerMode = le`). If BlueZ fails because it chose Classic, the app's message depends on the BlueZ version, which NuPrint reads from the adapter's `Modalias` (BlueZ's own device ID, such as `usb:v1D6Bp0246d0548` for 5.72):
   - 5.80 or newer, or a version NuPrint cannot read (for example because `DeviceID` in `main.conf` replaced it): "Linux tried to reach this printer over Classic Bluetooth. Set Experimental = true in /etc/bluetooth/main.conf, restart Bluetooth and retry."
   - Before 5.80, where experimental features cannot help: "This Linux system's Bluetooth (BlueZ) is too old to reach this printer. NuPrint needs BlueZ 5.80 or newer, so Ubuntu 24.04 and older are not supported." The diagnostic detail names the version.

When a job ends, or a connection attempt fails, NuPrint calls `Bearer.LE1.Disconnect` before `Device1.Disconnect`. With the printer paired over Classic, `Device1.Disconnect` alone left LE connected (BlueZ logged "No matching connection for device").

## Pairing

NuPrint never pairs. A connection through `Bearer.LE1` leaves the printer unpaired, and BlueZ forgets it about 30 seconds after the job (its default `TemporaryTimeout`). Turning the printer on near a Linux computer pairs nothing.

Pairing happens when something starts a Classic connection, for example clicking the printer in GNOME's Bluetooth settings. A printer paired that way (paired, bonded and trusted over Classic) still prints through NuPrint, but BlueZ reconnects to it over LE by itself about 10 seconds after NuPrint disconnects. The link then stays open between jobs and can stop a phone app from reaching the printer. **Forget** in Bluetooth settings ends that; after it the printer stayed disconnected.

## Distributions

| | BlueZ | Connection used | Status |
| --- | --- | --- | --- |
| Fedora 44 | 5.87 | `Bearer.LE1.Connect` | Tested: connection, check and text prints |
| Ubuntu 26.04 | 5.85 | `Bearer.LE1.Connect` | Expected; not tested |
| Ubuntu 25.10 and other BlueZ 5.80 to 5.83 | 5.83 | `PreferredBearer` | That path was tested on BlueZ 5.87 by skipping `Bearer.LE1`; not tested on these versions |
| Ubuntu 24.04 and older, and any BlueZ before 5.80 | 5.72 (24.04) | None | Not supported. `Experimental = true` does not help, because these versions have neither interface. `ControllerMode = le` in `/etc/bluetooth/main.conf` works but turns Classic Bluetooth off for every device |

BlueZ added `PreferredBearer` in 5.80 ([a22935f](https://github.com/bluez/bluez/commit/a22935f9ba)) and `org.bluez.Bearer.LE1` in 5.84 ([8c8b5ed](https://github.com/bluez/bluez/commit/8c8b5ed25a)); the `last-used` value of `PreferredBearer` arrived in 5.83. The Ubuntu versions are the published `bluez` packages on [Launchpad](https://launchpad.net/ubuntu/+source/bluez) as of 2026-10-31. Both interfaces are experimental in BlueZ: `Bearer.LE1` without experimental features has only its `Disconnected` signal, and `PreferredBearer` is absent.

## Alternatives considered

- **`ControllerMode = le`.** Works on any BlueZ (the first successful Linux print used it) but disables Classic Bluetooth for headphones and every other Classic device.
- **Printing over Classic (SPP).** The vendor app's SC03h model sets `useSPP` (APK-derived), so the protocol may work over a serial port. It would need a separate RFCOMM transport per OS, usually pairing, and hardware evidence that status and flow control work over a single stream. Not planned for V1.
- **Setting `Experimental = true` from the `.deb` or `.rpm`.** Rejected: it would silently change system Bluetooth configuration.
- **A button in the app that applies the setting through `pkexec`.** Possible later, with the user's explicit consent; not implemented.

## Known issue

`bluez-async` 0.9.0, which btleplug uses on Linux, unwraps the reply when it removes a D-Bus match as an event stream closes (`messagestream.rs:40`). If the match is already gone, that background task panics and prints "No match with that id found" to standard error. Tokio contains the panic to that task, nothing leaks and the app keeps running. No newer `bluez-async` or btleplug release exists; the fix belongs upstream.
