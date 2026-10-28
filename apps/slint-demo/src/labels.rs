//! Plain wording and number helpers, the same words the web UI uses.

use nuprint_core::api::*;
use serde_json::Value;

/// Tone ids shared with the screens: 0 neutral, 1 active, 2 good, 3 warn, 4 bad.
pub fn job_label(state: JobState) -> &'static str {
    match state {
        JobState::Held => "Held",
        JobState::Queued => "Waiting",
        JobState::Connecting => "Connecting",
        JobState::Sending => "Sending",
        JobState::WaitingForReady => "Finishing",
        JobState::Completed => "Done · printer ready",
        JobState::TransferUnconfirmed => "Sent · output unconfirmed",
        JobState::Accepted => "Accepted",
        JobState::Failed => "Failed",
        JobState::Uncertain => "Uncertain — check paper",
        JobState::Cancelled => "Cancelled",
    }
}

pub fn job_tone(state: JobState) -> i32 {
    match state {
        JobState::Held | JobState::TransferUnconfirmed | JobState::Uncertain => 3,
        JobState::Queued | JobState::Cancelled => 0,
        JobState::Connecting | JobState::Sending | JobState::WaitingForReady => 1,
        JobState::Completed | JobState::Accepted => 2,
        JobState::Failed => 4,
    }
}

pub fn support_label(level: SupportLevel) -> &'static str {
    match level {
        SupportLevel::Candidate => "Not supported",
        SupportLevel::Experimental => "Experimental",
        SupportLevel::VerifiedBasic => "Verified",
        SupportLevel::ReleaseSupported => "Supported",
    }
}

pub fn support_tone(level: SupportLevel) -> i32 {
    match level {
        SupportLevel::Candidate => 4,
        SupportLevel::Experimental => 3,
        SupportLevel::VerifiedBasic | SupportLevel::ReleaseSupported => 2,
    }
}

/// (label, explanation, tone)
pub fn adapter_text(status: AdapterStatus) -> (&'static str, &'static str, i32) {
    match status {
        AdapterStatus::Ready => ("Bluetooth is on", "Turn the printer on and keep it close, then find it.", 2),
        AdapterStatus::PoweredOff => ("Bluetooth is off", "Turn Bluetooth on in the system settings, then recheck.", 3),
        AdapterStatus::Absent => {
            ("No Bluetooth adapter", "This computer has no Bluetooth Low Energy adapter NuPrint can use.", 4)
        }
        AdapterStatus::PermissionDenied => {
            ("Bluetooth is blocked", "Allow NuPrint to use Bluetooth in the system privacy settings, then recheck.", 4)
        }
        AdapterStatus::PermissionNotAsked => (
            "Bluetooth permission needed",
            "The system will ask for permission the first time NuPrint looks for printers.",
            3,
        ),
        AdapterStatus::PermissionPending => {
            ("Waiting for permission", "Answer the system's Bluetooth permission prompt.", 3)
        }
        AdapterStatus::Unknown => {
            ("Bluetooth state unknown", "Try finding printers; the result will say what is wrong.", 0)
        }
    }
}

pub fn time_of_day(at_ms: f64) -> String {
    let secs = (at_ms / 1000.0) as u64 % 86_400;
    format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

pub fn level_label(level: LogLevel) -> (&'static str, i32) {
    match level {
        LogLevel::Info => ("INFO", 0),
        LogLevel::Warn => ("WARN", 3),
        LogLevel::Error => ("ERROR", 4),
        LogLevel::Debug => ("DEBUG", 0),
    }
}

pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

pub fn format_mm(mm: f32) -> String {
    let rounded = (mm * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 { format!("{} mm", rounded as i64) } else { format!("{rounded:.1} mm") }
}

pub fn format_bytes(bytes: f64) -> String {
    if bytes >= 1_048_576.0 {
        format!("{:.1} MB", bytes / 1_048_576.0)
    } else {
        format!("{:.0} KB", (bytes / 1024.0).max(1.0))
    }
}

pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The first non-empty line of the text, without a leading heading mark.
pub fn title_from_text(text: &str) -> String {
    let line = text.lines().map(|l| l.trim().trim_start_matches("# ").trim()).find(|l| !l.is_empty());
    let line = line.unwrap_or("Text");
    let mut title: String = line.chars().take(40).collect();
    if line.chars().count() > 40 {
        title.push('…');
    }
    title
}

// ---------------------------------------------------------------- advanced settings

/// The settings people tune while testing; everything else is in the JSON.
pub const QUICK_FIELDS: [(&str, &[&str]); 6] = [
    ("Heat: light", &["driver", "energy", "light"]),
    ("Heat: normal", &["driver", "energy", "normal"]),
    ("Heat: dark", &["driver", "energy", "dark"]),
    ("Print speed", &["driver", "print_speed"]),
    ("Chunk size (bytes)", &["transport", "max_chunk_bytes"]),
    ("Delay between chunks (ms)", &["transport", "pacing", "chunk_delay_ms"]),
];

pub fn get_path<'a>(doc: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(doc, |node, key| node.get(*key))
}

pub fn set_path(doc: &mut Value, path: &[&str], value: u64) {
    let Some((head, rest)) = path.split_first() else { return };
    if !doc.is_object() {
        *doc = Value::Object(Default::default());
    }
    let map = doc.as_object_mut().expect("object");
    if rest.is_empty() {
        map.insert((*head).to_owned(), Value::from(value));
    } else {
        set_path(map.entry((*head).to_owned()).or_insert_with(|| Value::Object(Default::default())), rest, value);
    }
}

/// A quarter to twice the built-in normal heat; other fields have fixed limits.
pub fn quick_range(index: usize, base: Option<&Value>) -> (u64, u64) {
    match index {
        0..=2 => {
            let normal =
                base.and_then(|b| get_path(b, &["driver", "energy", "normal"])).and_then(Value::as_u64).unwrap_or(5000);
            (normal / 4, normal * 2)
        }
        3 => (1, 255),
        4 => (20, 512),
        _ => (0, 120_000),
    }
}
