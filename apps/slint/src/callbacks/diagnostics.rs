//! The Diagnostics screen: the report preview and its export.

use slint::ComponentHandle;

use crate::ctx::Ctx;
use crate::{AppWindow, Data};

pub(super) fn wire(ui: &AppWindow, ctx: &Ctx) {
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
}
