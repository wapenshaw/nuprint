//! Hardware check that sends no print data: cancel a connection at increasing delays, then check
//! the printer each time to show Bluetooth was released.
//!
//! cargo run -p nuprint-cli --example cancel_connect -- [device-name-or-id]
//!
//! Each round starts an inspection (connect, list services, disconnect; no printer commands),
//! cancels it after the round's delay and then runs a connection check, which only queries
//! status. An early delay cancels while the link is being established, a later one during
//! service discovery, and the longest lets the inspection finish. The timeline shows whether
//! "Disconnected" or the inspection's end came before the next check connected.

use std::sync::Arc;
use std::time::{Duration, Instant};

use nuprint_core::api::{AppEvent, SavePrinterRequest};
use nuprint_core::{AppService, ServiceConfig};
use nuprint_transport::ble::BleTransport;

const DELAYS_MS: [u64; 7] = [0, 100, 250, 500, 1000, 2000, 6000];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let start = Instant::now();
    let stamp = move || format!("{:>7.0} ms", start.elapsed().as_secs_f64() * 1000.0);
    let sink: Arc<dyn nuprint_core::EventSink> = Arc::new(move |event: AppEvent| {
        if let AppEvent::Log { line } = event {
            println!("{}   log  {}", stamp(), line.message);
        }
    });
    let config = ServiceConfig { data_dir: None, pdfium_dirs: Vec::new(), version: env!("CARGO_PKG_VERSION").into() };
    let service = AppService::start(config, Arc::new(BleTransport::new()), sink)?;

    let device = match std::env::args().nth(1) {
        Some(name) => service.find_device(&name, Duration::from_secs(10)).await?,
        None => service.find_first_supported(Duration::from_secs(10)).await?,
    };
    let profile_id = device.candidates.first().map(|c| c.profile_id.clone()).unwrap_or_default();
    let printer = service.save_printer(SavePrinterRequest {
        discovery_id: device.discovery_id.clone(),
        profile_id,
        acknowledge_experimental: false,
    })?;

    let mut failed = 0;
    for delay in DELAYS_MS {
        println!("\n--- cancel after {delay} ms");
        let started = Instant::now();
        let inspection = {
            let service = service.clone();
            let id = device.discovery_id.clone();
            tokio::spawn(async move { service.inspect_device(&id).await })
        };
        tokio::time::sleep(Duration::from_millis(delay)).await;
        service.cancel_inspections();
        let outcome = match inspection.await? {
            Ok(_) => "finished before the cancel".to_string(),
            Err(e) => format!("{:?}: {}", e.code, e.message),
        };
        println!("inspection ended after {} ms: {outcome}", started.elapsed().as_millis());

        let checked = Instant::now();
        match service.check_printer(&printer.id).await {
            Ok(check) => println!(
                "check took {} ms: ready={} status={}",
                checked.elapsed().as_millis(),
                check.ready,
                check.status_text
            ),
            Err(e) => {
                failed += 1;
                println!("check FAILED after {} ms: {:?}: {}", checked.elapsed().as_millis(), e.code, e.message);
            }
        }
    }
    println!("\n{} of {} checks after a cancel failed", failed, DELAYS_MS.len());
    service.shutdown(Duration::from_secs(12)).await;
    Ok(())
}
