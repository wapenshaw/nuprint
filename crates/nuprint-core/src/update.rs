//! The part of updating that both apps share: where releases are published and what the update
//! prompt says about one. Checking, downloading and installing belong to each app's updater, which
//! verifies the release's signature before installing it.

use crate::api::UpdateInfo;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// The public half of the key the release workflow signs updates with (minisign, base64). The
/// Tauri app holds the same key in tauri.conf.json, where its updater reads it.
pub const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEI0OUJGMTQ0MzJGMEZGRDgKUldUWS8vQXlSUEdidEp2b2tFN01wWWRFNGpQS1RZK2R5TVVvb1JqL3VCWURKc213SGF0RmtzeG8K";

/// The address of `file` in the newest published release, for an updater's manifest.
#[must_use]
pub fn latest_release_file(file: &str) -> String {
    format!("{REPOSITORY}/releases/latest/download/{file}")
}

/// The page of the release with this version.
#[must_use]
pub fn release_url(version: &str) -> String {
    format!("{REPOSITORY}/releases/tag/v{}", version.trim_start_matches('v'))
}

/// What the update prompt shows about a newer release; `notes` are the release's Markdown notes.
#[must_use]
pub fn update_info(version: &str, current_version: &str, notes: Option<&str>, can_install: bool) -> UpdateInfo {
    UpdateInfo {
        version: version.trim_start_matches('v').to_owned(),
        current_version: current_version.to_owned(),
        notes: notes.map(release_notes).unwrap_or_default(),
        release_url: release_url(version),
        can_install,
    }
}

/// The Markdown release notes release-please writes, as plain text for the update prompt: one
/// line per section heading and a bullet per change, without the version heading, scopes, links
/// or commit references.
#[must_use]
pub fn release_notes(markdown: &str) -> String {
    let mut out = String::new();
    for line in markdown.lines().map(str::trim) {
        if line.is_empty() || line.starts_with("## ") || line.starts_with("# ") {
            // The version heading repeats what the prompt already says.
            continue;
        }
        if let Some(heading) = line.strip_prefix("### ") {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&sentence_case(&plain(heading)));
        } else if let Some(item) = line.strip_prefix("* ").or_else(|| line.strip_prefix("- ")) {
            out.push_str("• ");
            out.push_str(&capitalised(&plain(without_scope(without_commit(item)))));
        } else {
            out.push_str(&plain(line));
        }
        out.push('\n');
    }
    out.trim_end().to_owned()
}

/// `text ([abc1234](…/commit/…))` → `text`.
fn without_commit(item: &str) -> &str {
    match item.rfind(" ([") {
        Some(at) if item.ends_with("))") => &item[..at],
        _ => item,
    }
}

/// `**core:** text` → `text`.
fn without_scope(item: &str) -> &str {
    match item.strip_prefix("**").and_then(|rest| rest.split_once(":** ")) {
        Some((_, text)) => text,
        None => item,
    }
}

/// Links become their text; bold and code marks are dropped.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let link = after.find("](").and_then(|mid| Some((mid, mid + 2 + after[mid + 2..].find(')')?)));
        match link {
            Some((mid, end)) => {
                out.push_str(&after[..mid]);
                rest = &after[end + 1..];
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace('`', "")
}

/// `Bug Fixes` → `Bug fixes`.
fn sentence_case(text: &str) -> String {
    capitalised(&text.to_lowercase())
}

/// The first letter in upper case.
fn capitalised(text: &str) -> String {
    match text.char_indices().find(|(_, c)| c.is_alphabetic()) {
        Some((at, c)) => format!("{}{}{}", &text[..at], c.to_uppercase(), &text[at + c.len_utf8()..]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTES: &str = "## [1.0.1](https://github.com/wapenshaw/nuprint/compare/v1.0.0...v1.0.1) (2026-10-29)\n\n\n\
### Bug Fixes\n\n\
* **core:** back up the store from sqlite before upgrading it ([3dc8726](https://github.com/wapenshaw/nuprint/commit/3dc8726))\n\
* recover the log writer and ble device locks after a panic ([273bbe1](https://github.com/wapenshaw/nuprint/commit/273bbe1))\n\n\n\
### Performance\n\n\
* **slint:** append log lines instead of rebuilding the `log` ([75ca51c](https://github.com/wapenshaw/nuprint/commit/75ca51c))\n";

    #[test]
    fn release_please_notes_become_plain_sections() {
        assert_eq!(
            release_notes(NOTES),
            "Bug fixes\n\
             • Back up the store from sqlite before upgrading it\n\
             • Recover the log writer and ble device locks after a panic\n\
             \n\
             Performance\n\
             • Append log lines instead of rebuilding the log"
        );
    }

    #[test]
    fn other_text_keeps_its_words_without_links() {
        assert_eq!(
            release_notes("### ⚠ BREAKING CHANGES\n\nSee [the guide](https://example.com) first.\n- a [half link"),
            "⚠ Breaking changes\nSee the guide first.\n• A [half link"
        );
        assert_eq!(release_notes(""), "");
    }

    #[test]
    fn update_info_points_to_the_release_page() {
        let info = update_info("v1.2.0", "1.0.1", None, true);
        assert_eq!(info.version, "1.2.0");
        assert_eq!(info.release_url, "https://github.com/wapenshaw/nuprint/releases/tag/v1.2.0");
        assert_eq!(info.notes, "");
        assert_eq!(
            latest_release_file("latest.json"),
            "https://github.com/wapenshaw/nuprint/releases/latest/download/latest.json"
        );
    }
}
