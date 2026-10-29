//! The Queue screen: hold, cancel, retry, remove and clear.

use slint::ComponentHandle;

use crate::ctx::Ctx;
use crate::{AppWindow, Data};

pub(super) fn wire(ui: &AppWindow, ctx: &Ctx) {
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
}
