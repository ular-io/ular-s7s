//! Core data models.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Supported types of AI CLI agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Agent {
    Claude,
    Codex,
    Antigravity,
}

impl Agent {
    /// Lowercase name for display and filter matching.
    pub fn key(&self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Antigravity => "antigravity",
            Agent::Codex => "codex",
        }
    }

    /// Label for status bar and modal displays.
    pub fn label(&self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Antigravity => "antigravity",
            Agent::Codex => "codex",
        }
    }

    /// List of all agents (used for modals, initial filter values, etc.).
    pub fn all() -> [Agent; 3] {
        [Agent::Claude, Agent::Codex, Agent::Antigravity]
    }
}

/// A single conversation session. The final unit parsed and stored in the cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Agent type.
    pub agent: Agent,
    /// Parent profile ID. The value stored in the cache is not trusted and is re-assigned on scan.
    #[serde(default)]
    pub profile_id: String,
    /// Session/Conversation ID used for resuming.
    pub id: String,
    /// Original session file/DB path. Used for the deletion feature.
    #[serde(default)]
    pub source_path: Option<PathBuf>,
    /// Working directory where the conversation took place (absolute path).
    pub cwd: PathBuf,
    /// Folder name (basename of cwd). Used for folder filtering and table columns.
    pub folder: String,
    /// Last modified time (epoch milliseconds). Used for sorting and display.
    pub mtime_ms: i64,
    /// Creation time (file birth time, epoch milliseconds). Used for display.
    #[serde(default)]
    pub ctime_ms: i64,
    /// Source file size in bytes. Re-derived from fs metadata on every scan
    /// (same pattern as ctime_ms), so no cache version bump is required.
    #[serde(default)]
    pub size_bytes: u64,
    /// Cleaned user turns (questions). Stored in raw NFC format.
    pub user_turns: Vec<String>,
    /// Precomputed lowercase NFC text for searching (combines all user turns, title, and folder name).
    #[serde(default)]
    pub search_blob: String,
    /// Automatic or explicit title candidate.
    #[serde(default)]
    pub title_hint: Option<String>,
    /// Indicates whether the title has been explicitly fixed.
    #[serde(default)]
    pub title_fixed: bool,
}

impl Session {
    /// Title for the table's "Title" column.
    pub fn title(&self) -> String {
        crate::title::resolve(
            &self.user_turns,
            self.title_hint.as_deref(),
            self.title_fixed,
        )
    }

    /// Converts mtime to local date string (YYYY-MM-DD). Does not include time.
    pub fn date_str(&self) -> String {
        let secs = self.mtime_ms.div_euclid(1000);
        let local = secs + local_utc_offset_secs(secs);
        let (y, mo, d, _, _) = civil_from_epoch(local);
        format!("{:04}-{:02}-{:02}", y, mo, d)
    }

    /// Converts creation time to local date/time string (YYYY-MM-DD HH:MM).
    /// Falls back to mtime if ctime is missing (e.g. older cache versions).
    pub fn created_str(&self) -> String {
        let ms = if self.ctime_ms > 0 {
            self.ctime_ms
        } else {
            self.mtime_ms
        };
        format_epoch_ms(ms)
    }

    /// Converts last modified time to local date/time string (YYYY-MM-DD HH:MM).
    pub fn updated_str(&self) -> String {
        format_epoch_ms(self.mtime_ms)
    }

    /// Human-readable source file size for the table's SIZE column (KB/MB units only).
    pub fn size_str(&self) -> String {
        human_size_kb_mb(self.size_bytes)
    }
}

/// Bytes -> "NK" (minimum 1K for non-empty files) or "N.NM"/"NM".
/// Switches to MB once the rounded KB value reaches 1000 (e.g. 1005K renders as 1.0M),
/// so the KB display never exceeds 999K. Max width is 5 cells (e.g. "9.9M", "2048M").
pub fn human_size_kb_mb(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes == 0 {
        return "0K".to_string();
    }
    let kb = (bytes as f64 / 1024.0).round().max(1.0) as u64;
    if kb < 1000 {
        return format!("{}K", kb);
    }
    let mb = bytes as f64 / MIB as f64;
    if mb < 10.0 {
        format!("{:.1}M", mb)
    } else {
        format!("{:.0}M", mb)
    }
}

/// Epoch milliseconds -> Local time "YYYY-MM-DD HH:MM".
fn format_epoch_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let local = secs + local_utc_offset_secs(secs);
    let (y, mo, d, h, mi) = civil_from_epoch(local);
    format!("{:04}-{:02}-{:02} {:02}:{:02}", y, mo, d, h, mi)
}

/// Collapses multi-line text into a single line by replacing newlines with spaces.
pub fn one_line(s: &str) -> String {
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
}

/// Unix seconds -> (year, month, day, hour, minute). Implements Howard Hinnant's civil_from_days algorithm.
fn civil_from_epoch(secs: i64) -> (i64, u32, u32, u32, u32) {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let h = (rem / 3600) as u32;
    let mi = ((rem % 3600) / 60) as u32;

    // days since 1970-01-01 → civil date
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d, h, mi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_size_uses_kb_below_1000k() {
        assert_eq!(human_size_kb_mb(0), "0K");
        assert_eq!(human_size_kb_mb(1), "1K"); // non-empty files never show 0K
        assert_eq!(human_size_kb_mb(512 * 1024), "512K");
        assert_eq!(human_size_kb_mb(999 * 1024), "999K");
    }

    #[test]
    fn human_size_uses_mb_from_1000k() {
        // 1000K..1MiB never renders a 4-digit KB value (1005K -> 1.0M).
        assert_eq!(human_size_kb_mb(1000 * 1024), "1.0M");
        assert_eq!(human_size_kb_mb(1005 * 1024), "1.0M");
        assert_eq!(human_size_kb_mb(1024 * 1024 - 1), "1.0M");
        assert_eq!(human_size_kb_mb(1024 * 1024), "1.0M");
        assert_eq!(human_size_kb_mb(9 * 1024 * 1024 + 512 * 1024), "9.5M");
        assert_eq!(human_size_kb_mb(38 * 1024 * 1024), "38M");
        // GB-scale still renders in MB (units are capped at KB/MB by spec).
        assert_eq!(human_size_kb_mb(2 * 1024 * 1024 * 1024), "2048M");
    }

    #[test]
    fn human_size_max_width_is_five_cells() {
        for b in [
            0,
            1,
            1023,
            1024,
            1024 * 1024 - 1,
            1024 * 1024,
            u32::MAX as u64,
        ] {
            assert!(human_size_kb_mb(b).len() <= 5, "too wide for {b}");
        }
    }
}

/// System local UTC offset in seconds. Cached after querying `date +%z` once.
fn local_utc_offset_secs(_secs: i64) -> i64 {
    use std::sync::OnceLock;
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        // E.g. "+0900" format
        if let Ok(out) = std::process::Command::new("date").arg("+%z").output() {
            if let Ok(s) = String::from_utf8(out.stdout) {
                let s = s.trim();
                if s.len() == 5 {
                    let sign = if s.starts_with('-') { -1 } else { 1 };
                    if let (Ok(hh), Ok(mm)) = (s[1..3].parse::<i64>(), s[3..5].parse::<i64>()) {
                        return sign * (hh * 3600 + mm * 60);
                    }
                }
            }
        }
        0
    })
}
