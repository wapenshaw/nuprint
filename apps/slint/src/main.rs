//! NuPrint drawn with Slint. It uses the same `AppService` as the Tauri app and the CLI, so
//! scanning, printers, rendering, the job queue and diagnostics behave identically; only the
//! screens are different. The screens are in `ui/` and hold no logic: `Data` carries their state
//! and actions, and this file fills it and answers it.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod bootstrap;
mod ctx;
mod labels;
mod preview;
mod state;
mod sync;
mod view;

use std::time::{Duration, Instant};

use ctx::Ctx;
use labels::*;
use nuprint_core::api::*;
use serde_json::Value;
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use state::{CheckView, Prefs, State};
use view::*;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    bootstrap::run()
}

const MAX_LOG_LINES: usize = 500;
const FEED_DEBOUNCE: Duration = Duration::from_millis(400);
const PRINT_GUARD: Duration = Duration::from_millis(800);

// ------------------------------------------------------------------ context

impl Ctx {
    // -------------------------------------------------------------- printers

    fn check_printer(&self, id: String) {
        {
            let mut st = self.state.lock().unwrap();
            if st.checking.is_some() {
                return;
            }
            st.checking = Some(id.clone());
        }
        self.sync_printers();
        let this = self.clone();
        self.rt.spawn(async move {
            let result = this.service.check_printer(&id).await;
            {
                let mut st = this.state.lock().unwrap();
                st.checking = None;
                match &result {
                    Ok(check) => {
                        st.checks.insert(id.clone(), check_view(check));
                        st.last_check = Some((id.clone(), check.clone()));
                    }
                    Err(e) => {
                        st.checks.insert(
                            id.clone(),
                            CheckView { error: e.message.clone(), paper: true, ..Default::default() },
                        );
                    }
                }
            }
            this.sync_printers();
            this.sync_diagnostics();
            if let Err(e) = result {
                this.fail(&e);
            }
        });
    }

    // -------------------------------------------------------------- finding printers

    // -------------------------------------------------------------- queue

    fn on_event(&self, event: AppEvent) {
        match event {
            AppEvent::ScanDevice { device, .. } => {
                {
                    let mut st = self.state.lock().unwrap();
                    match st.found.iter_mut().find(|d| d.discovery_id == device.discovery_id) {
                        Some(existing) => *existing = device,
                        None => st.found.push(device),
                    }
                }
                self.sync_scan();
            }
            AppEvent::ScanFinished { found, error, .. } => {
                {
                    let mut st = self.state.lock().unwrap();
                    st.scan = None;
                    st.scan_error = error.as_ref().map(|e| e.message.clone()).unwrap_or_default();
                }
                self.sync_scan();
                if error.is_none() && found == 0 {
                    self.notify("No printers found. Make sure the printer is on and close its phone app.", 0, None);
                }
            }
            AppEvent::JobChanged { job, .. } => {
                let tracked = self.state.lock().unwrap().tracked.as_deref() == Some(job.id.as_str());
                if tracked && job.state.is_terminal() {
                    let tone = match job_tone(job.state) {
                        2 => 2,
                        4 => 4,
                        _ => 0,
                    };
                    self.notify(format!("{}: {}", job.title, job.state_text), tone, Some(("View queue", "queue")));
                }
                self.refresh_jobs();
            }
            AppEvent::JobRemoved { .. } => self.refresh_jobs(),
            AppEvent::QueueFault { fault, .. } => {
                self.notify(fault.message, 4, Some(("View queue", "queue")));
                self.refresh_jobs();
            }
            AppEvent::JobProgress { job_id, bytes_sent, bytes_total, .. } => {
                let progress = if bytes_total > 0.0 { (bytes_sent / bytes_total).clamp(0.0, 1.0) as f32 } else { 0.0 };
                let tracked = self.state.lock().unwrap().tracked.as_deref() == Some(job_id.as_str());
                self.ui(move |ui| {
                    let d = ui.global::<Data>();
                    let model = d.get_jobs();
                    for index in 0..model.row_count() {
                        if let Some(mut row) = model.row_data(index).filter(|r| r.id == job_id.as_str()) {
                            row.progress = progress;
                            row.progress_text = format!("{:.0}% sent", progress * 100.0).into();
                            model.set_row_data(index, row);
                            break;
                        }
                    }
                    if tracked {
                        d.set_job_progress(progress);
                    }
                });
            }
            AppEvent::PrintersChanged { .. } => self.refresh_printers(),
            AppEvent::Log { line } => {
                {
                    let mut st = self.state.lock().unwrap();
                    if line.seq <= st.log_seq {
                        return;
                    }
                    st.log_seq = line.seq;
                    st.logs.push(line);
                    if st.logs.len() > MAX_LOG_LINES {
                        st.logs.remove(0);
                    }
                }
                self.sync_diagnostics();
            }
        }
    }

