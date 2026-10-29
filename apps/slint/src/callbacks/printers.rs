//! The Printers screen: choosing, checking and finding printers.

use nuprint_core::api::*;
use slint::ComponentHandle;

use crate::ctx::Ctx;
use crate::state::CheckView;
use crate::view::check_view;
use crate::{AppWindow, Data};

pub(super) fn wire(ui: &AppWindow, ctx: &Ctx) {
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
                    st.scan_error = e.message;
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
}

impl Ctx {
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
}
