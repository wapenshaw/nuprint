//! Shutdown, the dispatcher's supervisor, and failing the service closed.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::api::*;
use crate::store::now_ms;

use super::{AppService, ShutdownOutcome, relock};

impl AppService {
    /// Stop accepting work, cancel the active transfer and wait (bounded) for its outcome
    /// to be persisted and the dispatcher to end. Pending jobs stay queued/held for the next
    /// start.
    ///
    /// `wait` bounds each of two waits: the active job reaching a terminal state (cancelled, or
    /// uncertain if print bytes may already have been sent), then the dispatcher's supervisor
    /// exiting. A job that never settles is left for restart recovery, which marks it uncertain;
    /// it is never replayed. On [`ShutdownOutcome::TimedOut`] nothing is released: the dispatcher
    /// (which owns the Bluetooth session) and the supervisor keep the service, so the data folder
    /// stays locked until their cleanup really ends. Last, the log file is finished, for up to two
    /// seconds; when the disk stalls longer this also returns `TimedOut`, and the log writer keeps
    /// the data folder until its write completes. Safe to call more than once.
    pub async fn shutdown(&self, wait: Duration) -> ShutdownOutcome {
        self.inner.shutdown.cancel();
        if let Some(active) = relock(&self.inner.active).as_ref() {
            active.cancel.cancel();
        }
        let deadline = Instant::now() + wait;
        while relock(&self.inner.active).is_some() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let task = relock(&self.inner.supervisor).take();
        if let Some(mut task) = task {
            match tokio::time::timeout(wait, &mut task).await {
                Err(_) => {
                    *relock(&self.inner.supervisor) = Some(task);
                    return ShutdownOutcome::TimedOut;
                }
                // The supervisor itself died; the dispatcher's result was never classified.
                Ok(Err(_)) => self.fail_closed(ServiceFaultKind::DispatcherStopped),
                Ok(Ok(())) => {}
            }
        }
        if relock(&self.inner.active).is_some() {
            return ShutdownOutcome::TimedOut;
        }
        // The dispatcher has ended; later lines stay in memory. On a timeout the file stays open,
        // because the dispatcher may still log while it cleans up.
        if !self.close_log_file().await {
            return ShutdownOutcome::TimedOut;
        }
        if self.inner.fault.get().is_some() { ShutdownOutcome::Faulted } else { ShutdownOutcome::Clean }
    }

    /// Watch the dispatcher. It ends only at a requested shutdown; any other end is a failure.
    pub(super) async fn supervise(self, worker: tokio::task::JoinHandle<()>) {
        let kind = match worker.await {
            Ok(()) if self.inner.shutdown.is_cancelled() => return,
            Err(e) if e.is_panic() => ServiceFaultKind::DispatcherPanicked,
            Err(_) if self.inner.shutdown.is_cancelled() => return,
            Ok(()) | Err(_) => ServiceFaultKind::DispatcherStopped,
        };
        self.fail_closed(kind);
    }

    /// Stop the service for good after a failure: record it (lock-free, first failure wins),
    /// hold the queue, and reconcile the claimed attempt. Never panics and never requeues.
    ///
    /// Bluetooth is not released afterwards. Dropping the failed task is not proof that the native
    /// connection ended, so `claim_hardware` refuses until the app restarts (increment B of the
    /// follow-up plan may allow a bounded in-process recovery).
    pub(super) fn fail_closed(&self, kind: ServiceFaultKind) {
        // A dead dispatcher can no longer finish its job, so its slot is released here. On a
        // storage failure the dispatcher is still running and keeps its slot until it unwinds.
        let dispatcher_gone = kind != ServiceFaultKind::StorageInvalid;
        let job_id = {
            let mut active = relock(&self.inner.active);
            let id = active.as_ref().map(|a| {
                a.cancel.cancel();
                a.job_id.clone()
            });
            if dispatcher_gone {
                *active = None;
            }
            id
        };
        let fault = ServiceFault {
            kind,
            message: match kind {
                ServiceFaultKind::StorageInvalid => {
                    "NuPrint's saved queue could not be checked after an internal error. Restart NuPrint; \
                     nothing will be printed until then."
                }
                _ => {
                    "NuPrint's print queue stopped working. Nothing will be printed or connected until you \
                     restart NuPrint. Waiting jobs are kept. If a job was printing, check the paper before \
                     printing it again."
                }
            }
            .into(),
            at_ms: now_ms() as f64,
            job_id: job_id.clone(),
        };
        if self.inner.fault.set(fault.clone()).is_err() {
            return;
        }
        self.inner.held.store(true, Ordering::Relaxed);
        // The rest touches locks and callbacks that may be the reason for the failure.
        let _ = catch_unwind(AssertUnwindSafe(|| {
            // On a storage failure the caller may hold the poisoned store lock; leave the job for
            // the restart recovery rather than lock it again here.
            let note = match &job_id {
                Some(job_id) if dispatcher_gone => self.reconcile_failed_attempt(job_id),
                Some(_) => "the active job stays active and a restart will mark it uncertain",
                None => "no job was active",
            };
            self.log(LogLevel::Error, format!("Queue stopped ({kind:?}); {note}. Restart NuPrint."));
            self.emit(AppEvent::QueueFault { seq: self.next_seq(), fault });
        }));
    }

    /// Mark the attempt the failed dispatcher had claimed as uncertain, if storage can be saved.
    /// Otherwise it stays active and the next start marks it uncertain.
    fn reconcile_failed_attempt(&self, job_id: &str) -> &'static str {
        let result = {
            let mut store = self.store();
            if !store.is_consistent() {
                return "storage is unusable, so a restart will mark the job uncertain";
            }
            store.mark_active_uncertain(
                job_id,
                "NuPrint's print queue failed while this job was printing. Check the paper; it was not resent.",
            )
        };
        match result {
            Ok(Some(job)) => {
                self.emit_job(&job);
                "the job was marked uncertain"
            }
            Ok(None) => "the job had not started or had finished",
            Err(_) => "the job could not be saved, so a restart will mark it uncertain",
        }
    }
}
