//! `nuprint`: headless diagnostics and printing over the same application service the
//! desktop app uses. State is kept in memory for the life of one command.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};
use nuprint_core::api::*;
use nuprint_core::{AppError, AppService, ServiceConfig};
use nuprint_protocol::Density;
use nuprint_transport::ble::BleTransport;
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(name = "nuprint", version, about = "Print directly to supported Bluetooth LE thermal printers")]
#[command(after_help = "Exit status: 0 printed and confirmed (or preview saved), 1 error, \
2 transfer finished without printer confirmation, 3 outcome uncertain (check the paper), 130 cancelled.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List built-in printer profiles and their support level on this OS
    Profiles,
    /// Find nearby printers
    Scan {
        /// Scan duration in seconds
        #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u32).range(1..=30))]
        seconds: u32,
        /// Also list devices that match no profile
        #[arg(long)]
        all: bool,
    },
    /// Connect and query status without printing
    Diagnose {
        #[command(flatten)]
        target: Target,
        /// Save a diagnostic JSON report
        #[arg(long)]
        output: Option<PathBuf>,
        /// Keep device names and addresses in the saved report (redacted by default)
        #[arg(long)]
        include_identifiers: bool,
    },
    /// List any device's Bluetooth services without sending printer commands (for reporting new models)
    Inspect {
        /// Device name, address or OS identifier, as shown by `scan --all`
        #[arg(long)]
        device: String,
        /// Seconds to search for the device
        #[arg(long, default_value_t = 12, value_parser = clap::value_parser!(u64).range(1..=120))]
        find_seconds: u64,
        /// Save the result as JSON
        #[arg(long)]
        output: Option<PathBuf>,
        /// Keep the full device name and address in the saved file (unit suffix masked by default)
        #[arg(long)]
        include_identifiers: bool,
    },
    /// Print the built-in test pattern
    Test {
        #[command(flatten)]
        job: JobArgs,
    },
    /// Print text
    Text {
        text: String,
        /// Font size in pixels
        #[arg(long, default_value_t = 24, value_parser = clap::value_parser!(u16).range(8..=160))]
        size: u16,
        #[arg(long)]
        bold: bool,
        /// Use the monospaced font
        #[arg(long)]
        mono: bool,
        #[command(flatten)]
        job: JobArgs,
    },
    /// Print a picture or a PDF page
    File {
        path: PathBuf,
        /// PDF page, starting at 1
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
        page: u32,
        /// PDF password, if the document is protected
        #[arg(long)]
        password: Option<String>,
        /// Crisp black/white instead of photo dithering
        #[arg(long)]
        threshold: bool,
        /// Higher-contrast dithering (Atkinson) instead of Floyd-Steinberg
        #[arg(long, conflicts_with = "threshold")]
        atkinson: bool,
        /// Rotate clockwise by degrees
        #[arg(long, default_value = "0", value_parser = ["0", "90", "180", "270"])]
        rotate: String,
        #[command(flatten)]
        job: JobArgs,
    },
}

#[derive(Args)]
struct Target {
    /// Device name, address or OS identifier (default: first supported printer found)
    #[arg(long)]
    device: Option<String>,
    /// Profile ID to use when a device matches more than one, or for --preview
    #[arg(long)]
    profile: Option<String>,
    /// Allow profiles that are experimental on this operating system
    #[arg(long)]
    allow_experimental: bool,
    /// Seconds to search for the device
    #[arg(long, default_value_t = 12, value_parser = clap::value_parser!(u64).range(1..=120))]
    find_seconds: u64,
}