    // -------------------------------------------------------------- diagnostics

    // -------------------------------------------------------------- source and preview

    fn print(&self, ui: &AppWindow) {
        let d = ui.global::<Data>();
        if !d.get_can_print() {
            return;
        }
        let (snapshot, printer, profile, title) = {
            let mut st = self.state.lock().unwrap();
            if st.last_print.is_some_and(|t| t.elapsed() < PRINT_GUARD) {
                return;
            }
            let printer = st.printers.iter().find(|p| Some(&p.id) == st.selected.as_ref()).cloned();
            let profile = Self::selected_profile(&st);
            let title = match d.get_mode() {
                0 => title_from_text(&d.get_text_value()),
                1 => Self::current_source(&st, 1).map(|s| s.name).unwrap_or_else(|| "Image".into()),
                2 => Self::current_source(&st, 2)
                    .map(|s| format!("{} – page {}", s.name, d.get_pdf_page()))
                    .unwrap_or_else(|| "PDF".into()),
                _ => "Test pattern".into(),
            };
            let snapshot = st.preview.as_ref().map(|p| p.snapshot_id.clone());
            st.last_print = Some(Instant::now());
            (snapshot, printer, profile, title)
        };
        let (Some(snapshot_id), Some(printer), Some(profile)) = (snapshot, printer, profile) else { return };
        let density =
            profile.densities.get(d.get_density_index().max(0) as usize).copied().unwrap_or(profile.default_density);
        let request = EnqueueRequest {
            snapshot_id,
            printer_id: printer.id,
            copies: d.get_copies().max(1) as u16,
            density,
            feed_mm: d.get_feed_mm().max(0) as u16,
            idempotency_key: uuid::Uuid::new_v4().to_string(),
            title,
        };
        match self.service.enqueue(request) {
            Ok(job) => {
                self.state.lock().unwrap().tracked = Some(job.id.clone());
                self.notify(format!("Added “{}” to the queue.", job.title), 2, Some(("View queue", "queue")));
                self.refresh_jobs();
            }
            Err(e) => self.fail(&e),
        }
    }

    /// Density and feed belong to the printer, so they persist; feed waits for typing to settle.
    fn settings_changed(&self, ui: &AppWindow) {
        self.print_bar(ui);
        let d = ui.global::<Data>();
        let (printer, profile) = {
            let st = self.state.lock().unwrap();
            (st.printers.iter().find(|p| Some(&p.id) == st.selected.as_ref()).cloned(), Self::selected_profile(&st))
        };
        let (Some(printer), Some(profile)) = (printer, profile) else { return };
        let density =
            profile.densities.get(d.get_density_index().max(0) as usize).copied().unwrap_or(profile.default_density);
        let feed = d.get_feed_mm().max(0) as u16;
        let seq = {
            let mut st = self.state.lock().unwrap();
            st.feed_seq += 1;
            st.feed_seq
        };
        let this = self.clone();
        self.rt.spawn(async move {
            tokio::time::sleep(FEED_DEBOUNCE).await;
            if this.state.lock().unwrap().feed_seq != seq || !profile.feed.contains(feed) {
                return;
            }
            if let Err(e) =
                this.service.update_printer_settings(&printer.id, PrinterSettings { density, feed_mm: feed })
            {
                this.fail(&e);
            }
        });
    }

    // -------------------------------------------------------------- dialogs

