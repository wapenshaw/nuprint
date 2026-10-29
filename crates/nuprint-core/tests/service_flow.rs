//! End-to-end job flows against a simulated SC03h-family printer on the mock transport.
//! Offline: no Bluetooth, no paper. Time is paused and auto-advanced by Tokio.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use nuprint_core::api::*;
use nuprint_core::{AppService, ErrorCode, ServiceConfig};
use nuprint_protocol::iprint::{self, DIRECTION_RESPONSE, FrameDecoder, Iprint5178Driver, Iprint5178Params, opcode};
use nuprint_protocol::{Density as D, PrintRequest};
use nuprint_transport::mock::{MockDevice, MockTransport, WriteHook};
use nuprint_transport::{
    Advertisement, CharProps, CharacteristicInfo, Endpoint, Notification, ServiceInfo, TransportError, short_uuid,
};
use tokio::sync::mpsc;

const PROFILE: &str = "sc03h-iprint-384";

/// Behaviour of the simulated printer.
#[derive(Clone, Default)]
struct Sim {
    /// Status byte returned for successive A3 queries; the last value repeats.
    statuses: Vec<u8>,
    /// Never answer status queries.
    mute: bool,
    /// Send a flow-pause after this many raster rows (resume must be injected).
    pause_after_rows: Option<usize>,
    /// Drop the connection after this many raster rows.
    disconnect_after_rows: Option<usize>,
}

fn notify(op: u8, payload: &[u8]) -> Notification {
    Notification {
        endpoint: Endpoint { service: short_uuid(0xAE30), characteristic: short_uuid(0xAE02) },
        data: iprint::frame_with_direction(op, DIRECTION_RESPONSE, payload).unwrap(),
    }
}

fn sc03h(name: &str, sim: Sim) -> MockDevice {
    let wwr = CharProps { write_without_response: true, ..Default::default() };
    let n = CharProps { notify: true, ..Default::default() };
    MockDevice {
        advertisement: Advertisement {
            backend_id: format!("id-{name}"),
            local_name: Some(name.into()),
            address: Some("AA:BB:CC:DD:EE:FF".into()),
            rssi: Some(-50),
            services: vec![short_uuid(0xAF30)],
            manufacturer_ids: vec![],
        },
        services: vec![ServiceInfo {
            uuid: short_uuid(0xAE30),
            characteristics: vec![
                CharacteristicInfo { uuid: short_uuid(0xAE01), properties: wwr },
                CharacteristicInfo { uuid: short_uuid(0xAE02), properties: n },
                CharacteristicInfo { uuid: short_uuid(0xAE04), properties: n },
            ],
        }],
        max_write: 180,
        // The status script continues across connections (one physical printer).
        hook: Arc::new({
            let queries = Arc::new(AtomicUsize::new(0));
            move || {
                let sim = sim.clone();
                let queries = queries.clone();
                let mut decoder = FrameDecoder::new();
                let mut rows = 0usize;
                let hook: WriteHook = Box::new(move |_endpoint, data| {
                    let mut out = Vec::new();
                    for frame in decoder.feed(data) {
                        match frame.opcode {
                            opcode::STATUS if !sim.mute => {
                                let n = queries.fetch_add(1, Ordering::Relaxed);
                                let status = sim.statuses.get(n).or(sim.statuses.last()).copied().unwrap_or(0);
                                out.push(notify(opcode::STATUS, &[status, 0x12, 0x27]));
                            }
                            opcode::DEVICE_INFO => {
                                out.push(notify(opcode::DEVICE_INFO, &hex::decode("810003332e302e343900").unwrap()));
                            }
                            opcode::RASTER_ROW => {
                                rows += 1;
                                if Some(rows) == sim.disconnect_after_rows {
                                    return Err(TransportError::Disconnected);
                                }
                                if Some(rows) == sim.pause_after_rows {
                                    out.push(notify(opcode::FLOW, &[0x10]));
                                }
                            }
                            _ => {}
                        }
                    }
                    Ok(out)
                });
                hook
            }
        }),
    }
}

struct Harness {
    service: AppService,
    transport: MockTransport,
    events: mpsc::UnboundedReceiver<AppEvent>,
    _dir: Option<tempfile::TempDir>,
}

fn start(devices: Vec<MockDevice>, data_dir: Option<tempfile::TempDir>) -> Harness {
    start_on(MockTransport::new(devices), data_dir)
}

fn start_on(transport: MockTransport, data_dir: Option<tempfile::TempDir>) -> Harness {
    let (tx, events) = mpsc::unbounded_channel();
    let service = AppService::start(
        ServiceConfig {
            data_dir: data_dir.as_ref().map(|d| d.path().to_path_buf()),
            pdfium_dirs: vec![],
            version: "test".into(),
        },
        Arc::new(transport.clone()),
        Arc::new(move |e: AppEvent| {
            let _ = tx.send(e);
        }),
    )
    .unwrap();
    Harness { service, transport, events, _dir: data_dir }
}

impl Harness {
    async fn save_first_printer(&self) -> SavedPrinter {
        let found = self.service.find_first_supported(Duration::from_secs(5)).await.unwrap();
        self.service
            .save_printer(SavePrinterRequest {
                discovery_id: found.discovery_id,
                profile_id: PROFILE.into(),
                acknowledge_experimental: true,
            })
            .unwrap()
    }

    async fn preview(&self, height_hint: &str) -> PreviewResult {
        let text = TextSpec { text: height_hint.into(), ..TextSpec::default() };
        self.service
            .render_preview(PreviewRequest {
                generation: 1.0,
                profile_id: PROFILE.into(),
                source: SourceSpec::Text(text),
            })
            .await
            .unwrap()
    }

    fn enqueue(&self, preview: &PreviewResult, printer: &SavedPrinter, key: &str, feed_mm: u16) -> JobSummary {
        self.service
            .enqueue(EnqueueRequest {
                snapshot_id: preview.snapshot_id.clone(),
                printer_id: printer.id.clone(),
                copies: 1,
                density: D::Normal,
                feed_mm,
                idempotency_key: key.into(),
                title: "Note".into(),
            })
            .unwrap()
    }

    async fn wait_terminal(&mut self, job_id: &str) -> JobSummary {
        let deadline = tokio::time::Instant::now() + Duration::from_mins(10);
        loop {
            let event = tokio::time::timeout_at(deadline, self.events.recv()).await.expect("job finished").unwrap();
            if let AppEvent::JobChanged { job, .. } = event
                && job.id == job_id
                && job.state.is_terminal()
            {
                // Let follow-up persistence (content deletion) settle.
                tokio::time::sleep(Duration::from_millis(10)).await;
                return self.service.get_job(job_id).unwrap();
            }
        }
    }

