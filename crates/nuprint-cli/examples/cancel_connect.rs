//! Hardware check that sends no print data: cancel connections at known points, then check the
//! printer each time to show Bluetooth was released.
//!
//! cargo run -p nuprint-cli --example cancel_connect -- [device-name-or-id]
//!
//! Every connection first scans until the printer is seen, as the app does, because the OS
//! forgets it after a disconnect. It times one full connection (link plus service discovery, no
//! printer commands), then cancels a connection at fractions of that time, so early rounds stop
//! while the link is being established and later ones during service discovery, and runs a
//! connection check after each; the check only queries status. Last, it cancels an inspection
//! 100 ms in, while it is still looking for the printer, six times (`SEARCH_ONLY=1` runs only
//! this part); on Windows that used to leave the next scan deaf, see `SILENT_SCAN_RESTART` in the
//! BLE transport. Each round prints how long the cancel took to return, which includes the
//! disconnect when the link was already up.

use std::sync::Arc;
use std::time::{Duration, Instant};

use nuprint_core::api::{AppEvent, SavePrinterRequest};
use nuprint_core::session::locate;
use nuprint_core::{AppService, ServiceConfig};
use nuprint_transport::Transport;
use nuprint_transport::ble::BleTransport;
use tokio_util::sync::CancellationToken;

const FRACTIONS: [f64; 6] = [0.05, 0.2, 0.4, 0.6, 0.8, 0.95];
const SEARCH_REPEATS: usize = 6;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(25);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Transport warnings, such as a silent scan being restarted, go to stderr.
    tracing_subscriber::fmt().with_writer(std::io::stderr).with_ansi(false).with_env_filter("warn").init();
    let start = Instant::now();
    let stamp = move || format!("{:>7.0} ms", start.elapsed().as_secs_f64() * 1000.0);
    let sink: Arc<dyn nuprint_core::EventSink> = Arc::new(move |event: AppEvent| {
        if let AppEvent::Log { line } = event {
            println!("{}   log  {}", stamp(), line.message);
        }
    });
    let config = ServiceConfig { data_dir: None, pdfium_dirs: Vec::new(), version: env!("CARGO_PKG_VERSION").into() };
    let transport = Arc::new(BleTransport::new());
    let service = AppService::start(config, transport.clone(), sink)?;

    let device = match std::env::args().nth(1) {
        Some(name) => service.find_device(&name, Duration::from_secs(10)).await?,
        None => service.find_first_supported(Duration::from_secs(10)).await?,
    };
    let id = device.discovery_id.clone();
    let profile_id = device.candidates.first().map(|c| c.profile_id.clone()).unwrap_or_default();
    let printer = service.save_printer(SavePrinterRequest {
        discovery_id: id.clone(),
        profile_id,
        acknowledge_experimental: false,
    })?;

    // The OS forgets the device after a disconnect, so every connection starts with a scan that
    // finds it again, as the app does; the cancel delay is timed from the connect call.
    let find_again = async || {
        locate(transport.as_ref(), Duration::from_secs(15), &CancellationToken::new(), |a| a.backend_id == id).await
    };
    find_again().await?;
    let timed = Instant::now();
    let link = transport.connect(&id, CONNECT_TIMEOUT, &CancellationToken::new()).await?;
    let full = timed.elapsed();
    let _ = link.disconnect().await;
    println!("\nOne full connection took {} ms", full.as_millis());

    let mut rounds = 0;
    let mut failed = 0;
    let mut check = async |label: String| {
        rounds += 1;
        let checked = Instant::now();
        match service.check_printer(&printer.id).await {
            Ok(c) => {
                println!("check took {} ms: ready={} status={}", checked.elapsed().as_millis(), c.ready, c.status_text)
            }
            Err(e) => {
                failed += 1;
                println!(
                    "check FAILED after {label} ({} ms): {:?}: {}",
                    checked.elapsed().as_millis(),
                    e.code,
                    e.message
                );
            }
        }
    };

    for fraction in FRACTIONS.into_iter().filter(|_| std::env::var_os("SEARCH_ONLY").is_none()) {
        let delay = full.mul_f64(fraction);
        println!("\n--- cancel a connection after {} ms ({:.0}% of a full one)", delay.as_millis(), fraction * 100.0);
        find_again().await?;
        let cancel = CancellationToken::new();
        let connecting = {
            let (transport, id, cancel) = (transport.clone(), id.clone(), cancel.clone());
            tokio::spawn(async move { transport.connect(&id, CONNECT_TIMEOUT, &cancel).await })
        };
        tokio::time::sleep(delay).await;
        let early =
            if connecting.is_finished() { " (ended BEFORE the cancel, so this round tested nothing)" } else { "" };
        let cancelled = Instant::now();
        cancel.cancel();
        let outcome = match connecting.await? {
            Ok(link) => {
                let _ = link.disconnect().await;
                format!("connected{early}; disconnected")
            }
            Err(e) => format!("{e}{early}"),
        };
        println!("returned {} ms after the cancel: {outcome}", cancelled.elapsed().as_millis());
        check(format!("a cancel at {} ms", delay.as_millis())).await;
    }

    for _ in 0..SEARCH_REPEATS {
        println!("\n--- cancel an inspection after 100 ms, while it looks for the printer");
        let inspection = {
            let (service, id) = (service.clone(), id.clone());
            tokio::spawn(async move { service.inspect_device(&id).await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        let cancelled = Instant::now();
        service.cancel_inspections();
        let outcome = match inspection.await? {
            Ok(_) => "finished before the cancel".to_string(),
            Err(e) => format!("{:?}", e.code),
        };
        println!("returned {} ms after the cancel: {outcome}", cancelled.elapsed().as_millis());
        check("a search cancelled at 100 ms".into()).await;
    }

    println!("\n{failed} of {rounds} checks after a cancel failed");
    service.shutdown(Duration::from_secs(12)).await;
    Ok(())
}
