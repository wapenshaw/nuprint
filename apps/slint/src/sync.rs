//! Pushing state to the screens: each `sync_*` builds screen items from `State` and hands them to
//! the UI thread.

use std::collections::VecDeque;

use nuprint_core::api::*;
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::ctx::Ctx;
use crate::labels::*;
use crate::view::*;
use crate::{Data, DeviceItem, LogItem, PrinterItem};

/// The activity log keeps this many of the newest lines on screen.
pub(crate) const MAX_LOG_LINES: usize = 500;

/// How a batch of new log lines joins the rows on screen.
#[derive(Debug, Default)]
struct LogMerge {
    /// Lines older than the newest row, each with the row index it goes in at, applied in order.
    inserts: Vec<(usize, LogLine)>,
    /// Lines newer than every row, oldest first; added after the inserts.
    appends: Vec<LogLine>,
    /// Rows to remove from the front afterwards.
    trim: usize,
}

/// Merge `lines` into the rows whose sequence numbers are `shown`, and update `shown`.
///
/// Log events can arrive out of order: two threads log at once and the later sequence number is
/// delivered first. The start-up snapshot can also repeat the first events. So lines are placed
/// by sequence number, and a line already on screen is skipped.
fn merge_logs(shown: &mut VecDeque<f64>, mut lines: Vec<LogLine>, max: usize) -> LogMerge {
    lines.sort_by(|a, b| a.seq.total_cmp(&b.seq));
    let mut merge = LogMerge::default();
    for line in lines {
        let Err(at) = shown.binary_search_by(|seq| seq.total_cmp(&line.seq)) else { continue };
        shown.insert(at, line.seq);
        // Sorted input puts every insert before the first append.
        if at + 1 == shown.len() {
            merge.appends.push(line);
        } else {
            merge.inserts.push((at, line));
        }
    }
    merge.trim = shown.len().saturating_sub(max);
    shown.drain(..merge.trim);
    merge
}

impl Ctx {
    pub(crate) fn sync_status(&self) {
        let status = self.state.lock().unwrap().status.clone();
        self.ui(move |ui| {
            let d = ui.global::<Data>();
            d.set_has_status(status.is_some());
            if let Some(s) = status {
                d.set_status_prefix(
                    match s.tone {
                        4 => "Problem",
                        2 => "Done",
                        _ => "Note",
                    }
                    .into(),
                );
                d.set_status_text(s.text.into());
                d.set_status_tone(s.tone);
                d.set_status_action_label(s.action.map(|a| a.0).unwrap_or_default().into());
            } else {
                d.set_status_action_label("".into());
            }
        });
    }

    pub(crate) fn sync_printers(&self) {
        let (items, selected, profile, apply, selected_id, checking_any) = {
            let mut st = self.state.lock().unwrap();
            let default = st.selected.clone();
            let items: Vec<PrinterItem> = st
                .printers
                .iter()
                .map(|p| {
                    let check = st.checks.get(&p.id);
                    let checking = st.checking.as_ref() == Some(&p.id);
                    let (light, label) = if checking {
                        (1, "Checking…")
                    } else if let Some(c) = check {
                        if !c.error.is_empty() {
                            (4, "Not reachable")
                        } else if c.ok {
                            (2, "Ready")
                        } else {
                            (3, "Needs attention")
                        }
                    } else {
                        (0, "Not checked")
                    };
                    PrinterItem {
                        id: p.id.clone().into(),
                        name: p.name.clone().into(),
                        profile_line: format!(
                            "{}{}",
                            p.profile_name,
                            p.firmware.as_ref().map(|f| format!(" · firmware {f}")).unwrap_or_default()
                        )
                        .into(),
                        support: support_label(p.support).into(),
                        support_tone: support_tone(p.support),
                        customized: p.customized,
                        is_default: default.as_ref() == Some(&p.id),
                        unavailable: !p.profile_available,
                        light,
                        light_label: label.into(),
                        paper: check.map(|c| c.paper).unwrap_or(true),
                        checking,
                        check_error: check.map(|c| c.error.clone()).unwrap_or_default().into(),
                        has_check: check.is_some_and(|c| c.error.is_empty()),
                        check_ok: check.is_some_and(|c| c.ok),
                        check_line: check.map(|c| c.line.clone()).unwrap_or_default().into(),
                        check_flags: check.map(|c| c.flags.clone()).unwrap_or_default().into(),
                        check_firmware: check.map(|c| c.firmware.clone()).unwrap_or_default().into(),
                    }
                })
                .collect();
            let selected = st.printers.iter().find(|p| Some(&p.id) == st.selected.as_ref()).cloned();
            let profile = Self::selected_profile(&st);
            let apply = st.applied_printer != st.selected;
            st.applied_printer = st.selected.clone();
            (items, selected, profile, apply, st.selected.clone(), st.checking.is_some())
        };
        self.ui(move |ui| {
            let d = ui.global::<Data>();
            let current = items.iter().find(|i| Some(i.id.as_str()) == selected_id.as_deref()).cloned();
            d.set_has_printer(selected.is_some());
            d.set_printers(ModelRc::new(VecModel::from(items)));
            d.set_selected_id(selected_id.unwrap_or_default().into());
            d.set_checking_any(checking_any);
            if let Some(item) = current {
                d.set_printer_name(item.name);
                d.set_printer_light(item.light);
                d.set_printer_light_label(item.light_label);
                d.set_printer_paper(item.paper);
                d.set_printer_support(item.support);
                d.set_printer_support_tone(item.support_tone);
            }
            d.set_profile_ok(profile.is_some());
            if let Some(profile) = &profile {
                d.set_density_model(strings(profile.densities.iter().map(|x| capitalize(x.as_str())).collect()));
                d.set_feed_min(i32::from(profile.feed.min_mm));
                d.set_feed_max(i32::from(profile.feed.max_mm));
                d.set_max_copies(i32::from(profile.max_copies));
                d.set_feed_info(
                    format!(
                        "Paper moved forward after the last copy so it clears the tear bar. {} to {} mm; {} mm is the verified distance.",
                        profile.feed.min_mm, profile.feed.max_mm, profile.feed.default_mm
                    )
                    .into(),
                );
                if apply && let Some(printer) = &selected {
                    let index = profile.densities.iter().position(|x| *x == printer.settings.density).unwrap_or(0);
                    d.set_density_index(index as i32);
                    d.set_feed_mm(i32::from(printer.settings.feed_mm));
                }
            }
        });
    }

