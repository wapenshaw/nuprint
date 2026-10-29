//! The dispatcher: claims queued jobs, holds the Bluetooth lock, connects and prints.

use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use nuprint_protocol::CompletionPolicy;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::api::*;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::jobs::classify;
use crate::profile::ResolvedProfile;
use crate::session::{PrinterSession, SessionEvent, locate};
use crate::store::JobRow;

#[cfg(feature = "fault-injection")]
use super::FaultPoint;
use super::logging::private;
use super::{AppService, relock};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);

pub(super) struct ActiveJob {
    pub(super) job_id: String,
    pub(super) cancel: CancellationToken,
}

#[cfg(feature = "fault-injection")]
macro_rules! fault_point {
    ($service:expr, $point:ident) => {
        $service.maybe_panic(FaultPoint::$point)
    };
}
#[cfg(not(feature = "fault-injection"))]
macro_rules! fault_point {
    ($service:expr, $point:ident) => {};
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HardwareUse {
    Printing,
    Checking,
    Inspecting,
}

/// The Bluetooth lock, recording what holds it until dropped.
pub(super) struct HardwareClaim<'a> {
    _guard: tokio::sync::MutexGuard<'a, ()>,
    holder: &'a Mutex<Option<HardwareUse>>,
}

impl Drop for HardwareClaim<'_> {
    fn drop(&mut self) {
        // Runs before `_guard` is released, so it never clears a newer holder.
        *relock(self.holder) = None;
    }
}

fn busy(holder: Option<HardwareUse>) -> AppError {
    let message = match holder {
        Some(HardwareUse::Printing) => "A print is in progress. Try again when it finishes.",
        Some(HardwareUse::Checking) => "A connection check is in progress. Try again when it finishes.",
        Some(HardwareUse::Inspecting) => "A device inspection is in progress. Try again when it finishes.",
        None => "Another Bluetooth operation is in progress. Try again when it finishes.",
    };
    AppError::new(ErrorCode::PrinterBusy, message)
}

