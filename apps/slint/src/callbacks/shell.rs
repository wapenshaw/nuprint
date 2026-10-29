//! The shell around the screens: view switching and the status line.

use slint::ComponentHandle;

use crate::ctx::Ctx;
use crate::{AppWindow, Data};

pub(super) fn wire(ui: &AppWindow, ctx: &Ctx) {
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
}
