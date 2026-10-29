//! Failure of the dispatcher task itself: the service must stop, keep the claimed job
//! conservative and never touch Bluetooth again. Uses the `fault-injection` test seam plus
//! panics inside the mock transport. Offline: no Bluetooth, no paper.

use super::*;
use nuprint_core::service::{FaultPoint, ShutdownOutcome};

async fn next_fault(h: &mut Harness) -> ServiceFault {
    let deadline = tokio::time::Instant::now() + Duration::from_mins(1);
    loop {
        let event = tokio::time::timeout_at(deadline, h.events.recv()).await.expect("a fault was reported");
        if let Some(AppEvent::QueueFault { fault, .. }) = event {
            return fault;
        }
    }
}

/// Start a job that makes the dispatcher panic and wait for the failure to be reported.
async fn fail_one_job(sim: Sim, point: Option<FaultPoint>) -> (Harness, SavedPrinter, JobSummary, ServiceFault) {
    let mut h = start(vec![sc03h("SC03h-TEST", sim)], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("A note long enough to need several raster rows\nand a second line").await;
    if let Some(point) = point {
        h.service.inject_panic(point);
    }
    let job = h.enqueue(&preview, &printer, "k", 12);
    let fault = next_fault(&mut h).await;
    (h, printer, job, fault)
}

/// Everything that must stay refused after a failure; nothing reaches Bluetooth again.
async fn assert_closed(h: &Harness, printer: &SavedPrinter, job: &JobSummary, state: JobState) {
    let connects = h.transport.log().connects;
    let writes = h.transport.log().writes.len();
    let preview = h.preview("Another note").await;
    let refused = h
        .service
        .enqueue(EnqueueRequest {
            snapshot_id: preview.snapshot_id.clone(),
            printer_id: printer.id.clone(),
            copies: 1,
            density: D::Normal,
            feed_mm: 12,
            idempotency_key: "second".into(),
            title: "Second".into(),
        })
        .expect_err("a faulted service accepts no new jobs");
    assert!(refused.message.contains("restart NuPrint"), "{}", refused.message);
    let resume = h.service.resume_queue().expect_err("resume is refused while faulted");
    assert!(resume.message.contains("restart NuPrint"), "{}", resume.message);
    assert!(h.service.check_printer(&printer.id).await.is_err(), "no second Bluetooth owner");
    tokio::time::sleep(Duration::from_secs(30)).await;

    let snapshot = h.service.list_jobs().unwrap();
    assert!(snapshot.held && snapshot.fault.is_some());
    assert_eq!(snapshot.jobs.iter().find(|j| j.id == job.id).unwrap().state, state, "never requeued or replayed");
    assert_eq!(h.transport.log().connects, connects, "no reconnect");
    assert_eq!(h.transport.log().writes.len(), writes, "no replay and no new writes");
}

#[tokio::test(start_paused = true)]
async fn failure_before_the_claim_keeps_the_job_queued_and_stops_the_queue() {
    let (h, printer, job, fault) = fail_one_job(Sim::default(), Some(FaultPoint::BeforeClaim)).await;
    assert_eq!(fault.kind, ServiceFaultKind::DispatcherPanicked);
    assert_eq!(fault.job_id.as_deref(), Some(job.id.as_str()));
    assert_eq!(h.transport.log().connects, 0);
    assert_closed(&h, &printer, &job, JobState::Queued).await;
}

#[tokio::test(start_paused = true)]
async fn failure_after_the_claim_before_connecting_is_uncertain() {
    let (h, printer, job, _) = fail_one_job(Sim::default(), Some(FaultPoint::AfterClaim)).await;
    let stored = h.service.get_job(&job.id).unwrap();
    assert_eq!(stored.state, JobState::Uncertain);
    assert!(stored.partial_output_possible);
    assert_eq!(stored.error.unwrap().code, ErrorCode::OutcomeUncertain);
    assert_eq!(h.transport.log().connects, 0);
    assert_closed(&h, &printer, &job, JobState::Uncertain).await;
}

#[tokio::test(start_paused = true)]
async fn failure_while_setting_up_the_connection_is_uncertain() {
    let (h, printer, job, _) = fail_one_job(Sim { panic_on_status: true, ..Sim::default() }, None).await;
    assert_eq!(h.service.get_job(&job.id).unwrap().state, JobState::Uncertain);
    assert_eq!(h.transport.log().connects, 1);
    assert_closed(&h, &printer, &job, JobState::Uncertain).await;
}

#[tokio::test(start_paused = true)]
async fn failure_in_the_first_print_write_is_uncertain_and_not_resent() {
    let (h, printer, job, _) = fail_one_job(Sim { panic_on_row: Some(1), ..Sim::default() }, None).await;
    assert_eq!(h.service.get_job(&job.id).unwrap().state, JobState::Uncertain);
    assert!(!h.print_bytes_written().is_empty(), "the byte may have arrived");
    assert_closed(&h, &printer, &job, JobState::Uncertain).await;
}

#[tokio::test(start_paused = true)]
async fn failure_after_bytes_arrived_is_uncertain_and_not_resent() {
    let (h, printer, job, _) = fail_one_job(Sim { panic_on_row: Some(5), ..Sim::default() }, None).await;
    let stored = h.service.get_job(&job.id).unwrap();
    assert_eq!(stored.state, JobState::Uncertain);
    assert!(stored.state_text.contains("heck"), "{}", stored.state_text);
    assert_closed(&h, &printer, &job, JobState::Uncertain).await;
}

#[tokio::test(start_paused = true)]
async fn failure_while_saving_the_outcome_poisons_the_store_but_it_is_validated_and_reused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let mut h = start(vec![sc03h("SC03h-TEST", Sim::default())], Some(dir));
    let printer = h.save_first_printer().await;
    let preview = h.preview("Printed, then the queue fails").await;
    h.service.inject_panic(FaultPoint::WhilePersisting);
    let job = h.enqueue(&preview, &printer, "k", 12);
    let fault = next_fault(&mut h).await;
    // The transfer finished, but its outcome was never saved: only "uncertain" is honest.
    assert_eq!(fault.job_id.as_deref(), Some(job.id.as_str()));
    assert_eq!(h.service.get_job(&job.id).unwrap().state, JobState::Uncertain, "the store lock was recovered");
    assert_closed(&h, &printer, &job, JobState::Uncertain).await;
    // The failed service still owns the data folder.
    let second = AppService::start(
        ServiceConfig { data_dir: Some(path), pdfium_dirs: vec![], version: "test".into() },
        Arc::new(MockTransport::new(vec![])),
        Arc::new(|_e: AppEvent| {}),
    );
    assert_eq!(second.err().expect("still owned").code, ErrorCode::DataDirInUse);
}

