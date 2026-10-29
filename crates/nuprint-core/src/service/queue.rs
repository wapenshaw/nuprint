//! Admitting jobs from preview snapshots and the queue's queries and controls.

use std::sync::atomic::Ordering;

use nuprint_protocol::{ExchangePlan, MonoBitmap, PrintRequest, PrinterDriver};
use nuprint_render::RENDERER_VERSION;
use uuid::Uuid;

use crate::api::*;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::jobs::summarize;
use crate::store::{JobRow, now_ms};

use super::logging::private;
use super::{AppService, MAX_QUEUE_JOBS, MAX_RETAINED_BYTES};

impl AppService {
    fn job_summary(&self, job: &JobRow) -> AppResult<JobSummary> {
        let pages = self.store().pages(&job.id)?;
        Ok(summarize(job, &pages))
    }

    pub(super) fn emit_job(&self, job: &JobRow) {
        if let Ok(summary) = self.job_summary(job) {
            let queue_held = self.inner.held.load(Ordering::Relaxed);
            self.emit(AppEvent::JobChanged { seq: self.next_seq(), job: summary, queue_held });
        }
    }

    /// One plan per copy and page in print order, as (copy, page index, plan). The extra
    /// feed is added once, after the last page of the last copy.
    pub(super) fn plan_pages(
        driver: &dyn PrinterDriver,
        job: &JobRow,
        pages: &[(u32, MonoBitmap)],
    ) -> AppResult<Vec<(u16, usize, ExchangePlan)>> {
        let mut plans = Vec::with_capacity(usize::from(job.copies) * pages.len());
        for copy in 0..job.copies {
            for (index, (_, bitmap)) in pages.iter().enumerate() {
                let last = copy + 1 == job.copies && index + 1 == pages.len();
                let request =
                    PrintRequest { bitmap, density: job.density, trailing_feed_mm: if last { job.feed_mm } else { 0 } };
                plans.push((copy, index, driver.plan_job(&request)?));
            }
        }
        Ok(plans)
    }

    /// Atomically admit a job for the displayed raster snapshot.
    ///
    /// The same `idempotency_key` returns the job already admitted for it, so a repeated
    /// submission never queues a second print. The job is planned completely before it is
    /// stored, and records the printer's exact profile revision.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidInput`] for a blank or over-long key or options the profile does not
    /// allow, [`ErrorCode::NotFound`] for an unknown printer or expired snapshot,
    /// [`ErrorCode::StalePreview`] when the snapshot was rendered for a different profile or
    /// revision than the printer now has, [`ErrorCode::QueueFull`] at the job or retained-data
    /// limit, and the converted protocol error when a page cannot be planned.
    pub fn enqueue(&self, request: EnqueueRequest) -> AppResult<JobSummary> {
        if request.idempotency_key.trim().is_empty() || request.idempotency_key.len() > 128 {
            return Err(AppError::invalid("A submission key is required."));
        }
        let existing = self.store().find_job_by_key(&request.idempotency_key)?;
        if let Some(existing) = existing {
            return self.job_summary(&existing);
        }
        let snapshot = self.snapshot(&request.snapshot_id)?;
        let printer = self.printer_row(&request.printer_id)?;
        let resolved = self.printer_profile(&printer)?;
        if snapshot.profile_id != printer.profile_id || snapshot.profile_revision != printer.profile_revision {
            return Err(AppError::new(
                ErrorCode::StalePreview,
                "The preview was made for a different printer model. Wait for it to refresh, then print.",
            ));
        }
        resolved.check_job_options(request.density, request.feed_mm, Some(request.copies))?;
        let title: String = request.title.trim().chars().filter(|c| !c.is_control()).take(120).collect();
        let now = now_ms();
        let mut job = JobRow {
            id: Uuid::new_v4().to_string(),
            idempotency_key: request.idempotency_key,
            title: if title.is_empty() { "Untitled".into() } else { title },
            source_kind: snapshot.source_kind.into(),
            printer_id: printer.id.clone(),
            printer_name: printer.name.clone(),
            backend_id: printer.backend_id,
            profile_id: resolved.profile.id.clone(),
            profile_revision: resolved.profile.revision,
            protocol_id: resolved.driver.protocol_id().into(),
            protocol_version: resolved.driver.protocol_version(),
            state: JobState::Queued,
            created_ms: now,
            updated_ms: now,
            copies: request.copies,
            density: request.density,
            feed_mm: request.feed_mm,
            dots_per_mm: resolved.profile.raster.dots_per_mm,
            renderer_version: RENDERER_VERSION.into(),
            settings_json: snapshot.settings_json.clone(),
            content_retained: true,
            bytes_total: 0,
            bytes_sent: 0,
            error: None,
            partial_output: false,
            retry_of: None,
            revision: 0,
        };
        let plans = Self::plan_pages(resolved.driver.as_ref(), &job, &snapshot.pages)?;
        job.bytes_total = plans.iter().map(|(_, _, plan)| plan.print_bytes() as i64).sum();
        self.admit(job, &snapshot.pages)
    }