    /// Bytes written excluding standalone status/device-info query frames.
    fn print_bytes_written(&self) -> Vec<u8> {
        let all = self.transport.log().written_bytes();
        let status = iprint::command(opcode::STATUS, &[0]);
        let info = iprint::command(opcode::DEVICE_INFO, &[0]);
        let mut out = Vec::new();
        let mut i = 0;
        while i < all.len() {
            if all[i..].starts_with(&status) {
                i += status.len();
            } else if all[i..].starts_with(&info) {
                i += info.len();
            } else {
                out.push(all[i]);
                i += 1;
            }
        }
        out
    }
}

fn expected_bytes(preview_png: &[u8], feed_mm: u16) -> Vec<u8> {
    let bitmap = nuprint_render::decode_png_mono(preview_png).unwrap();
    let driver = Iprint5178Driver::new(Iprint5178Params::sc03h_reference()).unwrap();
    driver.encode_job(&PrintRequest { bitmap: &bitmap, density: D::Normal, trailing_feed_mm: feed_mm }).unwrap()
}

#[tokio::test(start_paused = true)]
async fn prints_exact_preview_bytes_and_confirms_ready() {
    let mut h = start(vec![sc03h("SC03h-TEST", Sim::default())], None);
    let printer = h.save_first_printer().await;
    assert_eq!(printer.settings.feed_mm, 12);
    let preview = h.preview("Hello from NuPrint").await;
    let png = h.service.preview_png(&preview.snapshot_id).unwrap();
    let job = h.enqueue(&preview, &printer, "k1", 3);
    let done = h.wait_terminal(&job.id).await;
    assert_eq!(done.state, JobState::Completed, "{:?}", done.error);
    assert_eq!(done.state_text, "Transfer complete · printer reports ready");
    assert!(!done.partial_output_possible);
    assert!(!done.content_retained, "completed content is deleted by default");
    assert_eq!(done.bytes_sent, done.bytes_total);
    // The printed raster is exactly the displayed preview, encoded like the Python reference.
    assert_eq!(h.print_bytes_written(), expected_bytes(&png, 3));
    let log = h.transport.log();
    assert_eq!((log.connects, log.disconnects), (1, 1));
    assert_eq!(log.subscriptions.len(), 2);
    assert!(log.writes.iter().all(|(_, chunk, _)| chunk.len() <= 180));
}

#[tokio::test(start_paused = true)]
async fn experimental_model_needs_opt_in_then_prints_like_sc03h() {
    const X6H: &str = "x6h-iprint-384";
    let mut h = start(vec![sc03h("X6h-TEST", Sim::default())], None);
    let found = h.service.find_first_supported(Duration::from_secs(5)).await.unwrap();
    assert_eq!(found.candidates.len(), 1);
    assert_eq!(found.candidates[0].profile_id, X6H);
    assert_eq!(found.candidates[0].support, SupportLevel::Experimental);
    let request = |acknowledge_experimental| SavePrinterRequest {
        discovery_id: found.discovery_id.clone(),
        profile_id: X6H.into(),
        acknowledge_experimental,
    };
    let refused = h.service.save_printer(request(false)).unwrap_err();
    assert_eq!(refused.code, ErrorCode::ExperimentalNotAcknowledged);
    let printer = h.service.save_printer(request(true)).unwrap();

    let text = TextSpec { text: "Hello X6h".into(), ..TextSpec::default() };
    let preview = h
        .service
        .render_preview(PreviewRequest { generation: 1.0, profile_id: X6H.into(), source: SourceSpec::Text(text) })
        .await
        .unwrap();
    let png = h.service.preview_png(&preview.snapshot_id).unwrap();
    let job = h.enqueue(&preview, &printer, "k1", 3);
    let done = h.wait_terminal(&job.id).await;
    assert_eq!(done.state, JobState::Completed, "{:?}", done.error);
    assert_eq!(h.print_bytes_written(), expected_bytes(&png, 3));
}

/// The settings document of a saved printer, edited through `f`.
fn edited(h: &Harness, printer: &SavedPrinter, f: impl FnOnce(&mut serde_json::Value)) -> SavePrinterProfileRequest {
    let doc = h.service.printer_profile_doc(&printer.id).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&doc.current_json).unwrap();
    f(&mut json);
    SavePrinterProfileRequest { printer_id: printer.id.clone(), json: json.to_string() }
}

#[tokio::test(start_paused = true)]
async fn edited_settings_print_with_the_new_values_survive_restart_and_reset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let mut h = start(vec![sc03h("SC03h-TEST", Sim::default())], Some(dir));
    let printer = h.save_first_printer().await;
    assert!(!printer.customized);

    let request = edited(&h, &printer, |doc| {
        doc["driver"]["energy"]["normal"] = 4000.into();
        doc["transport"]["max_chunk_bytes"] = 100.into();
    });
    let custom = h.service.save_printer_profile(request).unwrap();
    assert!(custom.customized);
    assert!(custom.profile_id.starts_with("custom:"), "{}", custom.profile_id);
    assert_eq!(custom.profile_revision, 1);
    assert_eq!(custom.support, SupportLevel::Experimental, "edited settings are never verified");
    assert!(h.service.profiles().iter().any(|p| p.id == custom.profile_id));

    // The preview, the job and the wire bytes all use the edited profile.
    let text = TextSpec { text: "Tuned".into(), ..TextSpec::default() };
    let preview = h
        .service
        .render_preview(PreviewRequest {
            generation: 1.0,
            profile_id: custom.profile_id.clone(),
            source: SourceSpec::Text(text),
        })
        .await
        .unwrap();
    let png = h.service.preview_png(&preview.snapshot_id).unwrap();
    let job = h.enqueue(&preview, &custom, "k1", 3);
    let done = h.wait_terminal(&job.id).await;
    assert_eq!(done.state, JobState::Completed, "{:?}", done.error);
    let mut params = Iprint5178Params::sc03h_reference();
    params.energy.insert(D::Normal, 4000);
    let bitmap = nuprint_render::decode_png_mono(&png).unwrap();
    let expected = Iprint5178Driver::new(params)
        .unwrap()
        .encode_job(&PrintRequest { bitmap: &bitmap, density: D::Normal, trailing_feed_mm: 3 })
        .unwrap();
    assert_eq!(h.print_bytes_written(), expected);
    assert!(h.transport.log().writes.iter().all(|(_, chunk, _)| chunk.len() <= 100));

    // Saving again makes a new revision; a preview rendered for revision 1 is then stale.
    let again =
        h.service.save_printer_profile(edited(&h, &custom, |doc| doc["driver"]["print_speed"] = 12.into())).unwrap();
    assert_eq!((again.profile_id.as_str(), again.profile_revision), (custom.profile_id.as_str(), 2));
    let stale = h.service.enqueue(EnqueueRequest {
        snapshot_id: preview.snapshot_id.clone(),
        printer_id: again.id.clone(),
        copies: 1,
        density: D::Normal,
        feed_mm: 3,
        idempotency_key: "k2".into(),
        title: "Stale".into(),
    });
    assert_eq!(stale.unwrap_err().code, ErrorCode::StalePreview);

    // The edit is stored with the printer: a restart still resolves it.
    h.service.shutdown(Duration::from_secs(1)).await;
    let dir = h._dir;
    drop(h.service);
    let service = AppService::start(
        ServiceConfig { data_dir: Some(path), pdfium_dirs: vec![], version: "test".into() },
        Arc::new(MockTransport::new(vec![sc03h("SC03h-TEST", Sim::default())])),
        Arc::new(|_e: AppEvent| {}),
    )
    .unwrap();
    let restored = service.list_printers().unwrap().remove(0);
    assert!(restored.customized && restored.profile_available);
    assert_eq!((restored.profile_id.as_str(), restored.profile_revision), (custom.profile_id.as_str(), 2));
    let doc = service.printer_profile_doc(&restored.id).unwrap();
    assert!(doc.customized);
    assert!(doc.current_json.contains("\"print_speed\": 12"));
    assert_ne!(doc.current_json, doc.base_json);

    // Reset returns to the built-in profile and its exact settings.
    let reset = service.reset_printer_profile(&restored.id).unwrap();
    assert!(!reset.customized);
    assert_eq!((reset.profile_id.as_str(), reset.profile_revision), (PROFILE, 1));
    let doc = service.printer_profile_doc(&reset.id).unwrap();
    assert!(!doc.customized);
    assert_eq!(doc.current_json, doc.base_json);
    drop(dir);
}