    pub(crate) fn sync_scan(&self) {
        let st = self.state.lock().unwrap();
        let devices: Vec<DeviceItem> = st
            .found
            .iter()
            .map(|d| {
                let candidate = d.candidates.first();
                DeviceItem {
                    id: d.discovery_id.clone().into(),
                    name: d.name.clone().unwrap_or_else(|| "Unnamed device".into()).into(),
                    meta: format!(
                        "{} · {}",
                        d.rssi.map(|r| format!("Signal {r} dBm")).unwrap_or_else(|| "Signal unknown".into()),
                        candidate.map(|c| c.display_name.clone()).unwrap_or_else(|| "No matching printer model".into())
                    )
                    .into(),
                    unknown: candidate.is_none(),
                    support: candidate.map(|c| support_label(c.support)).unwrap_or_default().into(),
                    support_tone: candidate.map(|c| support_tone(c.support)).unwrap_or(0),
                    saved: d.saved_printer_id.is_some(),
                    can_save: candidate.is_some() && d.saved_printer_id.is_none(),
                }
            })
            .collect();
        let adapter = adapter_text(st.adapter.unwrap_or(AdapterStatus::Unknown));
        let scanning = st.scan.is_some();
        let error = st.scan_error.clone();
        drop(st);
        self.ui(move |ui| {
            let d = ui.global::<Data>();
            d.set_devices(ModelRc::new(VecModel::from(devices)));
            d.set_scanning(scanning);
            d.set_adapter_label(adapter.0.into());
            d.set_adapter_text(adapter.1.into());
            d.set_adapter_tone(adapter.2);
            d.set_scan_error(error.into());
        });
    }

    /// The status bar shows the job you last sent, or the newest one still going.
    pub(crate) fn sync_jobs_bar(&self) {
        let job = {
            let st = self.state.lock().unwrap();
            st.tracked
                .as_ref()
                .and_then(|id| st.jobs.iter().find(|j| &j.id == id))
                .or_else(|| {
                    st.jobs
                        .iter()
                        .filter(|j| !j.state.is_terminal())
                        .max_by(|a, b| a.created_ms.total_cmp(&b.created_ms))
                })
                .cloned()
        };
        self.ui(move |ui| {
            let d = ui.global::<Data>();
            d.set_has_job(job.is_some());
            if let Some(job) = job {
                let progress =
                    if job.bytes_total > 0.0 { (job.bytes_sent / job.bytes_total).clamp(0.0, 1.0) as f32 } else { 0.0 };
                d.set_job_label(job_label(job.state).into());
                d.set_job_tone(job_tone(job.state));
                d.set_job_line(format!("{} · {}", job.title, job.printer_name).into());
                d.set_job_progress(progress);
                d.set_job_show_progress(job.state == JobState::Sending);
                d.set_job_cancellable(!job.state.is_terminal());
            }
        });
    }