    fn admit(&self, mut job: JobRow, pages: &[(u32, MonoBitmap)]) -> AppResult<JobSummary> {
        // A job added now would wait for a queue that cannot run until the app restarts.
        if let Some(error) = self.fault_error() {
            return Err(error);
        }
        let mut store = self.store();
        if store.count_jobs_with_content()? >= MAX_QUEUE_JOBS {
            return Err(AppError::new(
                ErrorCode::QueueFull,
                "The queue is full. Remove finished jobs or clear history.",
            ));
        }
        let new_bytes: i64 = pages.iter().map(|(_, b)| b.data().len() as i64).sum();
        if store.retained_content_bytes()? + new_bytes > MAX_RETAINED_BYTES {
            return Err(AppError::new(
                ErrorCode::QueueFull,
                "Saved print data is at its size limit. Clear history first.",
            ));
        }
        job.revision = store.next_revision();
        let driver = self.inner.registry.get_revision(&job.profile_id, job.profile_revision).map(|p| p.driver);
        let per_page: Vec<i64> = pages
            .iter()
            .map(|(_, bitmap)| {
                let request = PrintRequest { bitmap, density: job.density, trailing_feed_mm: 0 };
                driver.as_ref().and_then(|d| d.plan_job(&request).ok()).map_or(0, |p| p.print_bytes() as i64)
            })
            .collect();
        store.insert_job(&job, pages, &per_page)?;
        drop(store);
        self.log(
            LogLevel::Info,
            format!(
                "Queued \"{}\" for {} ({} cop{})",
                private(&job.title),
                private(&job.printer_name),
                job.copies,
                if job.copies == 1 { "y" } else { "ies" }
            ),
        );
        self.emit_job(&job);
        self.inner.wake.notify_one();
        self.job_summary(&job)
    }

    pub fn list_jobs(&self) -> AppResult<QueueSnapshot> {
        let store = self.store();
        let jobs = store.list_jobs_with_pages(200)?.iter().map(|(row, pages)| summarize(row, pages)).collect();
        let seq = self.inner.seq.load(Ordering::SeqCst) as f64;
        Ok(QueueSnapshot {
            held: self.inner.held.load(Ordering::Relaxed) || self.inner.fault.get().is_some(),
            revision: store.revision() as f64,
            seq,
            jobs,
            fault: self.inner.fault.get().cloned(),
        })
    }

    pub fn get_job(&self, job_id: &str) -> AppResult<JobSummary> {
        let job = self.store().get_job(job_id)?.ok_or_else(|| AppError::not_found("Job"))?;
        self.job_summary(&job)
    }

