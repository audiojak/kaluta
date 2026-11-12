//! What a bug report may quote from the core's log (spec §17.1).
//!
//! The log file is written unscrubbed; a report sends only its newest
//! warnings and errors, scrubbed, and only after the user has read them.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::Core;
use crate::logging::{self, LOG_FILE};

/// How much of the end of the log is read to find the lines.
const TAIL_BYTES: u64 = 512 * 1024;
/// A line longer than this is cut: a report is read by a person.
const MAX_LINE_CHARS: usize = 400;
/// The most a report quotes, whatever was asked for.
const MAX_LINES: usize = 200;

#[uniffi::export]
impl Core {
    /// The newest warnings and errors in the core's log, oldest first, at
    /// most `max_lines`, with addresses, tokens and keys removed and the
    /// home folder written `~`. Empty when there is no log.
    pub fn recent_log(&self, max_lines: u32) -> String {
        let Some(dir) = self.config.log_dir.as_deref() else { return String::new() };
        let Some(text) = tail(&Path::new(dir).join(LOG_FILE), TAIL_BYTES) else { return String::new() };
        let home = std::env::var("HOME").ok();
        excerpt(&text, max_lines as usize, home.as_deref())
    }
}

/// The last `bytes` of a file as text, from the first whole line.
fn tail(path: &Path, bytes: u64) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(bytes);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buffer = Vec::new();
    file.take(bytes).read_to_end(&mut buffer).ok()?;
    let text = String::from_utf8_lossy(&buffer).into_owned();
    if start == 0 {
        return Some(text);
    }
    // The read began mid-line.
    Some(text.split_once('\n').map(|(_, rest)| rest.to_owned()).unwrap_or_default())
}

/// A record's level, as `tracing`'s formatter writes it after the time.
fn is_warning_or_error(line: &str) -> bool {
    let mut words = line.split_whitespace();
    words.next();
    matches!(words.next(), Some("WARN" | "ERROR"))
}

fn excerpt(log: &str, max_lines: usize, home: Option<&str>) -> String {
    let lines: Vec<&str> = log.lines().filter(|line| is_warning_or_error(line)).collect();
    let keep = max_lines.min(MAX_LINES);
    lines[lines.len().saturating_sub(keep)..]
        .iter()
        .map(|line| {
            let line = match home {
                Some(home) if home.len() > 1 => line.replace(home, "~"),
                _ => (*line).to_owned(),
            };
            let scrubbed = logging::scrub(&line);
            if scrubbed.chars().count() > MAX_LINE_CHARS {
                let cut: String = scrubbed.chars().take(MAX_LINE_CHARS).collect();
                format!("{cut}…")
            } else {
                scrubbed
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\
2026-11-12T01:00:00.000000Z  INFO kaluta_core::sync: synced 12 threads
2026-11-12T01:00:01.000000Z  WARN mail_sync::imap: retrying after reset for alex@example.org
2026-11-12T01:00:02.000000Z ERROR kaluta_core::compose: send failed: Bearer ya29.a0AfB_byC-xyz123 expired
2026-11-12T01:00:03.000000Z  INFO kaluta_core::sync: idle
2026-11-12T01:00:04.000000Z  WARN kaluta_core::attachments: cannot read /Users/alex/Library/Application Support/Kaluta/a.bin
";

    #[test]
    fn only_warnings_and_errors_are_quoted_and_they_are_scrubbed() {
        let text = excerpt(LOG, 10, Some("/Users/alex"));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].ends_with("retrying after reset for <email>"), "{}", lines[0]);
        assert!(lines[1].ends_with("send failed: Bearer <token> expired"), "{}", lines[1]);
        assert!(lines[2].ends_with("cannot read ~/Library/Application Support/Kaluta/a.bin"), "{}", lines[2]);
        assert!(!text.contains("alex"));
    }

    #[test]
    fn the_newest_lines_are_kept_and_long_ones_cut() {
        assert!(excerpt(LOG, 1, None).contains("attachments"));
        assert_eq!(excerpt(LOG, 0, None), "");
        assert_eq!(excerpt("", 10, None), "");
        let long = format!("2026-11-12T01:00:00Z ERROR x: {}", "y".repeat(1000));
        assert_eq!(excerpt(&long, 5, None).chars().count(), MAX_LINE_CHARS + 1);
    }

    #[test]
    fn the_tail_starts_at_a_whole_line() {
        let dir = std::env::temp_dir().join(format!("kaluta-feedback-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(LOG_FILE);
        std::fs::write(&path, "first line\nsecond line\nthird line\n").unwrap();
        assert_eq!(tail(&path, 20).unwrap(), "third line\n");
        assert_eq!(tail(&path, 1000).unwrap(), "first line\nsecond line\nthird line\n");
        assert!(tail(&dir.join("missing.log"), 10).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
