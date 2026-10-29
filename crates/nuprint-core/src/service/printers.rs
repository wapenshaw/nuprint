//! Discovery, saved printers, their profiles and settings, and the connection check.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use nuprint_protocol::Density;
use nuprint_transport::{AdapterState, Advertisement, ServiceInfo};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::api::*;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::profile::{ModelProfile, ResolvedProfile};
use crate::session::locate;
use crate::store::{CustomProfileRow, PrinterRow, now_ms};

use super::{AppService, CUSTOM_PROFILE_PREFIX, HardwareUse, blocking_io, private, relock};

fn gatt_services(services: &[ServiceInfo]) -> Vec<GattService> {
    services
        .iter()
        .map(|s| GattService {
            uuid: s.uuid.to_string(),
            characteristics: s
                .characteristics
                .iter()
                .map(|c| {
                    let p = c.properties;
                    let props = [
                        (p.read, "read"),
                        (p.write, "write"),
                        (p.write_without_response, "write_without_response"),
                        (p.notify, "notify"),
                        (p.indicate, "indicate"),
                    ];
                    GattCharacteristic {
                        uuid: c.uuid.to_string(),
                        properties: props.iter().filter(|(on, _)| *on).map(|(_, n)| (*n).to_owned()).collect(),
                    }
                })
                .collect(),
        })
        .collect()
}

fn adapter_status(state: AdapterState) -> AdapterStatus {
    match state {
        AdapterState::PoweredOn => AdapterStatus::Ready,
        AdapterState::PoweredOff => AdapterStatus::PoweredOff,
        AdapterState::Absent => AdapterStatus::Absent,
        AdapterState::PermissionDenied => AdapterStatus::PermissionDenied,
        AdapterState::PermissionNotAsked => AdapterStatus::PermissionNotAsked,
        AdapterState::PermissionPending => AdapterStatus::PermissionPending,
        AdapterState::Unknown => AdapterStatus::Unknown,
    }
}

impl AppService {
    pub fn profiles(&self) -> Vec<ProfileSummary> {
        self.inner.registry.summaries()
    }

    pub async fn adapter_status(&self) -> AdapterStatus {
        adapter_status(self.inner.transport.adapter_state().await)
    }

    // ------------------------------------------------------------ discovery

    fn discovered_printer(&self, adv: &Advertisement) -> DiscoveredPrinter {
        let os = self.inner.registry.os();
        let candidates: Vec<ProfileMatch> =
            self.inner.registry.candidates(adv).into_iter().map(|p| p.as_match(os)).collect();
        let store = self.store();
        let saved_printer_id = candidates
            .iter()
            .find_map(|c| store.find_printer_by_backend(&adv.backend_id, &c.profile_id).ok().flatten().map(|p| p.id));
        DiscoveredPrinter {
            discovery_id: adv.backend_id.clone(),
            name: adv.local_name.clone(),
            address: adv.address.clone(),
            rssi: adv.rssi,
            advertised_services: adv.services.iter().map(|u| u.to_string()).collect(),
            candidates,
            saved_printer_id,
        }
    }

