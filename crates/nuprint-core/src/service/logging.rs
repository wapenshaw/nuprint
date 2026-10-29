//! The in-memory log, the log file, and the privacy markers that let diagnostics redact it.

use std::sync::Arc;
use std::time::Duration;

use crate::api::*;
use crate::session::LogFn;
use crate::store::now_ms;

use super::{AppService, relock};

const MAX_LOG_LINES: usize = 500;

/// How long shutdown and a diagnostics export wait for buffered log lines to reach the file.
const LOG_FLUSH_WAIT: Duration = Duration::from_secs(2);

/// Log text that identifies a printer, a document or a person is wrapped in these markers where it
/// is written. They are stripped for the screen and the log file, and the diagnostics export
/// replaces what they enclose unless identifiers were asked for. Redacting at the source keeps
/// working for printers and jobs that have since been removed.
const PRIVATE_START: char = '\u{1}';
const PRIVATE_END: char = '\u{2}';

pub(super) fn private(text: &str) -> String {
    // A marker inside the text would end the span early and leave the rest of it unredacted.
    let text = text.replace([PRIVATE_START, PRIVATE_END], "");
    format!("{PRIVATE_START}{text}{PRIVATE_END}")
}

/// `text` with every marked span replaced (`Some`) or just unmarked (`None`).
fn unmark(text: &str, replacement: Option<&str>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for c in text.chars() {
        match c {
            PRIVATE_START => {
                inside = true;
                if let Some(r) = replacement {
                    out.push_str(r);
                }
            }
            PRIVATE_END => inside = false,
            _ if inside && replacement.is_some() => {}
            _ => out.push(c),
        }
    }
    out
}

impl AppService {
    pub fn log(&self, level: LogLevel, message: String) {
        self.record(level, message, true);
    }

    /// Add a line to the in-memory log and the UI. The file gets it too when `to_file`; the
    /// writer's own health notices pass false so a failing file cannot log about itself.
    pub(super) fn record(&self, level: LogLevel, message: String, to_file: bool) {
        let at_ms = now_ms();
        // Everything that allocates happens before the lock; the markers stay in the ring.
        let plain = unmark(&message, None);
        let file_line = to_file.then(|| format!("{at_ms} {level:?} {plain}\n"));
        let line = {
            // A bounded ring that is only pushed and popped whole, so a panic elsewhere while it
            // was held cannot leave it half-changed; logging must keep working after a failure.
            let mut logs = relock(&self.inner.logs);
            // Sequence, ring order and file order are all decided under this lock.
            let seq = self.next_seq();
            if logs.len() >= MAX_LOG_LINES {
                logs.pop_front();
            }
            logs.push_back(LogLine { seq, at_ms: at_ms as f64, level, message });
            if let (Some(writer), Some(text)) = (&self.inner.log_writer, file_line) {
                writer.submit(level, text);
            }
            LogLine { seq, at_ms: at_ms as f64, level, message: plain }
        };
        match level {
            LogLevel::Error => tracing::error!("{}", line.message),
            LogLevel::Warn => tracing::warn!("{}", line.message),
            LogLevel::Info => tracing::info!("{}", line.message),
            LogLevel::Debug => tracing::debug!("{}", line.message),
        }
        self.emit(AppEvent::Log { line });
    }

    /// Wait (bounded) for buffered lines to be written. Without a data folder there is no file.
    pub(super) async fn flush_log_file(&self) {
        if let Some(writer) = self.inner.log_writer.clone() {
            let _ = tokio::task::spawn_blocking(move || writer.flush(LOG_FLUSH_WAIT)).await;
        }
    }

    /// Write what is buffered and stop the file writer; later lines reach only the in-memory log.
    /// False when the writer did not finish in time; it then keeps the data folder until it does.
    pub(super) async fn close_log_file(&self) -> bool {
        match self.inner.log_writer.clone() {
            Some(writer) => tokio::task::spawn_blocking(move || writer.close(LOG_FLUSH_WAIT)).await.unwrap_or(false),
            None => true,
        }
    }

    pub(super) fn logger(&self) -> LogFn {
        let service = self.clone();
        Arc::new(move |level, message| service.log(level, message))
    }

    pub fn logs(&self) -> Vec<LogLine> {
        relock(&self.inner.logs).iter().map(|l| LogLine { message: unmark(&l.message, None), ..l.clone() }).collect()
    }

    /// The log for a diagnostics export: identifying text is replaced unless `include_identifiers`.
    pub(super) fn export_logs(&self, include_identifiers: bool) -> Vec<LogLine> {
        if include_identifiers {
            return self.logs();
        }
        let replacement = Some("<redacted>");
        relock(&self.inner.logs)
            .iter()
            .map(|l| LogLine { message: unmark(&l.message, replacement), ..l.clone() })
            .collect()
    }
}

#[cfg(test)]
#[path = "log_tests.rs"]
mod log_tests;
