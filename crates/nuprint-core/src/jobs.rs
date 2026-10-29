//! Job presentation helpers and outcome classification (docs/technical-spec.md §4).

use nuprint_protocol::CompletionPolicy;

use crate::api::{JobState, JobSummary};
use crate::error::{AppError, ErrorCode};
use crate::store::{JobRow, PageRow};

/// Plain-language state text. Never claims the paper was optically verified.
pub fn state_text(job: &JobRow) -> String {
    match job.state {
        JobState::Held => "Held · resume the queue to print".into(),
        JobState::Queued => "Waiting to print".into(),
        JobState::Connecting => "Connecting to printer…".into(),
        JobState::Sending => "Sending data to printer…".into(),
        JobState::WaitingForReady => "Data sent · waiting for printer".into(),
        JobState::Completed => "Transfer complete · printer reports ready".into(),
        JobState::TransferUnconfirmed => "Data sent · output unconfirmed".into(),
        JobState::Accepted => "Printer accepted the job".into(),
        JobState::Failed => "Failed before any print data was sent".into(),
        JobState::Uncertain => "Uncertain · some paper may have printed; check before retrying".into(),
        JobState::Cancelled if job.partial_output => "Cancelled · data already in the printer may still print".into(),
        JobState::Cancelled => "Cancelled".into(),
    }
}

pub fn summarize(job: &JobRow, pages: &[PageRow]) -> JobSummary {
    let height: u32 = pages.iter().map(|p| p.height).sum();
    let width = pages.first().map_or(0, |p| p.width);
    let raster_mm = height as f32 / job.dots_per_mm.max(f32::EPSILON);
    JobSummary {
        id: job.id.clone(),
        title: job.title.clone(),
        state: job.state,
        state_text: state_text(job),
        printer_id: job.printer_id.clone(),
        printer_name: job.printer_name.clone(),
        profile_id: job.profile_id.clone(),
        profile_revision: job.profile_revision,
        created_ms: job.created_ms as f64,
        updated_ms: job.updated_ms as f64,
        copies: job.copies,
        pages: pages.len() as u32,
        width,
        height,
        density: job.density,
        feed_mm: job.feed_mm,
        paper_mm: paper_mm(raster_mm, job.copies, job.feed_mm),
        bytes_total: job.bytes_total as f64,
        bytes_sent: job.bytes_sent as f64,
        error: job.error.clone(),
        partial_output_possible: job.partial_output,
        content_retained: job.content_retained,
        retry_of: job.retry_of.clone(),
        revision: job.revision as f64,
    }
}

/// Raster length of one copy × copies, plus the trailing feed applied once at the end.
pub fn paper_mm(raster_mm_per_copy: f32, copies: u16, feed_mm: u16) -> f32 {
    raster_mm_per_copy * f32::from(copies) + f32::from(feed_mm)
}

/// Terminal state for a finished attempt, and whether paper may already have moved.
///
/// `print_bytes_at_risk` is the session's [`at-risk count`](crate::session::PrinterSession::print_bytes_at_risk):
/// confirmed writes plus a write that failed or was cancelled midway, since a failed write does
/// not prove nothing arrived. The job's own `bytes_sent` is the smaller confirmed count and is
/// never used here. A failure with any at-risk byte is [`JobState::Uncertain`] and is not
/// replayed automatically; with none it is a clean [`JobState::Failed`].
///
/// ```
/// use nuprint_core::api::JobState;
/// use nuprint_core::jobs::classify;
/// use nuprint_core::{AppError, ErrorCode};
/// use nuprint_protocol::CompletionPolicy;
///
/// let dropped = Err(AppError::new(ErrorCode::ConnectionFailed, "link lost"));
/// // Nothing was written: a plain failure, safe to retry.
/// assert_eq!(classify(&dropped, 0), (JobState::Failed, false));
/// // A write that failed midway may still have delivered bytes: uncertain, even though the
/// // confirmed count (`bytes_sent`) could be zero.
/// assert_eq!(classify(&dropped, 1), (JobState::Uncertain, true));
///
/// let cancelled = Err(AppError::cancelled());
/// assert_eq!(classify(&cancelled, 0), (JobState::Cancelled, false));
/// assert_eq!(classify(&cancelled, 48), (JobState::Cancelled, true));
///
/// // Success wording follows what the model can confirm, never that paper came out.
/// assert_eq!(classify(&Ok(CompletionPolicy::StatusConfirmedReady), 48), (JobState::Completed, false));
/// assert_eq!(classify(&Ok(CompletionPolicy::UnconfirmedTransfer), 48), (JobState::TransferUnconfirmed, false));
/// ```
pub fn classify(result: &Result<CompletionPolicy, AppError>, print_bytes_sent: usize) -> (JobState, bool) {
    match result {
        Ok(CompletionPolicy::StatusConfirmedReady) => (JobState::Completed, false),
        Ok(CompletionPolicy::AcknowledgedAcceptance) => (JobState::Accepted, false),
        Ok(CompletionPolicy::UnconfirmedTransfer) => (JobState::TransferUnconfirmed, false),
        Err(e) if e.code == ErrorCode::Cancelled => (JobState::Cancelled, print_bytes_sent > 0),
        Err(_) if print_bytes_sent == 0 => (JobState::Failed, false),
        Err(_) => (JobState::Uncertain, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_is_added_once_after_all_copies() {
        assert_eq!(paper_mm(20.5, 1, 12), 32.5);
        assert_eq!(paper_mm(20.0, 3, 12), 72.0);
        assert_eq!(paper_mm(20.0, 2, 0), 40.0);
    }

    #[test]
    fn failures_after_print_bytes_are_uncertain() {
        let fail = Err(AppError::new(ErrorCode::Disconnected, "x"));
        assert_eq!(classify(&fail, 0), (JobState::Failed, false));
        assert_eq!(classify(&fail, 1), (JobState::Uncertain, true));
        let cancel = Err(AppError::new(ErrorCode::Cancelled, "x"));
        assert_eq!(classify(&cancel, 0), (JobState::Cancelled, false));
        assert_eq!(classify(&cancel, 10), (JobState::Cancelled, true));
        assert_eq!(classify(&Ok(CompletionPolicy::StatusConfirmedReady), 10), (JobState::Completed, false));
        assert_eq!(classify(&Ok(CompletionPolicy::UnconfirmedTransfer), 10), (JobState::TransferUnconfirmed, false));
    }
}