    pub async fn start_scan(&self, request: StartScanRequest) -> AppResult<ScanStarted> {
        let adapter = self.adapter_status().await;
        match adapter {
            AdapterStatus::PoweredOff => return Err(nuprint_transport::TransportError::AdapterOff.into()),
            AdapterStatus::Absent => return Err(nuprint_transport::TransportError::AdapterAbsent.into()),
            AdapterStatus::PermissionDenied => return Err(nuprint_transport::TransportError::PermissionDenied.into()),
            // The scan itself waits for the answer to the macOS permission prompt.
            AdapterStatus::Ready
            | AdapterStatus::PermissionNotAsked
            | AdapterStatus::PermissionPending
            | AdapterStatus::Unknown => {}
        }
        let duration = Duration::from_secs(u64::from(request.duration_s.unwrap_or(8).clamp(1, 30)));
        let include_unknown = request.include_unknown.unwrap_or(false);
        let scan_id = Uuid::new_v4().to_string();
        let cancel = self.inner.shutdown.child_token();
        relock(&self.inner.scans).insert(scan_id.clone(), cancel.clone());
        let service = self.clone();
        let id = scan_id.clone();
        tokio::spawn(async move {
            let (tx, mut rx) = tokio::sync::mpsc::channel(128);
            let transport = service.inner.transport.clone();
            let scan_cancel = cancel.clone();
            let scan = tokio::spawn(async move { transport.scan(duration, tx, scan_cancel).await });
            let mut seen: HashMap<String, (Option<String>, Option<i16>)> = HashMap::new();
            let mut found = 0u32;
            while let Some(adv) = rx.recv().await {
                let key = (adv.local_name.clone(), adv.rssi);
                let previous = seen.get(&adv.backend_id).cloned();
                let changed = match &previous {
                    None => true,
                    Some((name, rssi)) => *name != key.0 || rssi.zip(key.1).is_some_and(|(a, b)| (a - b).abs() >= 5),
                };
                if !changed {
                    continue;
                }
                seen.insert(adv.backend_id.clone(), key);
                let device = service.discovered_printer(&adv);
                if device.candidates.is_empty() && !include_unknown {
                    continue;
                }
                relock(&service.inner.discovered).insert(adv.backend_id.clone(), adv);
                found += u32::from(previous.is_none());
                service.emit(AppEvent::ScanDevice { scan_id: id.clone(), device });
            }
            let error = match scan.await {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(AppError::from(e)),
                Err(e) => Some(AppError::internal(e)),
            };
            relock(&service.inner.scans).remove(&id);
            service.log(LogLevel::Info, format!("Scan finished: {found} device(s)"));
            service.emit(AppEvent::ScanFinished { scan_id: id, found, error });
        });
        Ok(ScanStarted { scan_id, adapter })
    }

    pub fn stop_scan(&self, scan_id: &str) {
        if let Some(token) = relock(&self.inner.scans).get(scan_id) {
            token.cancel();
        }
    }