    fn set_dialog(&self, ui: &AppWindow, name: &str) {
        let d = ui.global::<Data>();
        d.set_dialog(name.into());
        d.set_dialog_error("".into());
        d.set_dialog_busy(false);
    }

    fn save_dialog_choice(&self, ui: &AppWindow) {
        let d = ui.global::<Data>();
        let st = self.state.lock().unwrap();
        let level = st.save_candidates.get(d.get_save_choice().max(0) as usize).map(|c| c.support);
        d.set_save_experimental(level == Some(SupportLevel::Experimental));
        d.set_save_note(
            match level {
                Some(SupportLevel::Candidate) => "NuPrint does not support this model yet, so it cannot be saved.",
                Some(SupportLevel::Experimental) => {
                    "Support for this model is experimental: it may print wrongly or waste paper."
                }
                _ => "",
            }
            .into(),
        );
        d.set_save_can(level.is_some_and(|l| l != SupportLevel::Candidate));
    }

    fn adv_refresh(&self, ui: &AppWindow, rebuild: bool) {
        let d = ui.global::<Data>();
        let st = self.state.lock().unwrap();
        let Some(doc) = &st.adv_doc else { return };
        let text = d.get_adv_json().to_string();
        let parsed: Option<Value> = serde_json::from_str(&text).ok().filter(Value::is_object);
        let base: Option<Value> = serde_json::from_str(&doc.base_json).ok();
        let mut any_invalid = parsed.is_none();
        let fields: Vec<QuickField> = QUICK_FIELDS
            .iter()
            .enumerate()
            .map(|(i, (label, path))| {
                let shown = st
                    .adv_typing
                    .get(&i)
                    .cloned()
                    .or_else(|| {
                        parsed
                            .as_ref()
                            .and_then(|p| get_path(p, path))
                            .map(|v| v.to_string().trim_matches('"').to_owned())
                    })
                    .unwrap_or_default();
                let (lo, hi) = quick_range(i, base.as_ref());
                let invalid = !shown.is_empty() && shown.parse::<u64>().ok().is_none_or(|n| !(lo..=hi).contains(&n));
                any_invalid |= invalid;
                QuickField { label: (*label).into(), value: shown.into(), invalid }
            })
            .collect();
        d.set_adv_invalid(any_invalid);
        d.set_adv_changed(text != doc.current_json);
        d.set_adv_customized(doc.customized);
        let model = d.get_adv_quick();
        match model.as_any().downcast_ref::<VecModel<QuickField>>() {
            Some(vec) if !rebuild && vec.row_count() == fields.len() => {
                for (i, field) in fields.into_iter().enumerate() {
                    vec.set_row_data(i, field);
                }
            }
            _ => d.set_adv_quick(ModelRc::new(VecModel::from(fields))),
        }
    }
}

// ------------------------------------------------------------------ callbacks

/// Registers one `Data` callback with the context and the window already in hand.
macro_rules! on {
    ($ui:expr, $ctx:expr, $name:ident, |$c:ident, $w:ident $(, $a:ident)*| $body:block) => {{
        let $c = $ctx.clone();
        let weak = $ui.as_weak();
        $ui.global::<Data>().$name(move |$($a),*| {
            let Some($w) = weak.upgrade() else { return };
            $body
        });
    }};
}

