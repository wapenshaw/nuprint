//! The workspace screen: the controls, the file and PNG dialogs, and Print.

use std::time::{Duration, Instant};

use nuprint_core::api::*;
use slint::ComponentHandle;

use crate::ctx::Ctx;
use crate::labels::*;
use crate::{AppWindow, Data};

const FEED_DEBOUNCE: Duration = Duration::from_millis(400);
const PRINT_GUARD: Duration = Duration::from_millis(800);

pub(super) fn wire(ui: &AppWindow, ctx: &Ctx) {
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
}

impl Ctx {
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
}