    pub(crate) fn sync_diagnostics(&self) {
        let st = self.state.lock().unwrap();
        let mut app = Vec::new();
        if let Some(info) = &st.app_info {
            app.push(fact("Version", format!("NuPrint {}", info.version), false));
            app.push(fact("Renderer", info.renderer_version.clone(), false));
            app.push(fact("System", info.os.clone(), false));
            app.push(fact(
                "PDF",
                if info.pdf_available {
                    "Available".into()
                } else {
                    info.pdf_error.clone().unwrap_or("Not available".into())
                },
                false,
            ));
            app.push(fact("Data folder", info.data_dir.clone().unwrap_or_else(|| "In memory only".into()), true));
        }
        let mut checks = Vec::new();
        if let Some((id, c)) = &st.last_check {
            let name = st.printers.iter().find(|p| &p.id == id).map(|p| p.name.clone()).unwrap_or_default();
            checks.push(fact("Printer", name, false));
            checks.push(fact("Result", if c.ready { "Ready to print" } else { "Not ready" }, false));
            checks.push(fact("Status", c.status_text.clone(), false));
            checks.push(fact("Firmware", c.firmware.clone().unwrap_or_else(|| "Unknown".into()), false));
            checks.push(fact("Write size", format!("{} bytes", c.max_write_len), false));
            checks.push(fact("Raw status", c.raw_status_hex.clone(), true));
            checks.push(fact("Checked at", format!("{} UTC", time_of_day(c.checked_ms)), false));
        }
        let has_check = st.last_check.is_some();
        drop(st);
        self.ui(move |ui| {
            let d = ui.global::<Data>();
            d.set_app_facts(ModelRc::new(VecModel::from(app)));
            d.set_has_check(has_check);
            d.set_check_facts(ModelRc::new(VecModel::from(checks)));
        });
    }

    /// Add the new log lines to the activity log's rows, in one go for all the lines that arrived
    /// before the UI thread got to them, rather than rebuilding every row for each line.
    pub(crate) fn sync_logs(&self) {
        {
            let mut st = self.state.lock().unwrap();
            if st.logs_queued {
                return;
            }
            st.logs_queued = true;
        }
        let this = self.clone();
        self.ui(move |ui| {
            let logs = ui.global::<Data>().get_logs();
            let Some(rows) = logs.as_any().downcast_ref::<VecModel<LogItem>>() else { return };
            let merge = {
                let mut st = this.state.lock().unwrap();
                st.logs_queued = false;
                let lines = std::mem::take(&mut st.new_logs);
                merge_logs(&mut st.shown_logs, lines, MAX_LOG_LINES)
            };
            let item = |l: LogLine| {
                let (level, tone) = level_label(l.level);
                LogItem { time: time_of_day(l.at_ms).into(), level: level.into(), text: l.message.into(), tone }
            };
            for (at, line) in merge.inserts {
                rows.insert(at, item(line));
            }
            let kept = rows.row_count();
            if merge.trim >= kept {
                rows.set_vec(merge.appends.into_iter().skip(merge.trim - kept).map(item).collect::<Vec<_>>());
            } else {
                for _ in 0..merge.trim {
                    rows.remove(0);
                }
                rows.extend(merge.appends.into_iter().map(item));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(seq: f64) -> LogLine {
        LogLine { seq, at_ms: 0.0, level: LogLevel::Info, message: format!("line {seq}") }
    }

    fn seqs(lines: &[LogLine]) -> Vec<f64> {
        lines.iter().map(|l| l.seq).collect()
    }

    #[test]
    fn a_line_delivered_after_a_newer_one_is_kept_in_order() {
        let mut shown = VecDeque::new();
        let merge = merge_logs(&mut shown, vec![line(2.0)], 10);
        assert_eq!(seqs(&merge.appends), [2.0]);
        // Sequence 1 was assigned first but its event arrived second.
        let merge = merge_logs(&mut shown, vec![line(1.0)], 10);
        assert_eq!(merge.inserts.iter().map(|(at, l)| (*at, l.seq)).collect::<Vec<_>>(), [(0, 1.0)]);
        assert!(merge.appends.is_empty());
        assert_eq!(shown, [1.0, 2.0]);
    }

    #[test]
    fn repeated_lines_are_shown_once() {
        let mut shown = VecDeque::new();
        // The start-up snapshot and an event can carry the same line.
        let merge = merge_logs(&mut shown, vec![line(3.0), line(1.0), line(3.0)], 10);
        assert_eq!(seqs(&merge.appends), [1.0, 3.0]);
        let merge = merge_logs(&mut shown, vec![line(1.0), line(4.0)], 10);
        assert!(merge.inserts.is_empty());
        assert_eq!(seqs(&merge.appends), [4.0]);
        assert_eq!(shown, [1.0, 3.0, 4.0]);
    }

    #[test]
    fn only_the_newest_lines_stay() {
        let mut shown: VecDeque<f64> = [1.0, 2.0, 3.0].into();
        let merge = merge_logs(&mut shown, vec![line(5.0), line(4.0)], 3);
        assert_eq!(seqs(&merge.appends), [4.0, 5.0]);
        assert_eq!(merge.trim, 2);
        assert_eq!(shown, [3.0, 4.0, 5.0]);
    }
}