#[tokio::test(start_paused = true)]
async fn edited_settings_cannot_change_identity_or_use_unsafe_values() {
    let h = start(vec![sc03h("SC03h-TEST", Sim::default())], None);
    let printer = h.save_first_printer().await;
    let rejected = |request: SavePrinterProfileRequest| h.service.save_printer_profile(request).unwrap_err();

    let locked = rejected(edited(&h, &printer, |doc| doc["match"] = serde_json::json!({ "name_prefixes": ["X"] })));
    assert_eq!(locked.code, ErrorCode::InvalidInput);
    assert!(locked.message.contains("\"match\" cannot be changed"), "{}", locked.message);
    for (key, value) in [("id", "other"), ("protocol", "other"), ("verification", "[]")] {
        let request = edited(&h, &printer, |doc| doc[key] = value.into());
        assert_eq!(rejected(request).code, ErrorCode::InvalidInput, "{key}");
    }
    // Values that cannot work (or can only waste paper or overheat the head) are refused by name.
    type Edit = fn(&mut serde_json::Value);
    let certain_breakers: [(&str, Edit); 18] = [
        ("heat for light", |d| d["driver"]["energy"]["light"] = 0.into()),
        ("heat for normal", |d| d["driver"]["energy"]["normal"] = 20_000.into()),
        ("heat for dark", |d| d["driver"]["energy"]["dark"] = 100.into()),
        ("print_speed", |d| d["driver"]["print_speed"] = 0.into()),
        ("feed_steps_per_mm", |d| d["driver"]["feed_steps_per_mm"] = 0.into()),
        ("feed.max_mm", |d| d["driver"]["feed"]["max_mm"] = 200.into()),
        ("ready_poll_ms", |d| d["driver"]["timing"]["ready_poll_ms"] = 0.into()),
        ("status_timeout_ms", |d| d["driver"]["timing"]["status_timeout_ms"] = 0.into()),
        ("ready_deadline_ms", |d| d["driver"]["timing"]["ready_deadline_ms"] = 10.into()),
        ("max_chunk_bytes", |d| d["transport"]["max_chunk_bytes"] = 5.into()),
        ("min_notify_subscriptions", |d| d["transport"]["min_notify_subscriptions"] = 0.into()),
        ("flow_pause_ms", |d| d["transport"]["timeouts"]["flow_pause_ms"] = 0.into()),
        ("connect_ms", |d| d["transport"]["timeouts"]["connect_ms"] = 10.into()),
        ("max_copies", |d| d["limits"]["max_copies"] = 5000.into()),
        ("dots_per_mm", |d| d["raster"]["dots_per_mm"] = 0.1.into()),
        ("max_height", |d| {
            d["raster"]["max_height"] = 1_000_000.into();
            d["driver"]["max_height"] = 1_000_000.into();
        }),
        ("wire_packing", |d| d["raster"]["wire_packing"] = "msb_first".into()),
        ("display_name", |d| d["display_name"] = "".into()),
    ];
    for (what, edit) in certain_breakers {
        let refused = rejected(edited(&h, &printer, edit));
        assert_eq!(refused.code, ErrorCode::InvalidInput, "{what}");
        assert!(refused.message.contains(what), "{what}: {}", refused.message);
    }
    // A value near the built-in one is fine.
    let ok = h.service.save_printer_profile(edited(&h, &printer, |d| d["driver"]["energy"]["normal"] = 6000.into()));
    assert!(ok.unwrap().customized);
    h.service.reset_printer_profile(&printer.id).unwrap();

    let too_big = rejected(edited(&h, &printer, |doc| doc["transport"]["max_chunk_bytes"] = 9999.into()));
    assert!(too_big.message.contains("max_chunk_bytes"), "{}", too_big.message);
    let too_slow = rejected(edited(&h, &printer, |doc| doc["transport"]["pacing"]["chunk_delay_ms"] = 999_999.into()));
    assert!(too_slow.message.contains("chunk_delay_ms"), "{}", too_slow.message);
    let wrong_width = rejected(edited(&h, &printer, |doc| doc["raster"]["width_dots"] = 100.into()));
    assert_eq!(wrong_width.code, ErrorCode::InvalidInput);
    let broken = h
        .service
        .save_printer_profile(SavePrinterProfileRequest { printer_id: printer.id.clone(), json: "{".into() })
        .unwrap_err();
    assert_eq!(broken.code, ErrorCode::InvalidInput);

    // Nothing was stored, and settings identical to the built-in ones are not an edit.
    assert!(!h.service.list_printers().unwrap()[0].customized);
    let unchanged = h.service.save_printer_profile(edited(&h, &printer, |_| {})).unwrap();
    assert!(!unchanged.customized);
    assert_eq!(unchanged.profile_id, PROFILE);
}

#[tokio::test(start_paused = true)]
async fn exported_profile_is_complete_portable_and_reloads() {
    let h = start(vec![sc03h("SC03h-TEST", Sim::default())], None);
    let printer = h.save_first_printer().await;
    let builtin = h.service.export_printer_profile(&printer.id).unwrap();
    assert!(builtin.contains(&format!("\"id\": \"{PROFILE}\"")));

    let custom = h
        .service
        .save_printer_profile(edited(&h, &printer, |doc| doc["driver"]["energy"]["dark"] = 6000.into()))
        .unwrap();
    let exported = h.service.export_printer_profile(&custom.id).unwrap();
    assert!(!exported.contains("custom:"), "no local printer ID in a shared file");
    let profile = nuprint_core::profile::ModelProfile::parse(&exported).unwrap();
    assert_eq!(profile.id, format!("{PROFILE}-custom"));
    assert_eq!(profile.derived_from.as_deref(), Some(PROFILE));
    assert!(profile.verification.is_empty());
    assert!(exported.contains("6000"));
}