#[derive(Args)]
struct JobArgs {
    #[command(flatten)]
    target: Target,
    /// Print darkness (default: the profile's default)
    #[arg(long, value_parser = ["light", "normal", "dark"])]
    darkness: Option<String>,
    /// Extra paper after the image in nominal mm (default: the profile's default)
    #[arg(long)]
    feed_mm: Option<u16>,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..))]
    copies: u16,
    /// Save the exact print bitmap as PNG only; do not connect or print
    #[arg(long)]
    preview: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .init();
    let cli = Cli::parse();

    let (tx, events) = mpsc::unbounded_channel();
    let sink: Arc<dyn nuprint_core::EventSink> = Arc::new(move |event: AppEvent| {
        let _ = tx.send(event);
    });
    let config = ServiceConfig {
        data_dir: None,
        pdfium_dirs: nuprint_render::pdfium_search_dirs(&[]),
        version: env!("CARGO_PKG_VERSION").into(),
    };
    let service = match AppService::start(config, Arc::new(BleTransport::new()), sink) {
        Ok(service) => service,
        Err(e) => return fail(&e.into()),
    };
    let mut app = Runner { service, events };
    let code = match app.run(cli.command).await {
        Ok(code) => code,
        Err(e) => fail(&e),
    };
    app.service.shutdown(Duration::from_secs(12)).await;
    code
}

fn fail(error: &anyhow::Error) -> ExitCode {
    match error.downcast_ref::<AppError>() {
        Some(e) => {
            eprintln!("error [{}]: {}", enum_str(&e.code), e.message);
            if let Some(detail) = &e.detail {
                eprintln!("  detail: {detail}");
            }
        }
        None => eprintln!("error: {error:#}"),
    }
    ExitCode::FAILURE
}

struct Runner {
    service: AppService,
    events: mpsc::UnboundedReceiver<AppEvent>,
}

