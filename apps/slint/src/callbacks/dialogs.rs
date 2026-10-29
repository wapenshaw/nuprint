//! The dialogs: add, remove, inspect and advanced settings.

use nuprint_core::api::*;
use serde_json::Value;
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::ctx::Ctx;
use crate::labels::*;
use crate::{AppWindow, Choice, Data, QuickField};

pub(super) fn wire(ui: &AppWindow, ctx: &Ctx) {
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
}

impl Ctx {
    pub(super) fn set_dialog(&self, ui: &AppWindow, name: &str) {
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
