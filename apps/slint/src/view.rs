//! Plain data to screen items: job and printer-check views and the small builders the `sync_*`
//! functions share.

use nuprint_core::api::*;
use slint::{ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use crate::labels::*;
use crate::state::CheckView;
use crate::{Fact, JobItem};

pub(crate) fn decode_png(bytes: &[u8]) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let rgba = image::load_from_memory(bytes).ok()?.to_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()))
}

pub(crate) fn strings(items: Vec<String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(items.into_iter().map(SharedString::from).collect::<Vec<_>>()))
}

pub(crate) fn fact(key: &str, value: impl Into<String>, mono: bool) -> Fact {
    Fact { key: key.into(), value: value.into().into(), mono }
}

/// A job and its preview as plain data: `slint::Image` is not `Send`, so the item is built on
/// the UI thread.
pub(crate) struct JobView {
    pub(crate) job: JobSummary,
    pub(crate) thumb: Option<SharedPixelBuffer<Rgba8Pixel>>,
}

impl JobView {
    pub(crate) fn into_item(self) -> JobItem {
        let mut item = job_item(&self.job);
        if let Some(thumb) = self.thumb {
            item.has_thumb = true;
            item.thumb = slint::Image::from_rgba8(thumb);
        }
        item
    }
}

pub(crate) fn job_item(job: &JobSummary) -> JobItem {
    let progress = if job.bytes_total > 0.0 { (job.bytes_sent / job.bytes_total).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let terminal = job.state.is_terminal();
    JobItem {
        id: job.id.clone().into(),
        title: job.title.clone().into(),
        state: job.state.as_str().into(),
        label: job_label(job.state).into(),
        tone: job_tone(job.state),
        meta: format!(
            "{} · {} · {} · {}",
            job.printer_name,
            plural(job.copies as usize, "copy", "copies"),
            capitalize(job.density.as_str()),
            format_mm(job.paper_mm)
        )
        .into(),
        text: job.state_text.clone().into(),
        error: job.error.as_ref().map(|e| e.message.clone()).unwrap_or_default().into(),
        partial: job.partial_output_possible,
        progress,
        show_progress: job.state == JobState::Sending,
        progress_text: format!("{:.0}% sent", progress * 100.0).into(),
        has_thumb: false,
        thumb: Default::default(),
        blank_height: (job.height as f32 / 8.0).clamp(20.0, 80.0),
        can_cancel: !terminal,
        can_retry: terminal && job.content_retained && job.state != JobState::Completed,
        can_remove: terminal,
    }
}

pub(crate) fn check_view(check: &PrinterCheck) -> CheckView {
    CheckView {
        ok: check.ready,
        line: if check.ready { "Ready to print.".into() } else { format!("Not ready: {}", check.status_text) },
        flags: check.flags.iter().map(|f| f.label()).collect::<Vec<_>>().join(", "),
        firmware: check.firmware.clone().unwrap_or_default(),
        error: String::new(),
        paper: !check.flags.contains(&StatusFlag::PaperOut),
    }
}
