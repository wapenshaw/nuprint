//! The service's log path end to end: ring, UI events, file and diagnostics export.

use super::*;
use nuprint_transport::mock::MockTransport;

const SECRET: &str = "Kitchen SC03h-ABCD";

fn start(dir: Option<&Path>) -> (AppService, Arc<Mutex<Vec<LogLine>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let config = ServiceConfig { data_dir: dir.map(Path::to_path_buf), pdfium_dirs: vec![], version: "test".into() };
    let service = AppService::start(
        config,
        Arc::new(MockTransport::new(vec![])),
        Arc::new(move |event| {
            if let AppEvent::Log { line } = event {
                sink.lock().unwrap().push(line);
            }
        }),
    )
    .unwrap();
    (service, events)
}

fn file_lines(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join("logs").join("nuprint.log")).unwrap_or_default().lines().map(String::from).collect()
}

#[tokio::test]
async fn private_markers_stay_in_the_ring_and_go_only_at_the_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let (service, events) = start(Some(dir.path()));
    service.log(LogLevel::Info, format!("Saved printer {}", private(SECRET)));

    // Stored representation keeps the markers so a later export can still redact the span.
    let stored: Vec<String> = service.inner.logs.lock().unwrap().iter().map(|l| l.message.clone()).collect();
    assert!(stored.iter().any(|m| m.contains(PRIVATE_START) && m.contains(SECRET)), "{stored:?}");

    // Presentation, events and the file show the text without markers.
    for line in service.logs().iter().chain(events.lock().unwrap().iter()) {
        assert!(!line.message.contains(PRIVATE_START) && !line.message.contains(PRIVATE_END), "{line:?}");
    }
    service.shutdown(Duration::from_secs(5)).await;
    let file = file_lines(dir.path()).join("\n");
    assert!(file.contains(&format!("Saved printer {SECRET}")), "{file}");
    assert!(!file.contains(PRIVATE_START) && !file.contains(PRIVATE_END));

    // The whole report: redacted by default, present when identifiers are asked for.
    let redacted = serde_json::to_string(&service.diagnostics_report(false).await.unwrap()).unwrap();
    assert!(!redacted.contains(SECRET) && redacted.contains("Saved printer <redacted>"), "{redacted}");
    let full = serde_json::to_string(&service.diagnostics_report(true).await.unwrap()).unwrap();
    assert!(full.contains(SECRET));
}

#[tokio::test]
async fn ring_events_and_file_agree_on_order_across_producers() {
    let dir = tempfile::tempdir().unwrap();
    let (service, events) = start(Some(dir.path()));
    let before = service.logs().len();
    let barrier = Arc::new(std::sync::Barrier::new(4));
    let threads: Vec<_> = (0..4)
        .map(|t| {
            let service = service.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for i in 0..100 {
                    service.log(LogLevel::Info, format!("producer {t} line {i}"));
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    service.shutdown(Duration::from_secs(5)).await;

    let ring = service.logs();
    assert!(ring.windows(2).all(|w| w[0].seq < w[1].seq), "ring is in sequence order");
    let ring_text: Vec<_> = ring.iter().skip(before).map(|l| l.message.clone()).collect();
    let file_text: Vec<_> = file_lines(dir.path())
        .into_iter()
        .filter_map(|l| l.split_once(" Info ").map(|(_, m)| m.to_string()))
        .filter(|m| m.starts_with("producer"))
        .collect();
    // 400 lines fit the ring (500) and the writer queue, so nothing is dropped here.
    assert_eq!(file_text, ring_text);
    assert_eq!(events.lock().unwrap().iter().filter(|l| l.message.starts_with("producer")).count(), 400);
}

#[tokio::test]
async fn shutdown_writes_pending_lines_and_later_lines_stay_in_memory() {
    let dir = tempfile::tempdir().unwrap();
    let (service, _events) = start(Some(dir.path()));
    for i in 0..300 {
        service.log(LogLevel::Debug, format!("pending {i}"));
    }
    service.shutdown(Duration::from_secs(5)).await;
    assert!(file_lines(dir.path()).iter().any(|l| l.ends_with("pending 299")));
    service.log(LogLevel::Info, "after shutdown".into());
    assert!(service.logs().iter().any(|l| l.message == "after shutdown"));
    assert!(!file_lines(dir.path()).iter().any(|l| l.ends_with("after shutdown")));
}

#[tokio::test]
async fn a_diagnostics_export_flushes_the_file_first() {
    let dir = tempfile::tempdir().unwrap();
    let (service, _events) = start(Some(dir.path()));
    service.log(LogLevel::Warn, "needs to be on disk".into());
    service.export_diagnostics(&dir.path().join("report.json"), false).await.unwrap();
    assert!(file_lines(dir.path()).iter().any(|l| l.ends_with("needs to be on disk")));
    service.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn an_unwritable_log_file_is_reported_once_in_memory_and_never_blocks_logging() {
    let dir = tempfile::tempdir().unwrap();
    // A directory where the log file belongs makes every open fail.
    fs::create_dir_all(dir.path().join("logs").join("nuprint.log")).unwrap();
    let (service, events) = start(Some(dir.path()));
    for i in 0..50 {
        service.log(LogLevel::Info, format!("line {i}"));
        service.flush_log_file().await;
    }
    let notices = |lines: Vec<LogLine>| lines.iter().filter(|l| l.message.contains("cannot be written")).count();
    assert_eq!(notices(service.logs()), 1);
    assert_eq!(notices(events.lock().unwrap().clone()), 1);
    assert!(service.logs().iter().any(|l| l.message == "line 49"));
    service.shutdown(Duration::from_secs(5)).await;
}
