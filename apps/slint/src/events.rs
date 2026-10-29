//! Service events: they arrive on any thread and are folded into the state and the screens.

use nuprint_core::api::*;
use slint::{ComponentHandle, Model};

use crate::Data;
use crate::ctx::Ctx;
use crate::labels::*;
use crate::sync::{MAX_LOG_LINES, queue_log};

impl Ctx {
    pub(crate) fn on_event(&self, event: AppEvent) {
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
                    // Lines can arrive out of order; `sync_logs` places them by sequence number.
                    let mut st = self.state.lock().unwrap();
                    queue_log(&mut st.new_logs, line, MAX_LOG_LINES);
                }
                self.sync_logs();
            }
        }
    }
}