#[tokio::test(start_paused = true)]
async fn the_next_start_keeps_the_failed_jobs_conservative_and_held() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let mut h = start(vec![sc03h("SC03h-TEST", Sim::default())], Some(dir));
    let printer = h.save_first_printer().await;
    let preview = h.preview("First").await;
    h.service.inject_panic(FaultPoint::AfterClaim);
    let failed = h.enqueue(&preview, &printer, "a", 12);
    next_fault(&mut h).await;
    assert_eq!(h.service.shutdown(Duration::from_secs(1)).await, ShutdownOutcome::Faulted);
    let dir = h._dir;
    drop(h.service);

    let transport = MockTransport::new(vec![sc03h("SC03h-TEST", Sim::default())]);
    let service = AppService::start(
        ServiceConfig { data_dir: Some(path), pdfium_dirs: vec![], version: "test".into() },
        Arc::new(transport.clone()),
        Arc::new(|_e: AppEvent| {}),
    )
    .unwrap();
    let snapshot = service.list_jobs().unwrap();
    assert!(snapshot.fault.is_none(), "a restart clears the fault");
    assert_eq!(snapshot.jobs.iter().find(|j| j.id == failed.id).unwrap().state, JobState::Uncertain);
    assert_eq!(transport.log().connects, 0, "nothing is replayed");
    drop(dir);
}

#[tokio::test(start_paused = true)]
async fn shutdown_racing_a_failure_waits_for_it_and_reports_it() {
    let mut h = start(vec![sc03h("SC03h-TEST", Sim::default())], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Shutdown while failing").await;
    h.service.inject_panic(FaultPoint::WhilePersisting);
    let job = h.enqueue(&preview, &printer, "k", 12);
    // Wait for the claim, then shut down: the cancelled job still reaches its failing save.
    loop {
        if let Some(AppEvent::JobChanged { job: j, .. }) = h.events.recv().await
            && j.id == job.id
            && j.state == JobState::Connecting
        {
            break;
        }
    }
    assert_eq!(h.service.shutdown(Duration::from_secs(10)).await, ShutdownOutcome::Faulted);
    assert_eq!(h.service.get_job(&job.id).unwrap().state, JobState::Uncertain);
    assert!(h.service.list_jobs().unwrap().fault.is_some());
}

#[tokio::test(start_paused = true)]
async fn a_normal_shutdown_is_clean() {
    let h = start(vec![sc03h("SC03h-TEST", Sim::default())], None);
    assert_eq!(h.service.shutdown(Duration::from_secs(1)).await, ShutdownOutcome::Clean);
    assert!(h.service.list_jobs().unwrap().fault.is_none());
}