impl Runner {
    async fn run(&mut self, command: Command) -> Result<ExitCode> {
        match command {
            Command::Profiles => self.profiles(),
            Command::Scan { seconds, all } => self.scan(seconds, all).await,
            Command::Diagnose { target, output, include_identifiers } => {
                self.diagnose(&target, output.as_deref(), include_identifiers).await
            }
            Command::Inspect { device, find_seconds, output, include_identifiers } => {
                self.inspect(&device, find_seconds, output.as_deref(), include_identifiers).await
            }
            Command::Test { job } => {
                self.precheck(&job)?;
                self.print(job, SourceSpec::TestPattern, "Test pattern").await
            }
            Command::Text { text, size, bold, mono, job } => {
                self.precheck(&job)?;
                let title = text.lines().next().unwrap_or("Text").chars().take(40).collect::<String>();
                let spec = SourceSpec::Text(TextSpec {
                    text,
                    size_px: size,
                    family: if mono { FontFamily::Mono } else { FontFamily::Sans },
                    bold,
                    headings: false,
                    ..TextSpec::default()
                });
                self.print(job, spec, &title).await
            }
            Command::File { path, page, password, threshold, atkinson, rotate, job } => {
                self.precheck(&job)?;
                let adjust = ImageAdjust {
                    tone: if threshold {
                        Tone::Threshold
                    } else if atkinson {
                        Tone::Atkinson
                    } else {
                        Tone::Dither
                    },
                    rotation: match rotate.as_str() {
                        "90" => Rotation::R90,
                        "180" => Rotation::R180,
                        "270" => Rotation::R270,
                        _ => Rotation::R0,
                    },
                    brightness: 0,
                    contrast: 0,
                };
                let title = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "File".into());
                let info = self.service.import_source(&path).await?;
                let spec = match info.kind {
                    SourceKind::Image => SourceSpec::Image { source_id: info.source_id, adjust },
                    SourceKind::Pdf => SourceSpec::Pdf { source_id: info.source_id, page, adjust, password },
                };
                self.print(job, spec, &title).await
            }
        }
    }

    fn profiles(&self) -> Result<ExitCode> {
        for p in self.service.profiles() {
            println!("{}  r{}  {}  [{}]", p.id, p.revision, p.display_name, enum_str(&p.support));
            println!(
                "    {} dots @ {} dots/mm, feed {}-{} mm (default {}), densities {}",
                p.width_dots,
                p.dots_per_mm,
                p.feed.min_mm,
                p.feed.max_mm,
                p.feed.default_mm,
                p.densities.iter().map(|d| d.as_str()).collect::<Vec<_>>().join("/"),
            );
            println!("    {}", p.support_note);
        }
        Ok(ExitCode::SUCCESS)
    }

    async fn scan(&mut self, seconds: u32, all: bool) -> Result<ExitCode> {
        let started =
            self.service.start_scan(StartScanRequest { duration_s: Some(seconds), include_unknown: Some(all) }).await?;
        eprintln!("Scanning for {seconds} s...");
        let mut found = 0;
        while let Some(event) = self.events.recv().await {
            match event {
                AppEvent::ScanDevice { scan_id, device } if scan_id == started.scan_id => {
                    found += 1;
                    let profiles =
                        device.candidates.iter().map(|c| format!("{} [{}]", c.profile_id, enum_str(&c.support)));
                    println!(
                        "{:<20} {:<20} rssi {:>4}  {}",
                        device.name.as_deref().unwrap_or("(no name)"),
                        device.address.as_deref().unwrap_or(&device.discovery_id),
                        device.rssi.map(|r| r.to_string()).unwrap_or_else(|| "?".into()),
                        profiles.collect::<Vec<_>>().join(", "),
                    );
                }
                AppEvent::ScanFinished { scan_id, error, .. } if scan_id == started.scan_id => {
                    if let Some(error) = error {
                        return Err(error.into());
                    }
                    break;
                }
                _ => {}
            }
        }
        if found == 0 {
            eprintln!("No printers found. Check the printer is on and not connected to a phone.");
        }
        Ok(ExitCode::SUCCESS)
    }

    async fn diagnose(&mut self, target: &Target, output: Option<&Path>, include_ids: bool) -> Result<ExitCode> {
        let printer = self.connect_target(target).await?;
        let check = self.service.check_printer(&printer.id).await?;
        println!("Printer:   {} ({})", printer.name, printer.profile_name);
        println!("Firmware:  {}", check.firmware.as_deref().unwrap_or("unknown"));
        println!("Status:    {} (raw {})", check.status_text, check.raw_status_hex);
        println!("Ready:     {}", if check.ready { "yes" } else { "no" });
        println!("Write len: {} bytes", check.max_write_len);
        for service in &check.services {
            println!("Service {}", service.uuid);
            for c in &service.characteristics {
                println!("    {}  {}", c.uuid, c.properties.join(","));
            }
        }
        for line in &check.notifications {
            println!("Notify:    {line}");
        }
        if let Some(path) = output {
            self.service.export_diagnostics(path, include_ids).await?;
            println!("Report saved to {}", path.display());
        }
        Ok(ExitCode::SUCCESS)
    }

    /// Connect, list GATT services and disconnect. Works for devices no profile matches; sends no
    /// vendor bytes, so it is safe on unknown hardware.
    async fn inspect(
        &self,
        device: &str,
        find_seconds: u64,
        output: Option<&Path>,
        include_ids: bool,
    ) -> Result<ExitCode> {
        let found = self.service.find_device(device, Duration::from_secs(find_seconds)).await?;
        let inspection = self.service.inspect_device(&found.discovery_id).await?;
        let name = found.name.as_deref().unwrap_or("(no name)");
        println!("Device:    {name}");
        println!("Advertises: {}", list_or_none(&found.advertised_services));
        println!("Write len: {} bytes", inspection.max_write_len);
        for service in &inspection.services {
            println!("Service {}", service.uuid);
            for c in &service.characteristics {
                println!("    {}  {}", c.uuid, c.properties.join(","));
            }
        }
        let compatible: Vec<String> =
            inspection.compatible.iter().map(|m| format!("{} [{}]", m.profile_id, enum_str(&m.support))).collect();
        println!("Profiles:  {}", list_or_none(&compatible));
        for reason in &inspection.rejected {
            println!("Rejected:  {reason}");
        }
        if compatible.is_empty() && inspection.rejected.is_empty() {
            println!("No built-in profile matches this name. NuPrint will not print to it.");
        }
        if let Some(path) = output {
            let report = serde_json::json!({
                "nuprint_version": env!("CARGO_PKG_VERSION"),
                "os": std::env::consts::OS,
                "name": if include_ids { found.name.clone() } else { found.name.as_deref().map(mask_unit_suffix) },
                "address": if include_ids { found.address.clone() } else { None },
                "advertised_services": found.advertised_services,
                "max_write_len": inspection.max_write_len,
                "services": inspection.services,
                "compatible": inspection.compatible,
                "rejected": inspection.rejected,
                "includes_identifiers": include_ids,
            });
            std::fs::write(path, serde_json::to_string_pretty(&report)?)
                .with_context(|| format!("could not write {}", path.display()))?;
            println!("Report saved to {}", path.display());
        }
        Ok(ExitCode::SUCCESS)
    }

    /// Locate, validate and register the target device for this run.
    async fn connect_target(&self, target: &Target) -> Result<SavedPrinter> {
        let timeout = Duration::from_secs(target.find_seconds);
        let device = match &target.device {
            Some(device) => self.service.find_device(device, timeout).await?,
            None => self.service.find_first_supported(timeout).await?,
        };
        let name = device.name.clone().unwrap_or_else(|| device.discovery_id.clone());
        let profile_id = match (&target.profile, device.candidates.as_slice()) {
            (Some(id), candidates) => candidates
                .iter()
                .find(|c| &c.profile_id == id)
                .map(|c| c.profile_id.clone())
                .ok_or_else(|| anyhow!("{name} does not match profile {id}"))?,
            (None, [one]) => one.profile_id.clone(),
            (None, []) => bail!("{name} matches no supported printer profile"),
            (None, many) => bail!(
                "{name} matches several profiles; choose one with --profile: {}",
                many.iter().map(|c| c.profile_id.as_str()).collect::<Vec<_>>().join(", ")
            ),
        };
        eprintln!("Found {name}");
        Ok(self.service.save_printer(SavePrinterRequest {
            discovery_id: device.discovery_id,
            profile_id,
            acknowledge_experimental: target.allow_experimental,
        })?)
    }

    fn profile(&self, id: &str) -> Result<ProfileSummary> {
        self.service.profiles().into_iter().find(|p| p.id == id).ok_or_else(|| anyhow!("unknown profile {id}"))
    }

    /// `--profile`, else the best-supported profile on this OS (profiles are listed best first).
    fn preview_profile(&self, target: &Target) -> Result<ProfileSummary> {
        match &target.profile {
            Some(id) => self.profile(id),
            None => self.service.profiles().into_iter().next().context("no printer profiles are built in"),
        }
    }

    /// Reject bad options before touching the source or Bluetooth when the profile is already known.
    fn precheck(&self, job: &JobArgs) -> Result<()> {
        let known = match &job.target.profile {
            Some(id) => Some(self.profile(id)?),
            None => match self.service.profiles().as_slice() {
                [only] => Some(only.clone()),
                _ => None,
            },
        };
        if let Some(profile) = known {
            self.check_options(job, &profile)?;
        }
        Ok(())
    }

    /// Validate darkness, feed and copies against the profile; returns the darkness to print with.
    fn check_options(&self, job: &JobArgs, profile: &ProfileSummary) -> Result<Density> {
        let density = match &job.darkness {
            Some(d) => d.parse::<Density>().map_err(|_| anyhow!("unknown darkness {d}"))?,
            None => profile.default_density,
        };
        let feed_mm = job.feed_mm.unwrap_or(profile.feed.default_mm);
        self.service.check_job_options(&profile.id, density, feed_mm, job.copies)?;
        Ok(density)
    }

    async fn print(&mut self, job: JobArgs, spec: SourceSpec, title: &str) -> Result<ExitCode> {
        if let Some(path) = &job.preview {
            let profile = self.preview_profile(&job.target)?;
            let preview = self.render(&profile.id, spec).await?;
            self.service.save_preview_png(&preview.snapshot_id, path)?;
            if job.target.profile.is_none() {
                eprintln!("Previewing for {} (choose another with --profile)", profile.display_name);
            }
            println!(
                "Preview saved to {} ({}x{} dots, {:.1} mm)",
                path.display(),
                preview.width,
                preview.height,
                preview.length_mm
            );
            return Ok(ExitCode::SUCCESS);
        }

        let printer = self.connect_target(&job.target).await?;
        let profile = self.profile(&printer.profile_id)?;
        let density = self.check_options(&job, &profile)?;
        let preview = self.render(&profile.id, spec).await?;
        let summary = self.service.enqueue(EnqueueRequest {
            snapshot_id: preview.snapshot_id,
            printer_id: printer.id,
            copies: job.copies,
            density,
            feed_mm: job.feed_mm.unwrap_or(profile.feed.default_mm),
            idempotency_key: uuid::Uuid::new_v4().to_string(),
            title: title.into(),
        })?;
        eprintln!("Printing {} copy(ies), about {:.0} mm of paper. Ctrl+C cancels.", summary.copies, summary.paper_mm);
        self.wait(&summary.id).await
    }

    async fn render(&self, profile_id: &str, source: SourceSpec) -> Result<PreviewResult> {
        let preview = self
            .service
            .render_preview(PreviewRequest { generation: 1.0, profile_id: profile_id.into(), source })
            .await?;
        for warning in &preview.warnings {
            eprintln!("warning: {warning}");
        }
        Ok(preview)
    }

    async fn wait(&mut self, job_id: &str) -> Result<ExitCode> {
        let mut last_state = None;
        let mut last_pct = None;
        loop {
            let event = tokio::select! {
                event = self.events.recv() => event.context("the print service stopped")?,
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("\nCancelling...");
                    self.service.cancel_job(job_id)?;
                    continue;
                }
            };
            match event {
                AppEvent::JobProgress { job_id: id, bytes_sent, bytes_total, .. }
                    if id == job_id && bytes_total > 0.0 =>
                {
                    let pct = (bytes_sent / bytes_total * 100.0).floor() as u32;
                    if last_pct.is_none_or(|p| pct >= p + 10 || pct == 100) {
                        eprintln!("  sent {pct}%");
                        last_pct = Some(pct);
                    }
                }
                AppEvent::JobChanged { job, .. } if job.id == job_id => {
                    if last_state != Some(job.state) {
                        eprintln!("{}", job.state_text);
                        last_state = Some(job.state);
                    }
                    if job.state.is_terminal() {
                        if let Some(error) = &job.error {
                            eprintln!("  {}", error.message);
                        }
                        return Ok(ExitCode::from(match job.state {
                            JobState::Completed => 0,
                            JobState::TransferUnconfirmed | JobState::Accepted => 2,
                            JobState::Uncertain => 3,
                            JobState::Cancelled => 130,
                            _ => 1,
                        }));
                    }
                }
                AppEvent::QueueFault { fault, .. } => {
                    eprintln!("{}", fault.message);
                    return Ok(ExitCode::from(1));
                }
                _ => {}
            }
        }
    }
}

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() { "none".into() } else { items.join(", ") }
}

/// "SC03h-3DE8" → "SC03h-****": keeps the model prefix a report needs and hides the per-unit suffix.
fn mask_unit_suffix(name: &str) -> String {
    match name.rfind(['-', '_']) {
        Some(i) => format!("{}{}", &name[..=i], "*".repeat(name[i + 1..].chars().count())),
        None => name.to_owned(),
    }
}

fn enum_str<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::mask_unit_suffix;

    #[test]
    fn masks_only_the_unit_suffix() {
        assert_eq!(mask_unit_suffix("SC03h-3DE8"), "SC03h-****");
        assert_eq!(mask_unit_suffix("XC9-FL01-1A2B"), "XC9-FL01-****");
        assert_eq!(mask_unit_suffix("GB01_A1"), "GB01_**");
        assert_eq!(mask_unit_suffix("MX10"), "MX10");
    }
}