    /// Scan for one device by exact name, address or backend ID (CLI targeting).
    pub async fn find_device(&self, target: &str, timeout: Duration) -> AppResult<DiscoveredPrinter> {
        let cancel = self.inner.shutdown.child_token();
        let adv = locate(self.inner.transport.as_ref(), timeout, &cancel, |a| {
            a.backend_id.eq_ignore_ascii_case(target)
                || a.address.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(target))
                || a.local_name.as_deref() == Some(target)
        })
        .await?;
        relock(&self.inner.discovered).insert(adv.backend_id.clone(), adv.clone());
        Ok(self.discovered_printer(&adv))
    }

    /// First supported device seen during a scan (CLI default target).
    pub async fn find_first_supported(&self, timeout: Duration) -> AppResult<DiscoveredPrinter> {
        let cancel = self.inner.shutdown.child_token();
        let registry = &self.inner.registry;
        let adv =
            locate(self.inner.transport.as_ref(), timeout, &cancel, |a| !registry.candidates(a).is_empty()).await?;
        relock(&self.inner.discovered).insert(adv.backend_id.clone(), adv.clone());
        Ok(self.discovered_printer(&adv))
    }

    fn discovered(&self, discovery_id: &str) -> AppResult<Advertisement> {
        relock(&self.inner.discovered).get(discovery_id).cloned().ok_or_else(|| {
            AppError::new(ErrorCode::DeviceNotFound, "That device is no longer in the scan results. Scan again.")
        })
    }

    /// Read-only GATT inspection; no vendor protocol bytes are written.
    /// [`Self::cancel_inspections`] stops it when nobody is waiting for the result any more.
    pub async fn inspect_device(&self, discovery_id: &str) -> AppResult<DeviceInspection> {
        let id = Uuid::new_v4().to_string();
        let cancel = self.inner.shutdown.child_token();
        relock(&self.inner.inspections).insert(id.clone(), cancel.clone());
        let result = self.inspect(discovery_id, &cancel).await;
        relock(&self.inner.inspections).remove(&id);
        result
    }

    /// Cancel every running inspection; each releases Bluetooth once it has disconnected.
    pub fn cancel_inspections(&self) {
        for token in relock(&self.inner.inspections).values() {
            token.cancel();
        }
    }

    async fn inspect(&self, discovery_id: &str, cancel: &CancellationToken) -> AppResult<DeviceInspection> {
        let adv = self.discovered(discovery_id)?;
        let _hw = self.claim_hardware(HardwareUse::Inspecting, cancel).await?;
        let timeout = Duration::from_secs(25);
        let failed = |e: AppError| {
            if e.code != ErrorCode::Cancelled {
                self.log(LogLevel::Warn, format!("Inspection failed: {}", e.message));
            }
            e
        };
        // Find the device again first, as a print does: the OS can lose track of a device after
        // a disconnect (seen on macOS), leaving the handle from an earlier scan unable to connect.
        locate(self.inner.transport.as_ref(), Duration::from_secs(12), cancel, |a| a.backend_id == discovery_id)
            .await
            .map_err(failed)?;
        // Awaited to the end: on cancellation the transport disconnects before it returns.
        let link = self.inner.transport.connect(discovery_id, timeout, cancel).await.map_err(|e| failed(e.into()))?;
        let services = link.services().to_vec();
        let max_write_len = link.max_write_len(nuprint_transport::WriteMode::WithoutResponse) as u32;
        let _ = tokio::time::timeout(Duration::from_secs(10), link.disconnect()).await;
        let os = self.inner.registry.os();
        let mut compatible = Vec::new();
        let mut rejected = Vec::new();
        for profile in self.inner.registry.candidates(&adv) {
            match profile.profile.bind(&services) {
                Ok(_)
                    if profile.profile.notify_endpoints(&services).len()
                        >= profile.profile.transport.min_notify_subscriptions.max(1) =>
                {
                    compatible.push(profile.as_match(os))
                }
                Ok(_) => {
                    rejected.push(format!("{}: required status notifications missing", profile.profile.display_name))
                }
                Err(why) => rejected.push(format!("{}: {why}", profile.profile.display_name)),
            }
        }
        self.log(
            LogLevel::Info,
            format!(
                "Inspected {}: {} service(s)",
                private(adv.local_name.as_deref().unwrap_or("device")),
                services.len()
            ),
        );
        Ok(DeviceInspection {
            discovery_id: discovery_id.to_owned(),
            name: adv.local_name,
            services: gatt_services(&services),
            compatible,
            rejected,
            max_write_len,
        })
    }

    // ------------------------------------------------------------ printers

    fn saved_printer(&self, row: &PrinterRow) -> SavedPrinter {
        let os = self.inner.registry.os();
        let resolved = self.inner.registry.get_revision(&row.profile_id, row.profile_revision);
        let resolved = resolved.as_ref();
        SavedPrinter {
            id: row.id.clone(),
            name: row.name.clone(),
            backend_id: row.backend_id.clone(),
            address: row.address.clone(),
            profile_id: row.profile_id.clone(),
            profile_revision: row.profile_revision,
            profile_name: resolved.map_or_else(|| row.profile_id.clone(), |p| p.profile.display_name.clone()),
            support: resolved.map_or(SupportLevel::Candidate, |p| p.profile.support_on(os).0),
            firmware: row.firmware.clone(),
            last_seen_ms: row.last_seen_ms.map(|v| v as f64),
            settings: PrinterSettings { density: row.density, feed_mm: row.feed_mm },
            profile_available: resolved.is_some(),
            customized: row.base_profile_id.is_some(),
        }
    }

    pub fn list_printers(&self) -> AppResult<Vec<SavedPrinter>> {
        let rows = self.store().list_printers()?;
        Ok(rows.iter().map(|r| self.saved_printer(r)).collect())
    }

    pub(super) fn printer_row(&self, printer_id: &str) -> AppResult<PrinterRow> {
        self.store().get_printer(printer_id)?.ok_or_else(|| AppError::not_found("Printer"))
    }

    pub fn save_printer(&self, request: SavePrinterRequest) -> AppResult<SavedPrinter> {
        let adv = self.discovered(&request.discovery_id)?;
        let os = self.inner.registry.os();
        let candidates = self.inner.registry.candidates(&adv);
        // A manual choice cannot bypass name/identity rules.
        let resolved = candidates.into_iter().find(|p| p.profile.id == request.profile_id).ok_or_else(|| {
            AppError::new(ErrorCode::UnsupportedProfile, "This device does not match the selected printer profile.")
        })?;
        let (support, note) = resolved.profile.support_on(os);
        match support {
            SupportLevel::Candidate => {
                return Err(AppError::new(ErrorCode::UnsupportedProfile, "Printing to this model is not supported."));
            }
            SupportLevel::Experimental if !request.acknowledge_experimental => {
                return Err(AppError::new(
                    ErrorCode::ExperimentalNotAcknowledged,
                    "This printer is experimental on this computer. Confirm to use it anyway.",
                )
                .with_detail(note));
            }
            _ => {}
        }
        let store = self.store();
        let name = adv.local_name.clone().unwrap_or_else(|| resolved.profile.display_name.clone());
        let row = match store.find_printer_by_backend(&adv.backend_id, &resolved.profile.id)? {
            Some(mut existing) => {
                existing.name = name;
                existing.address = adv.address.clone();
                // Saving again must not discard settings the user edited.
                if existing.base_profile_id.is_none() {
                    existing.profile_revision = resolved.profile.revision;
                }
                existing.last_seen_ms = Some(now_ms());
                existing
            }
            None => PrinterRow {
                id: Uuid::new_v4().to_string(),
                name,
                backend_kind: "ble_gatt".into(),
                backend_id: adv.backend_id.clone(),
                address: adv.address.clone(),
                profile_id: resolved.profile.id.clone(),
                profile_revision: resolved.profile.revision,
                base_profile_id: None,
                firmware: None,
                last_seen_ms: Some(now_ms()),
                density: resolved.profile.default_density,
                feed_mm: resolved.driver.feed().default_mm,
                created_ms: now_ms(),
            },
        };
        store.upsert_printer(&row)?;
        if store.get_pref::<String>("selected_printer")?.is_none() {
            store.set_pref("selected_printer", &row.id)?;
        }
        drop(store);
        self.log(LogLevel::Info, format!("Saved printer {} ({})", private(&row.name), resolved.profile.display_name));
        self.emit(AppEvent::PrintersChanged { seq: self.next_seq() });
        Ok(self.saved_printer(&row))
    }

    pub fn remove_printer(&self, printer_id: &str) -> AppResult<()> {
        let store = self.store();
        if !store.delete_printer(printer_id)? {
            return Err(AppError::not_found("Printer"));
        }
        if store.get_pref::<String>("selected_printer")?.as_deref() == Some(printer_id) {
            let next = store.list_printers()?.first().map(|p| p.id.clone());
            store.set_pref("selected_printer", &next)?;
        }
        drop(store);
        self.emit(AppEvent::PrintersChanged { seq: self.next_seq() });
        Ok(())
    }

    pub(super) fn printer_profile(&self, row: &PrinterRow) -> AppResult<ResolvedProfile> {
        self.inner.registry.get_revision(&row.profile_id, row.profile_revision).ok_or_else(|| {
            AppError::new(ErrorCode::UnsupportedProfile, "This printer's profile is unavailable in this version.")
        })
    }

    /// The built-in profile a saved printer uses, or started from.
    fn base_profile(&self, row: &PrinterRow) -> AppResult<ResolvedProfile> {
        let base_id = row.base_profile_id.as_deref().unwrap_or(&row.profile_id);
        self.inner.registry.get(base_id).ok_or_else(|| {
            AppError::new(
                ErrorCode::UnsupportedProfile,
                "This printer's built-in profile is unavailable in this version.",
            )
        })
    }

    /// Keep a printer's darkness and feed valid after its profile changed.
    fn fit_settings(row: &mut PrinterRow, profile: &ResolvedProfile) {
        if !profile.driver.densities().contains(&row.density) {
            row.density = profile.profile.default_density;
        }
        let feed = profile.driver.feed();
        if !feed.contains(row.feed_mm) {
            row.feed_mm = feed.default_mm;
        }
    }

    /// The settings a user may edit for this printer, with the built-in ones for comparison.
    pub fn printer_profile_doc(&self, printer_id: &str) -> AppResult<PrinterProfileDoc> {
        let row = self.printer_row(printer_id)?;
        let current = self.printer_profile(&row)?;
        let base = self.base_profile(&row)?;
        let pretty = |p: &ModelProfile| serde_json::to_string_pretty(&p.tunable_doc()).map_err(AppError::internal);
        Ok(PrinterProfileDoc {
            printer_id: row.id.clone(),
            base_profile_id: base.profile.id.clone(),
            base_display_name: base.profile.display_name.clone(),
            customized: row.base_profile_id.is_some(),
            base_json: pretty(&base.profile)?,
            current_json: pretty(&current.profile)?,
        })
    }

    /// Store edited settings for one printer as a new revision of its own custom profile.
    /// Settings identical to the built-in ones go back to the built-in profile instead.
    /// Jobs already queued keep the revision they were rendered for.
    pub fn save_printer_profile(&self, request: SavePrinterProfileRequest) -> AppResult<SavedPrinter> {
        let doc: serde_json::Value = serde_json::from_str(&request.json)
            .map_err(|e| AppError::invalid(format!("The settings are not valid JSON: {e}")))?;
        // One store lock from choosing the revision to publishing it: two saves cannot pick the
        // same revision, and the registry only ever offers a revision that was stored.
        let mut store = self.store();
        let mut row = store.get_printer(&request.printer_id)?.ok_or_else(|| AppError::not_found("Printer"))?;
        let base = self.base_profile(&row)?;
        let custom_id = format!("{CUSTOM_PROFILE_PREFIX}{}", row.id);
        let revision =
            self.inner.registry.next_custom_revision(&custom_id).max(store.max_custom_revision(&custom_id)? + 1);
        let profile = ModelProfile::customized(&base.profile, &doc, custom_id, revision)?;
        if profile.tunable_doc() == base.profile.tunable_doc() {
            drop(store);
            return self.reset_printer_profile(&request.printer_id);
        }
        let json = serde_json::to_string(&profile).map_err(AppError::internal)?;
        let resolved = self.inner.registry.prepare_custom(profile)?;
        row.profile_id = resolved.profile.id.clone();
        row.profile_revision = revision;
        row.base_profile_id = Some(base.profile.id.clone());
        Self::fit_settings(&mut row, &resolved);
        store.save_custom_profile(
            &CustomProfileRow { id: resolved.profile.id.clone(), revision, json },
            &base.profile.id,
            &row,
        )?;
        self.inner.registry.publish_custom(resolved)?;
        drop(store);
        self.log(
            LogLevel::Info,
            format!("Saved edited settings for printer {} (revision {revision})", private(&row.name)),
        );
        self.emit(AppEvent::PrintersChanged { seq: self.next_seq() });
        Ok(self.saved_printer(&row))
    }

    /// Go back to the built-in settings. Edited revisions stay stored for jobs that name them.
    pub fn reset_printer_profile(&self, printer_id: &str) -> AppResult<SavedPrinter> {
        let store = self.store();
        let mut row = store.get_printer(printer_id)?.ok_or_else(|| AppError::not_found("Printer"))?;
        if row.base_profile_id.is_none() {
            drop(store);
            return Ok(self.saved_printer(&row));
        }
        let base = self.base_profile(&row)?;
        row.profile_id = base.profile.id.clone();
        row.profile_revision = base.profile.revision;
        row.base_profile_id = None;
        Self::fit_settings(&mut row, &base);
        store.upsert_printer(&row)?;
        drop(store);
        self.log(LogLevel::Info, format!("Printer {} uses the built-in settings again", private(&row.name)));
        self.emit(AppEvent::PrintersChanged { seq: self.next_seq() });
        Ok(self.saved_printer(&row))
    }

    /// The printer's settings as a complete profile file, for sharing. An edited profile gets a
    /// portable ID instead of this computer's printer ID.
    pub fn export_printer_profile(&self, printer_id: &str) -> AppResult<String> {
        let row = self.printer_row(printer_id)?;
        let mut profile = (*self.printer_profile(&row)?.profile).clone();
        if let Some(base) = &profile.derived_from {
            profile.id = format!("{base}-custom");
            profile.revision = 1;
        }
        serde_json::to_string_pretty(&profile).map_err(AppError::internal)
    }

    /// Write [`Self::export_printer_profile`] to a file the user chose.
    pub async fn save_printer_profile_file(&self, printer_id: &str, path: &Path) -> AppResult<()> {
        let json = self.export_printer_profile(printer_id)?;
        let path = path.to_path_buf();
        blocking_io(move || Ok(fs::write(path, json)?)).await
    }

    /// Reject darkness, extra feed or copies the profile does not accept, before any work.
    pub fn check_job_options(&self, profile_id: &str, density: Density, feed_mm: u16, copies: u16) -> AppResult<()> {
        let resolved = self.inner.registry.get(profile_id).ok_or_else(|| AppError::not_found("Printer profile"))?;
        resolved.check_job_options(density, feed_mm, Some(copies))
    }

    pub fn update_printer_settings(&self, printer_id: &str, settings: PrinterSettings) -> AppResult<SavedPrinter> {
        // Read, check and write under one store lock, so a profile saved meanwhile is not undone.
        let store = self.store();
        let row = store.get_printer(printer_id)?.ok_or_else(|| AppError::not_found("Printer"))?;
        self.printer_profile(&row)?.check_job_options(settings.density, settings.feed_mm, None)?;
        if !store.update_printer_settings(&row.id, settings.density, settings.feed_mm)? {
            return Err(AppError::not_found("Printer"));
        }
        drop(store);
        let row = PrinterRow { density: settings.density, feed_mm: settings.feed_mm, ..row };
        self.emit(AppEvent::PrintersChanged { seq: self.next_seq() });
        Ok(self.saved_printer(&row))
    }

    pub fn selected_printer(&self) -> AppResult<Option<String>> {
        Ok(self.store().get_pref::<Option<String>>("selected_printer")?.flatten())
    }

    pub fn select_printer(&self, printer_id: Option<String>) -> AppResult<()> {
        if let Some(id) = &printer_id {
            self.printer_row(id)?;
        }
        self.store().set_pref("selected_printer", &printer_id)?;
        self.emit(AppEvent::PrintersChanged { seq: self.next_seq() });
        Ok(())
    }

    /// Connect and query status without printing.
    pub async fn check_printer(&self, printer_id: &str) -> AppResult<PrinterCheck> {
        let row = self.printer_row(printer_id)?;
        let resolved = self.printer_profile(&row)?.clone();
        let cancel = self.inner.shutdown.child_token();
        let _hw = self.claim_hardware(HardwareUse::Checking, &cancel).await?;
        self.log(LogLevel::Info, format!("Checking {}…", private(&row.name)));
        let failed = |e: AppError| {
            self.log(LogLevel::Warn, format!("Connection check failed: {}", e.message));
            e
        };
        let mut session = self.connect_session(&resolved, &row.backend_id, &cancel).await.map_err(failed)?;
        let result = session.diagnose(&cancel).await;
        let services = gatt_services(session.services());
        let firmware = session.device_info().and_then(|i| i.firmware);
        let notifications = session.raw_notifications();
        let max_write_len = session.max_write_len() as u32;
        session.close().await;
        let status = result.map_err(failed)?;
        // `row` is as it was before the wait; only the check's own findings are written back.
        self.store().record_printer_seen(&row.id, firmware.as_deref(), now_ms())?;
        let check = PrinterCheck {
            printer_id: row.id.clone(),
            ready: status.is_ready(),
            status_text: status.text(),
            flags: status.flags.clone(),
            raw_status_hex: hex::encode(&status.raw),
            firmware,
            max_write_len,
            services,
            notifications,
            checked_ms: now_ms() as f64,
        };
        self.log(LogLevel::Info, format!("Connection verified · {}", check.status_text));
        *relock(&self.inner.last_check) = Some(check.clone());
        self.emit(AppEvent::PrintersChanged { seq: self.next_seq() });
        Ok(check)
    }
}