fn wire(ui: &AppWindow, ctx: &Ctx) {
    // ---- workspace
    on!(ui, ctx, on_source_changed, |c, ui| { c.schedule_preview(&ui, false) });
    on!(ui, ctx, on_adjust_changed, |c, ui| { c.schedule_preview(&ui, false) });
    on!(ui, ctx, on_mode_changed, |c, ui, mode| {
        ui.global::<Data>().set_mode(mode);
        c.sync_file(&ui);
        c.schedule_preview(&ui, true);
    });
    on!(ui, ctx, on_settings_changed, |c, ui| { c.settings_changed(&ui) });
    on!(ui, ctx, on_print, |c, ui| { c.print(&ui) });

    on!(ui, ctx, on_open_file, |c, ui| {
        let pdf = ui.global::<Data>().get_mode() == 2;
        let c2 = c.clone();
        c.rt.spawn(async move {
            let picked = tokio::task::spawn_blocking(move || {
                let dialog = rfd::FileDialog::new();
                if pdf {
                    dialog.add_filter("PDF", &["pdf"]).pick_file()
                } else {
                    dialog
                        .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "gif", "webp", "tif", "tiff"])
                        .pick_file()
                }
            })
            .await
            .ok()
            .flatten();
            let Some(path) = picked else { return };
            match c2.service.import_source(&path).await {
                Ok(info) => {
                    let mode = if info.kind == SourceKind::Pdf { 2 } else { 1 };
                    {
                        let mut st = c2.state.lock().unwrap();
                        if mode == 2 {
                            st.pdf_src = Some(info);
                        } else {
                            st.image_src = Some(info);
                        }
                    }
                    let c3 = c2.clone();
                    c2.ui(move |ui| {
                        let d = ui.global::<Data>();
                        d.set_mode(mode);
                        d.set_pdf_page(1);
                        d.set_pdf_password("".into());
                        c3.sync_file(ui);
                        c3.schedule_preview(ui, true);
                    });
                }
                Err(e) => c2.fail(&e),
            }
        });
    });

    on!(ui, ctx, on_save_png, |c, _ui| {
        let Some(snapshot) = c.state.lock().unwrap().preview.as_ref().map(|p| p.snapshot_id.clone()) else { return };
        let c2 = c.clone();
        c.rt.spawn(async move {
            let picked = tokio::task::spawn_blocking(|| {
                rfd::FileDialog::new()
                    .set_file_name("nuprint-preview.png")
                    .add_filter("PNG image", &["png"])
                    .save_file()
            })
            .await
            .ok()
            .flatten();
            let Some(path) = picked else { return };
            match c2.service.save_preview_png(&snapshot, &path).await {
                Ok(()) => c2.notify("Saved the preview as a PNG.", 2, None),
                Err(e) => c2.fail(&e),
            }
        });
    });

    // ---- shell
    on!(ui, ctx, on_show_view, |c, ui, view| {
        ui.global::<Data>().set_view(view.clone());
        if view == "printers" {
            c.refresh_adapter();
        }
    });
    on!(ui, ctx, on_close_view, |_c, ui| { ui.global::<Data>().set_view("workspace".into()) });
    on!(ui, ctx, on_dismiss_status, |c, _ui| {
        c.state.lock().unwrap().status = None;
        c.sync_status();
    });
    on!(ui, ctx, on_status_action, |c, ui| {
        let target = c.state.lock().unwrap().status.as_ref().and_then(|s| s.action.map(|a| a.1));
        if let Some(target) = target {
            ui.global::<Data>().set_view(target.into());
        }
    });
    on!(ui, ctx, on_cancel_latest_job, |c, _ui| {
        let id = {
            let st = c.state.lock().unwrap();
            st.tracked.clone().or_else(|| st.jobs.iter().find(|j| !j.state.is_terminal()).map(|j| j.id.clone()))
        };
        if let Some(id) = id
            && let Err(e) = c.service.cancel_job(&id)
        {
            c.fail(&e);
        }
    });

    // ---- printers
    on!(ui, ctx, on_select_printer, |c, _ui, id| {
        match c.service.select_printer(Some(id.to_string())) {
            Ok(()) => c.refresh_printers(),
            Err(e) => c.fail(&e),
        }
    });
    on!(ui, ctx, on_make_default, |c, _ui, id| {
        match c.service.select_printer(Some(id.to_string())) {
            Ok(()) => c.refresh_printers(),
            Err(e) => c.fail(&e),
        }
    });
    on!(ui, ctx, on_check_printer, |c, _ui, id| { c.check_printer(id.to_string()) });
    on!(ui, ctx, on_recheck_adapter, |c, _ui| { c.refresh_adapter() });
    on!(ui, ctx, on_find_printers, |c, ui| {
        let include_unknown = ui.global::<Data>().get_include_unknown();
        {
            let mut st = c.state.lock().unwrap();
            if st.scan.is_some() {
                return;
            }
            st.found.clear();
            st.scan_error.clear();
            st.scan = Some(String::new());
        }
        c.sync_scan();
        let c2 = c.clone();
        c.rt.spawn(async move {
            let request = StartScanRequest { duration_s: Some(8), include_unknown: Some(include_unknown) };
            match c2.service.start_scan(request).await {
                Ok(started) => {
                    let mut st = c2.state.lock().unwrap();
                    st.scan = Some(started.scan_id);
                    st.adapter = Some(started.adapter);
                }
                Err(e) => {
                    let mut st = c2.state.lock().unwrap();
                    st.scan = None;
                    st.scan_error = e.message.clone();
                }
            }
            c2.sync_scan();
        });
    });
    on!(ui, ctx, on_stop_scan, |c, _ui| {
        let id = c.state.lock().unwrap().scan.clone().filter(|s| !s.is_empty());
        if let Some(id) = id {
            c.service.stop_scan(&id);
        }
    });

    // ---- dialogs
    on!(ui, ctx, on_ask_save, |c, ui, id| {
        let device = c.state.lock().unwrap().found.iter().find(|d| d.discovery_id == id.as_str()).cloned();
        let Some(device) = device else { return };
        let d = ui.global::<Data>();
        d.set_save_title(format!("Add {}", device.name.clone().unwrap_or_else(|| "this printer".into())).into());
        d.set_save_choices(ModelRc::new(VecModel::from(
            device
                .candidates
                .iter()
                .map(|m| Choice {
                    id: m.profile_id.clone().into(),
                    label: m.display_name.clone().into(),
                    support: support_label(m.support).into(),
                    tone: support_tone(m.support),
                })
                .collect::<Vec<_>>(),
        )));
        d.set_save_choice(0);
        d.set_save_ack(false);
        {
            let mut st = c.state.lock().unwrap();
            st.dialog_device = Some(device.discovery_id);
            st.save_candidates = device.candidates;
        }
        c.save_dialog_choice(&ui);
        c.set_dialog(&ui, "save");
    });
    on!(ui, ctx, on_save_choice_changed, |c, ui, _index| { c.save_dialog_choice(&ui) });
    on!(ui, ctx, on_confirm_save, |c, ui| {
        let d = ui.global::<Data>();
        let (device, candidate) = {
            let st = c.state.lock().unwrap();
            (st.dialog_device.clone(), st.save_candidates.get(d.get_save_choice().max(0) as usize).cloned())
        };
        let (Some(device), Some(candidate)) = (device, candidate) else { return };
        let request = SavePrinterRequest {
            discovery_id: device,
            profile_id: candidate.profile_id,
            acknowledge_experimental: d.get_save_ack(),
        };
        match c.service.save_printer(request) {
            Ok(printer) => {
                c.set_dialog(&ui, "");
                c.notify(format!("Saved {}. Check the connection before the first print.", printer.name), 2, None);
                c.refresh_printers();
            }
            Err(e) => ui.global::<Data>().set_dialog_error(e.message.into()),
        }
    });
    on!(ui, ctx, on_ask_remove, |c, ui, id| {
        let name = c.state.lock().unwrap().printers.iter().find(|p| p.id == id.as_str()).map(|p| p.name.clone());
        let Some(name) = name else { return };
        ui.global::<Data>().set_remove_name(name.into());
        c.state.lock().unwrap().dialog_printer = Some(id.to_string());
        c.set_dialog(&ui, "remove");
    });
    on!(ui, ctx, on_confirm_remove, |c, ui| {
        let Some(id) = c.state.lock().unwrap().dialog_printer.clone() else { return };
        match c.service.remove_printer(&id) {
            Ok(()) => {
                c.state.lock().unwrap().checks.remove(&id);
                c.set_dialog(&ui, "");
                c.notify("Printer removed.", 2, None);
                c.refresh_printers();
            }
            Err(e) => ui.global::<Data>().set_dialog_error(e.message.into()),
        }
    });
    on!(ui, ctx, on_ask_inspect, |c, ui, id| {
        let device = c.state.lock().unwrap().found.iter().find(|d| d.discovery_id == id.as_str()).cloned();
        let Some(device) = device else { return };
        let name = device.name.clone().unwrap_or_else(|| "device".into());
        let d = ui.global::<Data>();
        d.set_inspect_title(format!("Inspect {name}").into());
        d.set_inspect_state(0);
        d.set_inspect_text("".into());
        c.set_dialog(&ui, "inspect");
        let c2 = c.clone();
        c.rt.spawn(async move {
            let (state, text) = match c2.service.inspect_device(&device.discovery_id).await {
                Ok(i) => {
                    let mut out = String::new();
                    for s in &i.services {
                        out += &format!("Service {}\n", s.uuid);
                        for ch in &s.characteristics {
                            out += &format!("  {}  [{}]\n", ch.uuid, ch.properties.join(", "));
                        }
                    }
                    out += &format!("\nWrite size: {} bytes\n", i.max_write_len);
                    out += &format!(
                        "Matching models: {}\n",
                        if i.compatible.is_empty() {
                            "none".to_owned()
                        } else {
                            i.compatible.iter().map(|m| m.display_name.clone()).collect::<Vec<_>>().join(", ")
                        }
                    );
                    for r in &i.rejected {
                        out += &format!("Rejected: {r}\n");
                    }
                    (1, out)
                }
                Err(e) => (2, e.message),
            };
            c2.ui(move |ui| {
                let d = ui.global::<Data>();
                d.set_inspect_state(state);
                d.set_inspect_text(text.into());
            });
        });
    });
    on!(ui, ctx, on_ask_advanced, |c, ui, id| {
        let name = c.state.lock().unwrap().printers.iter().find(|p| p.id == id.as_str()).map(|p| p.name.clone());
        let Some(name) = name else { return };
        match c.service.printer_profile_doc(&id) {
            Ok(doc) => {
                let d = ui.global::<Data>();
                d.set_adv_title(format!("Advanced settings for {name}").into());
                d.set_adv_json(doc.current_json.clone().into());
                {
                    let mut st = c.state.lock().unwrap();
                    st.dialog_printer = Some(id.to_string());
                    st.adv_doc = Some(doc);
                    st.adv_typing.clear();
                }
                c.set_dialog(&ui, "advanced");
                c.adv_refresh(&ui, true);
            }
            Err(e) => c.fail(&e),
        }
    });
    on!(ui, ctx, on_adv_json_edited, |c, ui, _text| {
        c.state.lock().unwrap().adv_typing.clear();
        c.adv_refresh(&ui, true);
    });
    on!(ui, ctx, on_adv_quick_edited, |c, ui, index, text| {
        let index = index as usize;
        let d = ui.global::<Data>();
        let mut parsed: Option<Value> = serde_json::from_str(&d.get_adv_json()).ok().filter(Value::is_object);
        c.state.lock().unwrap().adv_typing.insert(index, text.to_string());
        if let (Some(doc), Ok(n)) = (parsed.as_mut(), text.trim().parse::<u64>()) {
            set_path(doc, QUICK_FIELDS[index].1, n);
            if let Ok(pretty) = serde_json::to_string_pretty(doc) {
                d.set_adv_json(pretty.into());
            }
        }
        c.adv_refresh(&ui, false);
    });
    on!(ui, ctx, on_adv_save, |c, ui| {
        let Some(id) = c.state.lock().unwrap().dialog_printer.clone() else { return };
        let request =
            SavePrinterProfileRequest { printer_id: id, json: ui.global::<Data>().get_adv_json().to_string() };
        match c.service.save_printer_profile(request) {
            Ok(printer) => {
                c.set_dialog(&ui, "");
                c.notify(
                    format!(
                        "Saved the settings for {}. Print a short test strip before relying on them.",
                        printer.name
                    ),
                    2,
                    None,
                );
                c.state.lock().unwrap().applied_printer = None;
                c.refresh_printers();
            }
            Err(e) => ui.global::<Data>().set_dialog_error(e.message.into()),
        }
    });
    on!(ui, ctx, on_adv_reset, |c, ui| {
        let Some(id) = c.state.lock().unwrap().dialog_printer.clone() else { return };
        match c.service.reset_printer_profile(&id) {
            Ok(printer) => {
                c.set_dialog(&ui, "");
                c.notify(format!("{} uses its built-in settings again.", printer.name), 2, None);
                c.state.lock().unwrap().applied_printer = None;
                c.refresh_printers();
            }
            Err(e) => ui.global::<Data>().set_dialog_error(e.message.into()),
        }
    });
    on!(ui, ctx, on_close_dialog, |c, ui| {
        c.service.cancel_inspections();
        c.set_dialog(&ui, "");
    });

    // ---- queue
    on!(ui, ctx, on_toggle_hold, |c, _ui| {
        let held = c.state.lock().unwrap().held;
        let result = if held { c.service.resume_queue() } else { c.service.hold_queue() };
        match result {
            Ok(_) => c.refresh_jobs(),
            Err(e) => c.fail(&e),
        }
    });
    on!(ui, ctx, on_cancel_job, |c, _ui, id| {
        if let Err(e) = c.service.cancel_job(&id) {
            c.fail(&e);
        }
    });
    on!(ui, ctx, on_retry_job, |c, _ui, id| {
        match c.service.retry_job(&id, uuid::Uuid::new_v4().to_string()) {
            Ok(job) => {
                c.state.lock().unwrap().tracked = Some(job.id.clone());
                c.notify(format!("Added “{}” to the queue again.", job.title), 2, Some(("View queue", "queue")));
            }
            Err(e) => c.fail(&e),
        }
    });
    on!(ui, ctx, on_remove_job, |c, _ui, id| {
        c.state.lock().unwrap().thumbs.remove(id.as_str());
        if let Err(e) = c.service.remove_job(&id) {
            c.fail(&e);
        }
    });
    on!(ui, ctx, on_ask_clear, |c, ui| { c.set_dialog(&ui, "clear") });
    on!(ui, ctx, on_confirm_clear, |c, ui| {
        c.set_dialog(&ui, "");
        match c.service.clear_history() {
            Ok(()) => {
                c.state.lock().unwrap().thumbs.clear();
                c.refresh_jobs();
            }
            Err(e) => c.fail(&e),
        }
    });

    // ---- diagnostics
    on!(ui, ctx, on_preview_report, |c, ui| {
        let include = ui.global::<Data>().get_include_ids();
        let c2 = c.clone();
        c.rt.spawn(async move {
            let result = c2.service.diagnostics_report(include).await;
            c2.ui(move |ui| {
                let d = ui.global::<Data>();
                match result {
                    Ok(report) => {
                        d.set_report_error("".into());
                        d.set_report_text(serde_json::to_string_pretty(&report).unwrap_or_default().into());
                    }
                    Err(e) => d.set_report_error(e.message.into()),
                }
            });
        });
    });
    on!(ui, ctx, on_export_report, |c, ui| {
        let include = ui.global::<Data>().get_include_ids();
        ui.global::<Data>().set_exporting(true);
        let c2 = c.clone();
        c.rt.spawn(async move {
            let picked = tokio::task::spawn_blocking(|| {
                rfd::FileDialog::new().set_file_name("nuprint-diagnostics.json").save_file()
            })
            .await
            .ok()
            .flatten();
            let outcome = match picked {
                Some(path) => Some(c2.service.export_diagnostics(&path, include).await),
                None => None,
            };
            c2.ui(|ui| ui.global::<Data>().set_exporting(false));
            match outcome {
                Some(Ok(())) => c2.notify("Saved the diagnostics report.", 2, None),
                Some(Err(e)) => c2.fail(&e),
                None => {}
            }
        });
    });

    // ---- appearance
    on!(ui, ctx, on_set_mode_pref, |c, ui, mode| {
        ui.global::<Theme>().set_mode(mode);
        c.state.lock().unwrap().prefs.mode = mode;
        c.save_prefs();
    });
    on!(ui, ctx, on_set_accent, |c, ui, accent| {
        ui.global::<Theme>().set_accent_index(accent);
        c.state.lock().unwrap().prefs.accent = accent;
        c.save_prefs();
    });
    on!(ui, ctx, on_set_icon, |c, ui, icon| {
        ui.global::<Data>().set_brand_icon(icon);
        c.state.lock().unwrap().prefs.icon = icon;
        c.save_prefs();
    });
}
