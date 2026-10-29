//! Application info and the diagnostics report, with identifiers redacted unless asked for.

use std::fs;
use std::path::Path;

use nuprint_render::RENDERER_VERSION;

use crate::api::*;
use crate::error::{AppError, AppResult};
use crate::store::now_ms;

use super::{AppService, CUSTOM_PROFILE_PREFIX, blocking_io, relock};

impl AppService {
    pub fn app_info(&self) -> AppInfo {
        let pdf = self.pdf_engine();
        AppInfo {
            version: self.inner.config.version.clone(),
            renderer_version: RENDERER_VERSION.to_owned(),
            os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            data_dir: self.inner.config.data_dir.as_ref().map(|d| d.display().to_string()),
            pdf_available: pdf.is_ok(),
            pdf_error: pdf.err().and_then(|e| e.detail),
            persistent: self.inner.config.data_dir.is_some(),
        }
    }

    // ------------------------------------------------------------ diagnostics

    /// Collect the diagnostics report. Without `include_identifiers`, names, device IDs, titles and
    /// folder paths are replaced.
    ///
    /// # Errors
    ///
    /// A storage error from listing printers or jobs.
    pub async fn diagnostics_report(&self, include_identifiers: bool) -> AppResult<DiagnosticsReport> {
        let mut app = self.app_info();
        let mut printers = self.list_printers()?;
        let mut last_check = relock(&self.inner.last_check).clone();
        let mut recent_jobs = self.list_jobs()?.jobs;
        let mut profiles = self.profiles();
        recent_jobs.truncate(20);
        if !include_identifiers {
            app.data_dir = app.data_dir.map(|_| "<app data directory>".into());
            for (i, p) in printers.iter_mut().enumerate() {
                p.name = format!("Printer {}", i + 1);
                p.backend_id = "<redacted>".into();
                p.address = None;
                // Edited settings carry a name the owner typed.
                if p.customized {
                    p.profile_name = "<redacted>".into();
                }
            }
            for profile in &mut profiles {
                if profile.id.starts_with(CUSTOM_PROFILE_PREFIX) {
                    profile.display_name = "<redacted>".into();
                }
            }
            if let Some(check) = &mut last_check {
                check.printer_id = "<redacted>".into();
            }
            // PDFium search failures name local folders.
            app.pdf_error = app.pdf_error.map(|_| "PDFium could not be loaded (folder names removed).".into());
            for job in &mut recent_jobs {
                job.title = "<redacted>".into();
                job.printer_name = "<redacted>".into();
                // Backend details can name the device or its address; the code and message stay.
                if let Some(error) = &mut job.error {
                    error.detail = None;
                }
            }
        }
        Ok(DiagnosticsReport {
            generated_ms: now_ms() as f64,
            app,
            adapter: self.adapter_status().await,
            profiles,
            printers,
            last_check,
            recent_jobs,
            log: self.export_logs(include_identifiers),
            includes_identifiers: include_identifiers,
        })
    }

    /// Write the diagnostics report to `path` as JSON.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::diagnostics_report`], [`crate::ErrorCode::Internal`] when the report cannot be
    /// serialized, and an I/O error writing `path` ([`crate::ErrorCode::StorageFull`] on a full disk,
    /// otherwise [`crate::ErrorCode::Internal`]).
    pub async fn export_diagnostics(&self, path: &Path, include_identifiers: bool) -> AppResult<()> {
        self.flush_log_file().await;
        let report = self.diagnostics_report(include_identifiers).await?;
        let json = serde_json::to_string_pretty(&report).map_err(AppError::internal)?;
        let path = path.to_path_buf();
        blocking_io(move || Ok(fs::write(path, json)?)).await
    }
}
