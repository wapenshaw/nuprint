//! The context every callback and event handler carries: the service, the window handle, the
//! shared state and the runtime, plus the plumbing that reaches the UI thread and reloads data.

use std::sync::{Arc, Mutex};

use nuprint_core::api::*;
use nuprint_core::{AppError, AppService, ErrorCode};
use slint::{ComponentHandle, ModelRc, VecModel, Weak};

use crate::state::{State, Status};
use crate::view::*;
use crate::{AppWindow, Data};

#[derive(Clone)]
pub(crate) struct Ctx {
    pub(crate) service: AppService,
    pub(crate) ui: Weak<AppWindow>,
    pub(crate) state: Arc<Mutex<State>>,
    pub(crate) rt: tokio::runtime::Handle,
    /// Held for a whole background refresh, so an older read cannot land after a newer one.
    pub(crate) refreshing: Arc<Mutex<()>>,
}

impl Ctx {
    pub(crate) fn ui(&self, f: impl FnOnce(&AppWindow) + Send + 'static) {
        let _ = self.ui.upgrade_in_event_loop(move |ui| f(&ui));
    }

    pub(crate) fn notify(&self, text: impl Into<String>, tone: i32, action: Option<(&'static str, &'static str)>) {
        self.state.lock().unwrap().status = Some(Status { text: text.into(), tone, action });
        self.sync_status();
    }

    pub(crate) fn fail(&self, error: &AppError) {
        if error.code != ErrorCode::StalePreview {
            self.notify(error.message.clone(), 4, None);
        }
    }

    pub(crate) fn save_prefs(&self) {
        let st = self.state.lock().unwrap();
        if let Ok(json) = serde_json::to_vec(&st.prefs) {
            let _ = std::fs::write(st.data_dir.join("prefs.json"), json);
        }
    }

    pub(crate) fn refresh_adapter(&self) {
        let this = self.clone();
        self.rt.spawn(async move {
            let status = this.service.adapter_status().await;
            this.state.lock().unwrap().adapter = Some(status);
            this.sync_scan();
        });
    }

    pub(crate) fn refresh_printers(&self) {
        self.in_background(Self::load_printers);
    }

    /// Run a reload on a blocking thread: it reads the store (and may encode job thumbnails),
    /// which must not hold up the UI thread or a runtime worker. Its results reach the UI
    /// through `ui`, in the order the reloads finish.
    pub(crate) fn in_background(&self, load: fn(&Self)) {
        let this = self.clone();
        self.rt.spawn_blocking(move || {
            let _serial = this.refreshing.lock().unwrap();
            load(&this);
        });
    }

    /// Reload printers together with the profile list: saving or resetting edited settings
    /// changes which profile, and which revision, a printer uses.
    pub(crate) fn load_printers(&self) {
        let printers = match self.service.list_printers() {
            Ok(printers) => printers,
            Err(e) => return self.fail(&e),
        };
        let profiles = self.service.profiles();
        let mut selected =
            self.service.selected_printer().ok().flatten().filter(|id| printers.iter().any(|p| &p.id == id));
        if selected.is_none() {
            // The first saved printer is picked for you. Selecting reports a change, so only
            // do it when there is something to select.
            selected = printers.first().map(|p| p.id.clone());
            if selected.is_some() {
                let _ = self.service.select_printer(selected.clone());
            }
        }
        {
            let mut st = self.state.lock().unwrap();
            st.printers = printers;
            st.profiles = profiles;
            st.selected = selected;
            let saved: Vec<_> = st.printers.iter().filter_map(|p| p.address.clone()).collect();
            for device in &mut st.found {
                if device.saved_printer_id.is_none() && device.address.as_ref().is_some_and(|a| saved.contains(a)) {
                    device.saved_printer_id = Some(String::new());
                }
            }
        }
        self.sync_printers();
        self.sync_scan();
        let ctx = self.clone();
        self.ui(move |ui| ctx.schedule_preview(ui, true));
    }