#[tokio::test(start_paused = true)]
async fn duplicate_submission_creates_one_job() {
    let h = start(vec![sc03h("SC03h-TEST", Sim::default())], None);
    h.service.hold_queue().unwrap();
    let printer = h.save_first_printer().await;
    let preview = h.preview("Once").await;
    let a = h.enqueue(&preview, &printer, "same", 12);
    let b = h.enqueue(&preview, &printer, "same", 12);
    assert_eq!(a.id, b.id);
    assert_eq!(h.service.list_jobs().unwrap().jobs.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn paper_out_blocks_before_any_print_bytes() {
    let mut h = start(vec![sc03h("SC03h-TEST", Sim { statuses: vec![0x01], ..Sim::default() })], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Paper").await;
    let job = h.enqueue(&preview, &printer, "k", 12);
    let done = h.wait_terminal(&job.id).await;
    assert_eq!(done.state, JobState::Failed);
    assert_eq!(done.error.unwrap().code, ErrorCode::PaperOut);
    assert!(h.print_bytes_written().is_empty());
    assert!(done.content_retained, "failed content is kept for retry");
    assert_eq!(h.transport.log().disconnects, 1);
}

#[tokio::test(start_paused = true)]
async fn already_printing_is_refused() {
    let mut h = start(vec![sc03h("SC03h-TEST", Sim { statuses: vec![0x80], ..Sim::default() })], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Busy").await;
    let done = h.wait_terminal(&h.enqueue(&preview, &printer, "k", 12).id).await;
    assert_eq!(done.state, JobState::Failed);
    assert_eq!(done.error.unwrap().code, ErrorCode::PrinterBusy);
}

#[tokio::test(start_paused = true)]
async fn disconnect_mid_transfer_is_uncertain_and_never_replayed() {
    let sim = Sim { disconnect_after_rows: Some(5), ..Sim::default() };
    let mut h = start(vec![sc03h("SC03h-TEST", sim)], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("A longer note\nwith several lines\nso the job has many rows").await;
    let done = h.wait_terminal(&h.enqueue(&preview, &printer, "k", 12).id).await;
    assert_eq!(done.state, JobState::Uncertain);
    assert!(done.partial_output_possible);
    assert!(done.state_text.contains("check before retrying"));
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(h.transport.log().connects, 1, "no automatic reconnect or replay");
}

#[tokio::test(start_paused = true)]
async fn a_failed_first_print_write_is_uncertain_not_unsent() {
    // The very first raster write errors. A failed write does not prove nothing arrived.
    let sim = Sim { disconnect_after_rows: Some(1), ..Sim::default() };
    let mut h = start(vec![sc03h("SC03h-TEST", sim)], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("First write fails").await;
    let done = h.wait_terminal(&h.enqueue(&preview, &printer, "k", 12).id).await;
    assert_eq!(done.state, JobState::Uncertain);
    assert!(done.partial_output_possible);
    assert_eq!(done.bytes_sent, 0.0, "only confirmed writes are counted as sent");
}

#[tokio::test(start_paused = true)]
async fn mid_job_fault_stops_transfer() {
    // Fresh ready status, then overheated on the poll after transfer.
    let sim = Sim { statuses: vec![0x00, 0x04], ..Sim::default() };
    let mut h = start(vec![sc03h("SC03h-TEST", sim)], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Heat").await;
    let done = h.wait_terminal(&h.enqueue(&preview, &printer, "k", 12).id).await;
    assert_eq!(done.state, JobState::Uncertain);
    assert_eq!(done.error.unwrap().code, ErrorCode::Overheated);
}

#[tokio::test(start_paused = true)]
async fn flow_pause_waits_for_resume() {
    let sim = Sim { pause_after_rows: Some(3), ..Sim::default() };
    let mut h = start(vec![sc03h("SC03h-TEST", sim)], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Pause and resume").await;
    let job = h.enqueue(&preview, &printer, "k", 12);
    let transport = h.transport.clone();
    let resumer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let paused_at = transport.log().writes.len();
        transport.inject(notify(opcode::FLOW, &[0x00])).await;
        paused_at
    });
    let done = h.wait_terminal(&job.id).await;
    let writes_while_paused = resumer.await.unwrap();
    assert_eq!(done.state, JobState::Completed, "{:?}", done.error);
    assert!(h.transport.log().writes.len() > writes_while_paused, "sending resumed after the resume frame");
}

#[tokio::test(start_paused = true)]
async fn buffer_that_never_resumes_times_out_as_uncertain() {
    let sim = Sim { pause_after_rows: Some(1), ..Sim::default() };
    let mut h = start(vec![sc03h("SC03h-TEST", sim)], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Stuck").await;
    let done = h.wait_terminal(&h.enqueue(&preview, &printer, "k", 12).id).await;
    assert_eq!(done.state, JobState::Uncertain);
    assert_eq!(done.error.unwrap().code, ErrorCode::BufferTimeout);
}

#[tokio::test(start_paused = true)]
async fn silent_printer_fails_with_status_timeout() {
    let mut h = start(vec![sc03h("SC03h-TEST", Sim { mute: true, ..Sim::default() })], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Hello?").await;
    let done = h.wait_terminal(&h.enqueue(&preview, &printer, "k", 12).id).await;
    assert_eq!(done.state, JobState::Failed);
    assert_eq!(done.error.unwrap().code, ErrorCode::StatusTimeout);
    assert!(h.print_bytes_written().is_empty());
}

#[tokio::test(start_paused = true)]
async fn cancel_during_transfer_stops_writes_and_disconnects() {
    let sim = Sim { pause_after_rows: Some(2), ..Sim::default() };
    let mut h = start(vec![sc03h("SC03h-TEST", sim)], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Cancel me").await;
    let job = h.enqueue(&preview, &printer, "k", 12);
    // Wait until the job is sending (blocked on the pause), then cancel.
    loop {
        if let Some(AppEvent::JobChanged { job: j, .. }) = h.events.recv().await
            && j.id == job.id
            && j.state == JobState::Sending
        {
            break;
        }
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.service.cancel_job(&job.id).unwrap();
    let done = h.wait_terminal(&job.id).await;
    let writes = h.transport.log().writes.len();
    assert_eq!(done.state, JobState::Cancelled);
    assert!(done.partial_output_possible);
    assert_eq!(h.transport.log().disconnects, 1);
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(h.transport.log().writes.len(), writes, "no writes after cancellation");
}

#[tokio::test(start_paused = true)]
async fn zero_feed_omits_feed_but_still_waits_for_ready() {
    let mut h = start(vec![sc03h("SC03h-TEST", Sim::default())], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("No feed").await;
    let png = h.service.preview_png(&preview.snapshot_id).unwrap();
    let done = h.wait_terminal(&h.enqueue(&preview, &printer, "k", 0).id).await;
    assert_eq!(done.state, JobState::Completed);
    let written = h.print_bytes_written();
    assert_eq!(written, expected_bytes(&png, 0));
    assert!(!written.array_windows().any(|w| *w == [0x51, 0x78, opcode::FEED]));
    // Status queried before printing and polled afterwards.
    let status = iprint::command(opcode::STATUS, &[0]);
    let all = h.transport.log().written_bytes();
    assert!(all.windows(status.len()).filter(|w| *w == status.as_slice()).count() >= 2);
}

#[tokio::test(start_paused = true)]
async fn invalid_settings_are_rejected_before_admission() {
    let h = start(vec![sc03h("SC03h-TEST", Sim::default())], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Bounds").await;
    let request = |copies, feed_mm| EnqueueRequest {
        snapshot_id: preview.snapshot_id.clone(),
        printer_id: printer.id.clone(),
        copies,
        density: D::Normal,
        feed_mm,
        idempotency_key: format!("{copies}-{feed_mm}"),
        title: "x".into(),
    };
    assert_eq!(h.service.enqueue(request(1, 13)).unwrap_err().code, ErrorCode::InvalidInput);
    assert_eq!(h.service.enqueue(request(0, 12)).unwrap_err().code, ErrorCode::InvalidInput);
    assert_eq!(h.service.enqueue(request(21, 12)).unwrap_err().code, ErrorCode::InvalidInput);
    assert!(h.service.list_jobs().unwrap().jobs.is_empty());
    assert_eq!(h.transport.log().connects, 0);
}

#[tokio::test(start_paused = true)]
async fn uppercase_model_is_not_offered() {
    let h = start(vec![sc03h("SC03H-1234", Sim::default())], None);
    let err = h.service.find_first_supported(Duration::from_secs(2)).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::DeviceNotFound);
    let found = h.service.find_device("SC03H-1234", Duration::from_secs(2)).await.unwrap();
    assert!(found.candidates.is_empty());
    let save = h.service.save_printer(SavePrinterRequest {
        discovery_id: found.discovery_id,
        profile_id: PROFILE.into(),
        acknowledge_experimental: true,
    });
    assert_eq!(save.unwrap_err().code, ErrorCode::UnsupportedProfile);
}

#[tokio::test(start_paused = true)]
async fn copies_apply_feed_once_after_the_last_copy() {
    let mut h = start(vec![sc03h("SC03h-TEST", Sim::default())], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Twice").await;
    let job = h
        .service
        .enqueue(EnqueueRequest {
            snapshot_id: preview.snapshot_id.clone(),
            printer_id: printer.id.clone(),
            copies: 2,
            density: D::Normal,
            feed_mm: 12,
            idempotency_key: "copies".into(),
            title: "x".into(),
        })
        .unwrap();
    assert!((job.paper_mm - (preview.length_mm * 2.0 + 12.0)).abs() < 0.01);
    let done = h.wait_terminal(&job.id).await;
    assert_eq!(done.state, JobState::Completed);
    let written = h.print_bytes_written();
    let feeds = written.array_windows().filter(|w| **w == [0x51, 0x78, opcode::FEED]).count();
    assert_eq!(feeds, 2, "two 48-step A1 commands once, not per copy");
}

#[tokio::test(start_paused = true)]
async fn restart_holds_queued_jobs_and_marks_active_uncertain() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let h = start(vec![sc03h("SC03h-TEST", Sim::default())], Some(dir));
    h.service.hold_queue().unwrap();
    let printer = h.save_first_printer().await;
    let preview = h.preview("Survives restart").await;
    let job = h.enqueue(&preview, &printer, "k", 12);
    h.service.shutdown(Duration::from_secs(1)).await;
    let dir = h._dir;
    drop(h.service);
    // Restart with the same data directory.
    let transport = MockTransport::new(vec![sc03h("SC03h-TEST", Sim::default())]);
    let service = AppService::start(
        ServiceConfig { data_dir: Some(path), pdfium_dirs: vec![], version: "test".into() },
        Arc::new(transport.clone()),
        Arc::new(|_e: AppEvent| {}),
    )
    .unwrap();
    let snapshot = service.list_jobs().unwrap();
    assert!(snapshot.held);
    assert_eq!(snapshot.jobs[0].id, job.id);
    assert_eq!(snapshot.jobs[0].state, JobState::Held);
    assert_eq!(service.list_printers().unwrap().len(), 1);
    assert_eq!(transport.log().connects, 0, "nothing is sent just because the app restarted");
    drop(dir);
}

#[tokio::test(start_paused = true)]
async fn retry_creates_a_linked_new_job() {
    let mut h = start(vec![sc03h("SC03h-TEST", Sim { statuses: vec![0x02, 0x00], ..Sim::default() })], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Retry").await;
    let first = h.wait_terminal(&h.enqueue(&preview, &printer, "k", 12).id).await;
    assert_eq!(first.error.unwrap().code, ErrorCode::CoverOpen);
    let retry = h.service.retry_job(&first.id, "retry-1".into()).unwrap();
    assert_ne!(retry.id, first.id);
    assert_eq!(retry.retry_of.as_deref(), Some(first.id.as_str()));
    let done = h.wait_terminal(&retry.id).await;
    assert_eq!(done.state, JobState::Completed, "{:?}", done.error);
    assert_eq!(h.service.get_job(&first.id).unwrap().state, JobState::Failed, "terminal jobs are immutable");
}

#[tokio::test(start_paused = true)]
async fn queue_shows_a_kept_job_page_but_not_a_completed_one() {
    let mut h = start(vec![sc03h("SC03h-TEST", Sim { statuses: vec![0x02, 0x00], ..Sim::default() })], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Kept").await;
    let shown = nuprint_render::decode_png_mono(&h.service.preview_png(&preview.snapshot_id).unwrap()).unwrap();

    // Failed (cover open): the content is kept for a retry, so the queue can show it.
    let failed = h.wait_terminal(&h.enqueue(&preview, &printer, "k1", 12).id).await;
    assert!(failed.content_retained);
    let page = h.service.job_page_png(&failed.id).expect("kept page");
    assert_eq!(nuprint_render::decode_png_mono(&page).unwrap(), shown);

    // Completed: the print data is deleted, but the queue keeps the first page as a preview.
    let done = h.wait_terminal(&h.enqueue(&preview, &printer, "k2", 12).id).await;
    assert_eq!(done.state, JobState::Completed, "{:?}", done.error);
    assert!(!done.content_retained);
    let kept = h.service.job_page_png(&done.id).expect("kept preview");
    assert_eq!(nuprint_render::decode_png_mono(&kept).unwrap(), shown);
    assert!(h.service.retry_job(&done.id, "again".into()).is_err(), "the preview is not print data");
    h.service.remove_job(&done.id).unwrap();
    assert!(h.service.job_page_png(&done.id).is_none(), "removing the job removes its preview");
    assert!(h.service.job_page_png("no-such-job").is_none());
}

#[tokio::test(start_paused = true)]
async fn check_printer_reads_status_and_firmware_without_printing() {
    let h = start(vec![sc03h("SC03h-TEST", Sim::default())], None);
    let printer = h.save_first_printer().await;
    let check = h.service.check_printer(&printer.id).await.unwrap();
    assert!(check.ready);
    assert_eq!(check.status_text, "Ready");
    assert_eq!(check.raw_status_hex, "001227");
    assert_eq!(check.firmware.as_deref(), Some("3.0.49"));
    assert!(h.print_bytes_written().is_empty());
    assert_eq!(h.service.list_printers().unwrap()[0].firmware.as_deref(), Some("3.0.49"));
}

#[tokio::test(start_paused = true)]
async fn failed_check_is_recorded_in_the_log() {
    let h = start(vec![sc03h("SC03h-TEST", Sim { mute: true, ..Sim::default() })], None);
    let printer = h.save_first_printer().await;
    let error = h.service.check_printer(&printer.id).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::StatusTimeout);
    let logged = h.service.logs().iter().any(|l| l.level == LogLevel::Warn && l.message.contains(&error.message));
    assert!(logged, "the failure must reach the activity log and diagnostics export");
}

#[tokio::test(start_paused = true)]
async fn inspection_finds_the_device_again_and_writes_nothing() {
    let h = start(vec![sc03h("SC03h-TEST", Sim::default())], None);
    let found = h.service.find_first_supported(Duration::from_secs(5)).await.unwrap();
    let scans = h.transport.log().scans;
    let inspection = h.service.inspect_device(&found.discovery_id).await.unwrap();
    assert_eq!(h.transport.log().scans, scans + 1, "an earlier scan's OS handle may be stale");
    assert_eq!(inspection.compatible[0].profile_id, PROFILE);
    assert!(h.transport.log().writes.is_empty());
    assert_eq!(h.transport.log().disconnects, 1);
}

fn slow_to_connect() -> MockTransport {
    MockTransport::new(vec![sc03h("SC03h-TEST", Sim::default())]).with_connect_delay(Duration::from_secs(20))
}

#[tokio::test(start_paused = true)]
async fn cancelled_inspection_frees_bluetooth_for_the_next_action() {
    let h = start_on(slow_to_connect(), None);
    let printer = h.save_first_printer().await;
    let found = h.service.find_first_supported(Duration::from_secs(5)).await.unwrap();
    let service = h.service.clone();
    let inspection = tokio::spawn(async move { service.inspect_device(&found.discovery_id).await });
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.service.cancel_inspections();
    assert_eq!(inspection.await.unwrap().unwrap_err().code, ErrorCode::Cancelled);
    assert!(h.service.check_printer(&printer.id).await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn second_inspection_waits_for_the_first_instead_of_failing_busy() {
    let h = start_on(slow_to_connect(), None);
    let found = h.service.find_first_supported(Duration::from_secs(5)).await.unwrap();
    let (service, id) = (h.service.clone(), found.discovery_id.clone());
    let first = tokio::spawn(async move { service.inspect_device(&id).await });
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(h.service.inspect_device(&found.discovery_id).await.is_ok());
    assert!(first.await.unwrap().is_ok());
    assert_eq!(h.transport.log().connects, 2);
}

#[tokio::test(start_paused = true)]
async fn busy_refusal_says_a_print_is_in_progress() {
    let mut h = start_on(slow_to_connect(), None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Busy").await;
    let job = h.enqueue(&preview, &printer, "k", 12);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let error = h.service.check_printer(&printer.id).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::PrinterBusy);
    assert!(error.message.starts_with("A print is in progress"), "{}", error.message);
    assert_eq!(h.wait_terminal(&job.id).await.state, JobState::Completed);
}

#[tokio::test(start_paused = true)]
async fn preview_for_other_profile_revision_or_stale_generation_is_rejected() {
    let h = start(vec![sc03h("SC03h-TEST", Sim::default())], None);
    let newer = PreviewRequest { generation: 5.0, profile_id: PROFILE.into(), source: SourceSpec::TestPattern };
    h.service.render_preview(newer).await.unwrap();
    let older = PreviewRequest { generation: 4.0, profile_id: PROFILE.into(), source: SourceSpec::TestPattern };
    assert_eq!(h.service.render_preview(older).await.unwrap_err().code, ErrorCode::StalePreview);
    let missing = h.service.enqueue(EnqueueRequest {
        snapshot_id: "nope".into(),
        printer_id: "nope".into(),
        copies: 1,
        density: D::Normal,
        feed_mm: 12,
        idempotency_key: "x".into(),
        title: "x".into(),
    });
    assert_eq!(missing.unwrap_err().code, ErrorCode::StalePreview);
}

#[tokio::test(start_paused = true)]
async fn adapter_off_is_reported_distinctly() {
    let transport = MockTransport::new(vec![]).with_state(nuprint_transport::AdapterState::PoweredOff);
    let service = AppService::start(
        ServiceConfig { data_dir: None, pdfium_dirs: vec![], version: "test".into() },
        Arc::new(transport),
        Arc::new(|_e: AppEvent| {}),
    )
    .unwrap();
    assert_eq!(service.adapter_status().await, AdapterStatus::PoweredOff);
    let err = service.start_scan(StartScanRequest { duration_s: None, include_unknown: None }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::AdapterOff);
}

#[tokio::test(start_paused = true)]
async fn scan_goes_ahead_while_macos_asks_for_permission() {
    let transport = MockTransport::new(vec![sc03h("SC03h-TEST", Sim::default())])
        .with_state(nuprint_transport::AdapterState::PermissionNotAsked);
    let h = start_on(transport, None);
    assert_eq!(h.service.adapter_status().await, AdapterStatus::PermissionNotAsked);
    let started = h.service.start_scan(StartScanRequest { duration_s: None, include_unknown: None }).await.unwrap();
    assert_eq!(started.adapter, AdapterStatus::PermissionNotAsked);
    // An unanswered prompt is reported as missing permission, with what to do about it.
    let unanswered = nuprint_core::AppError::from(TransportError::PermissionPending);
    assert_eq!(unanswered.code, ErrorCode::PermissionDenied);
    assert!(unanswered.message.contains("choose Allow"));
}

#[tokio::test(start_paused = true)]
async fn a_second_service_cannot_share_a_running_data_folder() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let h = start(vec![sc03h("SC03h-TEST", Sim::default())], Some(dir));
    let second = AppService::start(
        ServiceConfig { data_dir: Some(path.clone()), pdfium_dirs: vec![], version: "test".into() },
        Arc::new(MockTransport::new(vec![])),
        Arc::new(|_e: AppEvent| {}),
    );
    assert_eq!(second.err().expect("refused").code, ErrorCode::DataDirInUse);
    // A separate folder is fine: that is how a second window gets its own queue.
    let other = tempfile::tempdir().unwrap();
    let separate = AppService::start(
        ServiceConfig { data_dir: Some(other.path().to_path_buf()), pdfium_dirs: vec![], version: "test".into() },
        Arc::new(MockTransport::new(vec![])),
        Arc::new(|_e: AppEvent| {}),
    );
    assert!(separate.is_ok());
    drop(h);
}

#[tokio::test(start_paused = true)]
async fn a_job_that_cannot_be_recorded_is_never_sent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let h = start(vec![sc03h("SC03h-TEST", Sim::default())], Some(dir));
    h.service.hold_queue().unwrap();
    let printer = h.save_first_printer().await;
    let preview = h.preview("Not recorded").await;
    let job = h.enqueue(&preview, &printer, "k", 12);
    // Make recording an attempt fail, as a full disk or a locked database would.
    rusqlite::Connection::open(path.join("state.sqlite3"))
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER block_attempts BEFORE INSERT ON attempts BEGIN SELECT RAISE(ABORT, 'blocked'); END;",
        )
        .unwrap();
    h.service.resume_queue().unwrap();
    tokio::time::sleep(Duration::from_secs(10)).await;

    assert_eq!(h.transport.log().connects, 0, "nothing is sent for a job that was not recorded");
    assert!(h.transport.log().writes.is_empty());
    let snapshot = h.service.list_jobs().unwrap();
    assert!(snapshot.held, "the queue pauses instead of retrying in a loop");
    let stored = snapshot.jobs.iter().find(|j| j.id == job.id).unwrap();
    assert_eq!(stored.state, JobState::Queued, "the durable state still says it never started");
}

#[tokio::test(start_paused = true)]
async fn cancelling_a_queued_job_prints_nothing() {
    let h = start(vec![sc03h("SC03h-TEST", Sim::default())], None);
    h.service.hold_queue().unwrap();
    let printer = h.save_first_printer().await;
    let preview = h.preview("Cancelled before start").await;
    let job = h.enqueue(&preview, &printer, "k", 12);
    h.service.cancel_job(&job.id).unwrap();
    h.service.resume_queue().unwrap();
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(h.transport.log().connects, 0);
    let stored = h.service.list_jobs().unwrap().jobs.into_iter().find(|j| j.id == job.id).unwrap();
    assert_eq!(stored.state, JobState::Cancelled);
}

#[tokio::test(start_paused = true)]
async fn redacted_diagnostics_leave_no_printer_name_or_title_anywhere() {
    let mut h = start(vec![sc03h("SC03h-PRIVATE", Sim::default())], None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Some text").await;
    let job = h
        .service
        .enqueue(EnqueueRequest {
            snapshot_id: preview.snapshot_id.clone(),
            printer_id: printer.id.clone(),
            copies: 1,
            density: D::Normal,
            feed_mm: 12,
            idempotency_key: "k".into(),
            title: "Sentinel secret title".into(),
        })
        .unwrap();
    h.wait_terminal(&job.id).await;
    h.service.check_printer(&printer.id).await.unwrap();
    // Even after the printer's own records are gone, the log must not have kept its name.
    let redacted = serde_json::to_string(&h.service.diagnostics_report(false).await.unwrap()).unwrap();
    for secret in ["SC03h-PRIVATE", "Sentinel secret title"] {
        assert!(!redacted.contains(secret), "{secret} leaked: {redacted}");
    }
    assert!(redacted.contains("<redacted>"));
    assert!(redacted.contains("Queued"), "the rest of the log is still useful");
    // Identifiers are included only on request.
    let full = serde_json::to_string(&h.service.diagnostics_report(true).await.unwrap()).unwrap();
    assert!(full.contains("SC03h-PRIVATE") && full.contains("Sentinel secret title"));
    assert!(!full.contains('\u{1}') && !full.contains('\u{2}'), "markers never leave the service");
    assert!(h.service.logs().iter().all(|l| !l.message.contains('\u{1}')));
}

fn slow_check(
    h: &Harness,
    printer: &SavedPrinter,
) -> tokio::task::JoinHandle<Result<PrinterCheck, nuprint_core::AppError>> {
    let (service, id) = (h.service.clone(), printer.id.clone());
    tokio::spawn(async move { service.check_printer(&id).await })
}

/// Wait until a check started by [`slow_check`] has found the printer and is connecting.
async fn until_connecting(h: &Harness) {
    while h.transport.log().scans < 2 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_connection_check_keeps_settings_changed_while_it_waited() {
    let connect =
        MockTransport::new(vec![sc03h("SC03h-TEST", Sim::default())]).with_connect_delay(Duration::from_secs(2));
    let h = start_on(connect, None);
    let printer = h.save_first_printer().await;
    let check = slow_check(&h, &printer);
    until_connecting(&h).await;
    let changed = PrinterSettings { density: D::Dark, feed_mm: 3 };
    h.service.update_printer_settings(&printer.id, changed).unwrap();
    let check = check.await.unwrap().unwrap();

    let saved = h.service.list_printers().unwrap().remove(0);
    assert_eq!(saved.settings, changed, "the check did not restore the old settings");
    assert_eq!(saved.firmware.as_deref(), check.firmware.as_deref(), "the check still records what it read");
}

#[tokio::test(start_paused = true)]
async fn a_connection_check_does_not_bring_back_a_removed_printer() {
    let connect =
        MockTransport::new(vec![sc03h("SC03h-TEST", Sim::default())]).with_connect_delay(Duration::from_secs(2));
    let h = start_on(connect, None);
    let printer = h.save_first_printer().await;
    let check = slow_check(&h, &printer);
    until_connecting(&h).await;
    h.service.remove_printer(&printer.id).unwrap();
    check.await.unwrap().unwrap();
    assert!(h.service.list_printers().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_profile_edit_that_cannot_be_saved_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let h = start(vec![sc03h("SC03h-TEST", Sim::default())], Some(dir));
    let printer = h.save_first_printer().await;
    let printer =
        h.service.save_printer_profile(edited(&h, &printer, |doc| doc["driver"]["print_speed"] = 11.into())).unwrap();
    rusqlite::Connection::open(path.join("state.sqlite3"))
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER block_profiles BEFORE INSERT ON custom_profiles BEGIN SELECT RAISE(ABORT, 'blocked'); END;",
        )
        .unwrap();
    let failed = h.service.save_printer_profile(edited(&h, &printer, |doc| doc["driver"]["print_speed"] = 12.into()));
    assert!(failed.is_err());

    assert_eq!(h.service.list_printers().unwrap()[0].profile_revision, 1);
    let listed = h.service.profiles().into_iter().find(|p| p.id == printer.profile_id).unwrap();
    assert_eq!(listed.revision, 1, "the unsaved revision was never offered");
    // A fresh preview is still for the revision the printer uses, so it can be printed.
    h.service.hold_queue().unwrap();
    let preview = h
        .service
        .render_preview(PreviewRequest {
            generation: 1.0,
            profile_id: printer.profile_id.clone(),
            source: SourceSpec::TestPattern,
        })
        .await
        .unwrap();
    assert_eq!(preview.profile_revision, 1);
    h.enqueue(&preview, &printer, "after-failed-save", 0);
}

/// Connects at once, then spends 20 s discovering services.
fn slow_to_discover() -> MockTransport {
    MockTransport::new(vec![sc03h("SC03h-TEST", Sim::default())]).with_discovery_delay(Duration::from_secs(20))
}

#[tokio::test(start_paused = true)]
async fn cancelling_while_services_are_discovered_disconnects_first() {
    let mut h = start_on(slow_to_discover(), None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Cancelled while connecting").await;
    let job = h.enqueue(&preview, &printer, "k", 12);
    while h.transport.log().connects == 0 {
        tokio::task::yield_now().await;
    }
    h.service.cancel_job(&job.id).unwrap();
    let done = h.wait_terminal(&job.id).await;
    assert_eq!(done.state, JobState::Cancelled);
    assert!(!done.partial_output_possible);
    let log = h.transport.log();
    assert_eq!((log.connects, log.disconnects), (1, 1), "the half-open connection was closed");
    assert!(log.writes.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_inspection_disconnects_before_bluetooth_is_free() {
    let h = start_on(slow_to_discover(), None);
    let found = h.service.find_first_supported(Duration::from_secs(5)).await.unwrap();
    let service = h.service.clone();
    let inspection = tokio::spawn(async move { service.inspect_device(&found.discovery_id).await });
    while h.transport.log().connects == 0 {
        tokio::task::yield_now().await;
    }
    h.service.cancel_inspections();
    assert_eq!(inspection.await.unwrap().unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(h.transport.log().disconnects, 1, "disconnected before the inspection returned");
}

#[tokio::test(start_paused = true)]
async fn a_link_that_stops_answering_fails_the_job_and_disconnects() {
    let connect = MockTransport::new(vec![sc03h("SC03h-TEST", Sim::default())]).with_hanging_connection_state();
    let mut h = start_on(connect, None);
    let printer = h.save_first_printer().await;
    let preview = h.preview("Stalled").await;
    let job = h.enqueue(&preview, &printer, "k", 12);
    let done = h.wait_terminal(&job.id).await;
    assert_eq!(done.state, JobState::Failed, "it stopped before the first print write");
    assert_eq!(done.error.unwrap().code, ErrorCode::ConnectionFailed);
    assert_eq!(h.transport.log().disconnects, 1);
}

/// A mock printer whose advertised name can change after it was saved.
struct RenamingTransport {
    inner: MockTransport,
    advertisement: Advertisement,
    renamed: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl nuprint_transport::Transport for RenamingTransport {
    fn kind(&self) -> nuprint_transport::TransportKind {
        nuprint_transport::TransportKind::BleGatt
    }

    async fn adapter_state(&self) -> nuprint_transport::AdapterState {
        nuprint_transport::AdapterState::PoweredOn
    }

    async fn scan(
        &self,
        _duration: Duration,
        sink: mpsc::Sender<Advertisement>,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(), TransportError> {
        let mut advertisement = self.advertisement.clone();
        if self.renamed.load(Ordering::SeqCst) {
            advertisement.local_name = Some("RENAMED-PRIVATE".into());
        }
        let _ = sink.send(advertisement).await;
        Ok(())
    }

    async fn connect(
        &self,
        backend_id: &str,
        timeout: Duration,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Box<dyn nuprint_transport::Link>, TransportError> {
        self.inner.connect(backend_id, timeout, cancel).await
    }
}

#[tokio::test(start_paused = true)]
async fn redacted_diagnostics_hide_names_in_errors_edits_and_odd_titles() {
    let device = sc03h("SC03h-PRIVATE", Sim::default());
    let transport = Arc::new(RenamingTransport {
        advertisement: device.advertisement.clone(),
        inner: MockTransport::new(vec![device]),
        renamed: false.into(),
    });
    let config = ServiceConfig { data_dir: None, pdfium_dirs: vec![], version: "test".into() };
    let service = AppService::start(config, transport.clone(), Arc::new(|_: AppEvent| {})).unwrap();
    service.hold_queue().unwrap();
    let found = service.find_first_supported(Duration::from_secs(5)).await.unwrap();
    let request = SavePrinterRequest {
        discovery_id: found.discovery_id,
        profile_id: PROFILE.into(),
        acknowledge_experimental: true,
    };
    let printer = service.save_printer(request).unwrap();

    // A name the owner typed into edited settings.
    let doc = service.printer_profile_doc(&printer.id).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&doc.current_json).unwrap();
    json["display_name"] = "Desk printer of PRIVATE-OWNER".into();
    let request = SavePrinterProfileRequest { printer_id: printer.id.clone(), json: json.to_string() };
    let printer = service.save_printer_profile(request).unwrap();

    // A title holding the end-of-redaction marker.
    let source = SourceSpec::TestPattern;
    let preview = PreviewRequest { generation: 1.0, profile_id: printer.profile_id.clone(), source };
    let preview = service.render_preview(preview).await.unwrap();
    service
        .enqueue(EnqueueRequest {
            snapshot_id: preview.snapshot_id,
            printer_id: printer.id.clone(),
            copies: 1,
            density: D::Normal,
            feed_mm: 0,
            idempotency_key: "k".into(),
            title: "Note\u{2} PRIVATE-TITLE".into(),
        })
        .unwrap();

    // The saved device now advertises a name that does not match its profile.
    transport.renamed.store(true, Ordering::SeqCst);
    let refused = service.check_printer(&printer.id).await.unwrap_err();
    assert_eq!(refused.code, ErrorCode::UnsupportedProfile);
    service.remove_printer(&printer.id).unwrap();

    let redacted = serde_json::to_string(&service.diagnostics_report(false).await.unwrap()).unwrap();
    for secret in ["SC03h-PRIVATE", "PRIVATE-OWNER", "PRIVATE-TITLE", "RENAMED-PRIVATE"] {
        assert!(!redacted.contains(secret), "{secret} leaked: {redacted}");
    }
    let full = serde_json::to_string(&service.diagnostics_report(true).await.unwrap()).unwrap();
    assert!(full.contains("PRIVATE-TITLE"), "the title itself is kept");
}