    /// Acknowledge cancellation intent. An active job reaches its terminal state after
    /// the transfer task unwinds and Bluetooth cleanup completes.
    ///
    /// Ownership depends on the phase. A queued or held job is cancelled by the store in one
    /// checked transition, so it cannot overwrite a job the dispatcher claimed first. The active
    /// job is only signalled here; the dispatcher alone writes its terminal state after
    /// Bluetooth cleanup, so this returning `Ok` does not mean the job has already stopped.
    /// Cancelling after print bytes were sent ends as cancelled with possible partial output.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotFound`] for an unknown job and [`ErrorCode::InvalidInput`] when the job
    /// is in its final moments and no longer cancellable. Cancelling a job that is already
    /// finished succeeds without effect.
    pub fn cancel_job(&self, job_id: &str) -> AppResult<()> {
        if self.cancel_active(job_id) {
            self.log(LogLevel::Info, "Cancelling transfer…".into());
            return Ok(());
        }
        let cancelled = self.store().cancel_unstarted(job_id)?;
        if let Some(job) = cancelled {
            self.emit_job(&job);
            return Ok(());
        }
        // Not queued or held any more. The dispatcher marks a job active before it claims it, so a
        // claim that just won the race is visible here.
        if self.cancel_active(job_id) {
            self.log(LogLevel::Info, "Cancelling transfer…".into());
            return Ok(());
        }
        let job = self.store().get_job(job_id)?.ok_or_else(|| AppError::not_found("Job"))?;
        if job.state.is_terminal() {
            Ok(())
        } else {
            Err(AppError::invalid("This job is finishing; it cannot be cancelled now."))
        }
    }

    pub fn retry_job(&self, job_id: &str, idempotency_key: String) -> AppResult<JobSummary> {
        let existing = self.store().find_job_by_key(&idempotency_key)?;
        if let Some(existing) = existing {
            return self.job_summary(&existing);
        }
        let (original, pages) = {
            let store = self.store();
            let original = store.get_job(job_id)?.ok_or_else(|| AppError::not_found("Job"))?;
            if !original.state.is_terminal() {
                return Err(AppError::invalid("Only finished jobs can be printed again."));
            }
            if !original.content_retained {
                return Err(AppError::invalid(
                    "This job's print data was not kept. Open the source and print it again.",
                ));
            }
            let pages = store.load_pages(job_id)?;
            (original, pages)
        };
        if self.inner.registry.get_revision(&original.profile_id, original.profile_revision).is_none() {
            return Err(AppError::new(
                ErrorCode::UnsupportedProfile,
                "This printer profile changed; print from the source again.",
            ));
        }
        let now = now_ms();
        let job = JobRow {
            id: Uuid::new_v4().to_string(),
            idempotency_key,
            state: JobState::Queued,
            created_ms: now,
            updated_ms: now,
            content_retained: true,
            bytes_sent: 0,
            error: None,
            partial_output: false,
            retry_of: Some(original.id.clone()),
            ..original
        };
        self.admit(job, &pages)
    }

    pub fn remove_job(&self, job_id: &str) -> AppResult<()> {
        let mut store = self.store();
        let job = store.get_job(job_id)?.ok_or_else(|| AppError::not_found("Job"))?;
        if !job.state.is_terminal() {
            return Err(AppError::invalid("Cancel the job before removing it."));
        }
        store.delete_job(job_id)?;
        drop(store);
        self.emit(AppEvent::JobRemoved { seq: self.next_seq(), job_id: job_id.to_owned() });
        Ok(())
    }

    pub fn clear_history(&self) -> AppResult<()> {
        let mut store = self.store();
        let removed: Vec<String> =
            store.list_jobs(100_000)?.into_iter().filter(|j| j.state.is_terminal()).map(|j| j.id).collect();
        for id in &removed {
            store.delete_job(id)?;
        }
        drop(store);
        for job_id in removed {
            self.emit(AppEvent::JobRemoved { seq: self.next_seq(), job_id });
        }
        Ok(())
    }

    pub fn hold_queue(&self) -> AppResult<QueueSnapshot> {
        self.inner.held.store(true, Ordering::Relaxed);
        self.store().set_pref("queue_held", &true)?;
        self.log(LogLevel::Info, "Queue held; the current job (if any) continues".into());
        self.list_jobs()
    }

    pub fn resume_queue(&self) -> AppResult<QueueSnapshot> {
        if let Some(error) = self.fault_error() {
            return Err(error);
        }
        self.inner.held.store(false, Ordering::Relaxed);
        let released = {
            let mut store = self.store();
            store.set_pref("queue_held", &false)?;
            let mut released = Vec::new();
            for mut job in store.list_jobs(100_000)?.into_iter().filter(|j| j.state == JobState::Held) {
                job.state = JobState::Queued;
                store.update_job(&mut job)?;
                released.push(job);
            }
            released
        };
        for job in &released {
            self.emit_job(job);
        }
        self.inner.wake.notify_one();
        self.list_jobs()
    }
}