    pub(crate) fn selected_profile(st: &State) -> Option<ProfileSummary> {
        let printer = st.printers.iter().find(|p| Some(&p.id) == st.selected.as_ref())?;
        if !printer.profile_available {
            return None;
        }
        st.profiles.iter().find(|p| p.id == printer.profile_id && p.revision == printer.profile_revision).cloned()
    }

    pub(crate) fn refresh_jobs(&self) {
        self.in_background(Self::load_jobs);
    }

    pub(crate) fn load_jobs(&self) {
        let queue = match self.service.list_jobs() {
            Ok(queue) => queue,
            Err(e) => return self.fail(&e),
        };
        let views: Vec<JobView> = queue
            .jobs
            .iter()
            .map(|job| {
                let cached = self.state.lock().unwrap().thumbs.get(&job.id).cloned();
                let thumb = cached.or_else(|| {
                    let buffer = decode_png(&self.service.job_page_png(&job.id)?)?;
                    self.state.lock().unwrap().thumbs.insert(job.id.clone(), buffer.clone());
                    Some(buffer)
                });
                JobView { job: job.clone(), thumb }
            })
            .collect();
        let open = queue.jobs.iter().filter(|j| !j.state.is_terminal()).count() as i32;
        let waiting = queue.jobs.iter().filter(|j| matches!(j.state, JobState::Queued | JobState::Held)).count();
        let can_clear = queue.jobs.iter().any(|j| j.state.is_terminal());
        let has_held = queue.jobs.iter().any(|j| j.state == JobState::Held);
        let held = queue.held;
        let fault = queue.fault.as_ref().map(|f| f.message.clone()).unwrap_or_default();
        {
            let mut st = self.state.lock().unwrap();
            st.held = held;
            st.jobs = queue.jobs;
        }
        self.ui(move |ui| {
            let d = ui.global::<Data>();
            d.set_jobs(ModelRc::new(VecModel::from(views.into_iter().map(JobView::into_item).collect::<Vec<_>>())));
            d.set_open_jobs(open);
            d.set_queue_held(held);
            d.set_queue_fault(fault.into());
            d.set_has_held_jobs(has_held && !held);
            d.set_can_clear(can_clear);
            d.set_waiting_text(if waiting == 0 {
                "Nothing waiting".into()
            } else {
                format!("{waiting} waiting").into()
            });
        });
        self.sync_jobs_bar();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::State;
    use nuprint_core::profile::ProfileRegistry;

    fn printer(profile_id: &str, profile_revision: u32) -> SavedPrinter {
        SavedPrinter {
            id: "p1".into(),
            name: "Test printer".into(),
            backend_id: "mock".into(),
            address: None,
            profile_id: profile_id.into(),
            profile_revision,
            profile_name: "Edited".into(),
            support: SupportLevel::Experimental,
            firmware: None,
            last_seen_ms: None,
            settings: PrinterSettings { density: Density::Normal, feed_mm: 12 },
            profile_available: true,
            customized: true,
        }
    }

    #[test]
    fn the_selected_printer_gets_exactly_its_profile_revision() {
        let builtin = ProfileRegistry::builtin().unwrap().summaries().remove(0);
        let custom = |revision| ProfileSummary { id: "custom:p1".into(), revision, ..builtin.clone() };
        let mut st = State {
            profiles: vec![builtin.clone(), custom(1), custom(2)],
            printers: vec![printer("custom:p1", 2)],
            selected: Some("p1".into()),
            ..State::default()
        };
        assert_eq!(Ctx::selected_profile(&st).map(|p| (p.id, p.revision)), Some(("custom:p1".into(), 2)));

        // A revision the catalog has not loaded yet selects nothing rather than an older one.
        st.printers[0].profile_revision = 3;
        assert!(Ctx::selected_profile(&st).is_none());
        st.profiles.push(custom(3));
        assert_eq!(Ctx::selected_profile(&st).map(|p| p.revision), Some(3));

        st.printers[0].profile_available = false;
        assert!(Ctx::selected_profile(&st).is_none());
        st.selected = None;
        assert!(Ctx::selected_profile(&st).is_none());
    }
}