impl AppService {
    /// Take the Bluetooth lock for a user action. Only an inspection is waited for, because it is
    /// short and bounded; behind a print or a check the action is refused as busy instead.
    pub(super) async fn claim_hardware(
        &self,
        purpose: HardwareUse,
        cancel: &CancellationToken,
    ) -> AppResult<HardwareClaim<'_>> {
        // After a dispatcher failure the old Bluetooth session may still be open.
        if let Some(error) = self.fault_error() {
            return Err(error);
        }
        let guard = match self.inner.hardware.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                let holder = *relock(&self.inner.hardware_use);
                if holder != Some(HardwareUse::Inspecting) {
                    return Err(busy(holder));
                }
                tokio::select! {
                    guard = self.inner.hardware.lock() => guard,
                    _ = cancel.cancelled() => return Err(AppError::cancelled()),
                }
            }
        };
        Ok(self.hold_hardware(guard, purpose))
    }

    fn hold_hardware<'a>(&'a self, guard: tokio::sync::MutexGuard<'a, ()>, purpose: HardwareUse) -> HardwareClaim<'a> {
        *relock(&self.inner.hardware_use) = Some(purpose);
        HardwareClaim { _guard: guard, holder: &self.inner.hardware_use }
    }

    /// Connect to a previously seen device, rescanning for it if the OS lost track of it.
    pub(super) async fn connect_session(
        &self,
        resolved: &ResolvedProfile,
        backend_id: &str,
        cancel: &CancellationToken,
    ) -> AppResult<PrinterSession> {
        let timeouts = &resolved.profile.transport.timeouts;
        let adv = locate(self.inner.transport.as_ref(), Duration::from_millis(timeouts.find_ms), cancel, |a| {
            a.backend_id == backend_id
        })
        .await?;
        // Revalidate identity: a device under this ID must still match the profile's name rules.
        // The message names neither the device nor an edited profile, because errors are logged
        // and kept with jobs.
        if let Some(name) = &adv.local_name
            && !resolved.profile.matches_name(name)
        {
            return Err(AppError::new(
                ErrorCode::UnsupportedProfile,
                "The device now advertises a different name, which does not match this printer's profile.",
            ));
        }
        relock(&self.inner.discovered).insert(adv.backend_id.clone(), adv);
        PrinterSession::open(self.inner.transport.as_ref(), resolved, backend_id, self.logger(), cancel).await
    }

    pub(super) async fn dispatch_loop(self) {
        loop {
            while !self.inner.held.load(Ordering::Relaxed)
                && !self.inner.shutdown.is_cancelled()
                && self.inner.fault.get().is_none()
            {
                let next = self.store().next_queued();
                match next {
                    Ok(Some(job)) => self.run_job(job).await,
                    Ok(None) => break,
                    Err(e) => {
                        self.log(LogLevel::Error, format!("Queue error: {e}"));
                        break;
                    }
                }
            }
            tokio::select! {
                _ = self.inner.shutdown.cancelled() => return,
                _ = self.inner.wake.notified() => {}
            }
            #[cfg(feature = "fault-injection")]
            if self.take_injected(FaultPoint::StopDispatcher) {
                return;
            }
        }
    }

    /// Record and announce a state. `false` means it could not be saved; the change is still
    /// announced so the screen shows what is happening, but the caller decides what to do.
    fn set_state(&self, job: &mut JobRow, state: JobState) -> bool {
        job.state = state;
        let saved = match self.store().update_job(job) {
            Ok(()) => true,
            Err(e) => {
                self.log(LogLevel::Error, format!("Could not save job state: {e}"));
                false
            }
        };
        self.emit_job(job);
        saved
    }

    /// Stop starting jobs, for this run only, when the queue can no longer be saved reliably.
    /// Printing on without a record could resend a job that was already tried.
    fn pause_queue_for_storage_error(&self, what: &str) {
        self.inner.held.store(true, Ordering::Relaxed);
        self.log(LogLevel::Error, format!("Queue paused: {what} could not be saved. Restart NuPrint to continue."));
    }

    pub(super) fn cancel_active(&self, job_id: &str) -> bool {
        let active = relock(&self.inner.active);
        match active.as_ref() {
            Some(active) if active.job_id == job_id => {
                active.cancel.cancel();
                true
            }
            _ => false,
        }
    }

    async fn run_job(&self, queued: JobRow) {
        let cancel = self.inner.shutdown.child_token();
        // Active before claimed: a cancel that misses the queued row still finds the job here.
        *relock(&self.inner.active) = Some(ActiveJob { job_id: queued.id.clone(), cancel: cancel.clone() });
        fault_point!(self, BeforeClaim);
        let hardware = tokio::select! {
            guard = self.inner.hardware.lock() => Some(self.hold_hardware(guard, HardwareUse::Printing)),
            _ = cancel.cancelled() => None,
        };
        let Some(hardware) = hardware else {
            // Cancelled while waiting for Bluetooth; nothing was claimed or sent.
            let cancelled = self.store().cancel_unstarted(&queued.id);
            match cancelled {
                Ok(Some(job)) => self.emit_job(&job),
                Ok(None) => {}
                Err(e) => self.log(LogLevel::Error, format!("Could not save the cancellation: {e}")),
            }
            *relock(&self.inner.active) = None;
            return;
        };
        // Claim the job durably before any Bluetooth I/O. If it is no longer queued, or the claim
        // cannot be saved, nothing is sent.
        let attempt_id = Uuid::new_v4().to_string();
        let claimed = self.store().claim_queued(&queued.id, &attempt_id);
        let mut job = match claimed {
            Ok(Some(job)) => job,
            Ok(None) => {
                *relock(&self.inner.active) = None;
                return;
            }
            Err(e) => {
                self.log(LogLevel::Error, format!("Could not claim a job: {e}"));
                self.pause_queue_for_storage_error("the start of a job");
                *relock(&self.inner.active) = None;
                return;
            }
        };
        fault_point!(self, AfterClaim);
        self.emit_job(&job);
        let (result, sent) = self.execute_job(&mut job, &attempt_id, &cancel).await;
        #[cfg(feature = "fault-injection")]
        self.hold_cleanup().await;
        drop(hardware);
        // `sent` counts bytes that may have arrived, which can exceed what was confirmed written;
        // `job.bytes_sent` keeps the confirmed count.
        let (state, partial) = classify(&result, sent);
        job.partial_output = partial;
        job.error = match (&result, state) {
            (Err(_), JobState::Cancelled) if partial => Some(AppError::new(
                ErrorCode::Cancelled,
                "Cancelled. Data already in the printer may still print; check the paper before printing again.",
            )),
            (Err(_), JobState::Cancelled) => None,
            // An OutcomeUncertain error already says what to check.
            (Err(e), JobState::Uncertain) if e.code != ErrorCode::OutcomeUncertain => Some(AppError {
                code: e.code,
                message: format!("{} Some paper may already have printed; check before retrying.", e.message),
                detail: e.detail.clone(),
            }),
            (Err(e), _) => Some(e.clone()),
            (Ok(_), _) => None,
        };
        {
            let store = self.store();
            fault_point!(self, WhilePersisting);
            if let Err(e) = store.finish_attempt(&attempt_id, state, result.as_ref().err().map(|e| e.code)) {
                self.log(LogLevel::Error, format!("Could not record the attempt outcome: {e}"));
            }
            let _ = store.add_event(&attempt_id, "outcome", state.as_str());
        }
        // If this is lost, the job stays in its in-progress state and a restart marks it uncertain.
        if !self.set_state(&mut job, state) {
            self.pause_queue_for_storage_error("the result of a job");
        }
        if state == JobState::Completed {
            // The print data is deleted; a preview of the first page and the details stay until
            // the job is removed or its 30 days are up.
            let thumbnail = self.job_page_png(&job.id);
            let mut store = self.store();
            if let Err(e) = store.drop_content(&mut job, thumbnail.as_deref()) {
                self.log(LogLevel::Warn, format!("Could not delete completed job data: {e}"));
            }
            drop(store);
            self.emit_job(&job);
        }
        match &result {
            Ok(_) => {
                self.log(LogLevel::Info, format!("\"{}\": {}", private(&job.title), crate::jobs::state_text(&job)))
            }
            Err(e) => self.log(LogLevel::Error, format!("\"{}\": {}", private(&job.title), e.message)),
        }
        *relock(&self.inner.active) = None;
    }

    /// Returns the outcome and the number of print bytes that may have reached the printer.
    async fn execute_job(
        &self,
        job: &mut JobRow,
        attempt_id: &str,
        cancel: &CancellationToken,
    ) -> (Result<CompletionPolicy, AppError>, usize) {
        let Some(resolved) = self.inner.registry.get_revision(&job.profile_id, job.profile_revision) else {
            return (
                Err(AppError::new(
                    ErrorCode::UnsupportedProfile,
                    "This job's printer profile is not in this version; print it again from the source.",
                )),
                0,
            );
        };
        // Load, verify and encode every page before opening Bluetooth.
        let pages = self.store().load_pages(&job.id);
        let plans = match pages.and_then(|pages| Self::plan_pages(resolved.driver.as_ref(), job, &pages)) {
            Ok(plans) => plans,
            Err(e) => return (Err(e), 0),
        };
        let completion = plans.last().map_or(CompletionPolicy::StatusConfirmedReady, |(_, _, p)| p.completion);
        let total: usize = plans.iter().map(|(_, _, p)| p.print_bytes()).sum();
        job.bytes_total = total as i64;
        let mut session = match self.connect_session(&resolved, &job.backend_id, cancel).await {
            Ok(session) => session,
            Err(e) => return (Err(e), 0),
        };
        let mut base = 0usize;
        let mut last_emit = Instant::now() - PROGRESS_INTERVAL;
        let mut result = Ok(completion);
        let job_id = job.id.clone();
        for (copy, index, plan) in &plans {
            let outcome = {
                let job = &mut *job;
                let mut on_event = |event: SessionEvent| match event {
                    SessionEvent::Phase(state) => {
                        if job.state != state {
                            self.set_state(job, state);
                        }
                    }
                    SessionEvent::Progress { sent, total: _ } => {
                        let now = Instant::now();
                        let done = base + sent;
                        if now.duration_since(last_emit) >= PROGRESS_INTERVAL || done == total {
                            last_emit = now;
                            self.emit(AppEvent::JobProgress {
                                seq: self.next_seq(),
                                job_id: job_id.clone(),
                                attempt_id: attempt_id.to_owned(),
                                bytes_sent: done as f64,
                                bytes_total: total as f64,
                            });
                        }
                    }
                };
                session.execute(plan, cancel, &mut on_event).await
            };
            job.bytes_sent = session.print_bytes_submitted() as i64;
            let _ = self.store().update_attempt(
                attempt_id,
                job.state,
                *index as u32,
                u32::from(*copy),
                job.bytes_sent,
                session.last_status().map(|s| hex::encode(s.raw)).as_deref(),
            );
            base += plan.print_bytes();
            if let Err(e) = outcome {
                result = Err(e);
                break;
            }
        }
        let at_risk = session.print_bytes_at_risk();
        session.close().await;
        (result, at_risk)
    }
}
