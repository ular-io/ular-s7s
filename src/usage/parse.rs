//! Screen parsers for `/usage` (claude, agy) and `/status` (codex), and the
//! reset-time expressions they contain.

use super::{ResetCountdown, UsageSnapshot, UsageWindow};
use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate, NaiveDateTime, NaiveTime};

/// claude `/usage`: Reads `N% used` (calculated as 100-N) and `Resets <absolute time>`
/// lines from the `Current session` and `Current week` blocks.
pub(super) fn parse_claude(text: &str) -> Option<UsageSnapshot> {
    parse_claude_at(text, Local::now().naive_local())
}

pub(super) fn parse_claude_at(text: &str, now: NaiveDateTime) -> Option<UsageSnapshot> {
    let lines: Vec<&str> = text.lines().collect();
    let current = claude_window(&lines, "current session", now);
    // For weekly, prioritize "all models" over model-specific entries (precedes them, but handled defensively against order changes).
    let weekly = claude_window(&lines, "current week (all models)", now)
        .or_else(|| claude_window(&lines, "current week", now));
    snapshot_or_none(current, weekly)
}

/// Searches for the `N% used` gauge and `Resets ...` line in the 3 lines following the label line.
fn claude_window(lines: &[&str], label: &str, now: NaiveDateTime) -> Option<UsageWindow> {
    let idx = lines
        .iter()
        .position(|l| l.trim().to_ascii_lowercase().starts_with(label))?;
    let mut pct = None;
    let mut reset = None;
    for line in lines.iter().skip(idx + 1).take(3) {
        let lower = line.to_ascii_lowercase();
        if pct.is_none() {
            pct = percent_before(&lower, "% used")
                .map(|used| (100.0 - used).clamp(0.0, 100.0).round() as u8);
        }
        if reset.is_none() {
            if let Some(p) = lower.find("resets") {
                reset = parse_reset_spec(&lower[p + "resets".len()..], now);
            }
        }
    }
    Some(UsageWindow {
        pct_left: pct?,
        reset,
    })
}

/// codex `/status`: Reads `N% left (resets <absolute time>)` from `5h limit:` and `Weekly limit:` lines.
pub(super) fn parse_codex(text: &str) -> Option<UsageSnapshot> {
    parse_codex_at(text, Local::now().naive_local())
}

pub(super) fn parse_codex_at(text: &str, now: NaiveDateTime) -> Option<UsageSnapshot> {
    let current = codex_window(text, "5h limit", now);
    let weekly = codex_window(text, "weekly limit", now);
    snapshot_or_none(current, weekly)
}

fn codex_window(text: &str, label: &str, now: NaiveDateTime) -> Option<UsageWindow> {
    let lower = text
        .lines()
        .map(|l| l.to_ascii_lowercase())
        .find(|l| l.contains(label) && l.contains("% left"))?;
    let pct = percent_before(&lower, "% left")?.clamp(0.0, 100.0).round() as u8;
    let reset = lower.find("(resets").and_then(|p| {
        let rest = &lower[p + "(resets".len()..];
        let end = rest.find(')').unwrap_or(rest.len());
        parse_reset_spec(&rest[..end], now)
    });
    Some(UsageWindow {
        pct_left: pct,
        reset,
    })
}

/// agy `/usage`: Divided into blocks of `Weekly Limit` and `Five Hour Limit` by model group
/// (GEMINI / CLAUDE AND GPT). Selects the group based on the active model shown in the bottom status line,
/// then reads the `N%` gauge (remaining) and `Refreshes in ...`.
pub(super) fn parse_agy(text: &str) -> Option<UsageSnapshot> {
    parse_agy_at(text, Local::now().naive_local())
}

pub(super) fn parse_agy_at(text: &str, now: NaiveDateTime) -> Option<UsageSnapshot> {
    let lines: Vec<&str> = text.lines().collect();
    let footer = lines
        .iter()
        .rev()
        .find(|l| !l.trim().is_empty())
        .copied()
        .unwrap_or("");
    // agy 1.4.9 added a separate `GEMINI 4 ARGON` bucket that has only a weekly
    // limit (`Gemini 4 Argon Weekly Limit`); its models also contain "Gemini".
    let (group, weekly_labels): (&str, &[&str]) = if footer.contains("Argon") {
        (
            "GEMINI 4 ARGON",
            &["gemini 4 argon weekly limit", "weekly limit"],
        )
    } else if footer.contains("Gemini") {
        ("GEMINI MODELS", &["weekly limit"])
    } else {
        ("CLAUDE AND GPT MODELS", &["weekly limit"])
    };
    let start = lines.iter().position(|l| l.contains(group)).unwrap_or(0);
    // Group section: from the header to just before the next group header.
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, l)| is_agy_group_header(l))
        .map(|(i, _)| i)
        .unwrap_or(lines.len());
    let section = &lines[start..end];
    let current = agy_window(section, &["five hour limit", "5h limit", "5-hour limit"]);
    let weekly = agy_window(section, weekly_labels);
    let _ = now; // agy only uses relative notation, but signature is kept consistent with other parsers.
    snapshot_or_none(current, weekly)
}

/// A quota group heading such as `GEMINI MODELS` or `GEMINI 4 ARGON`: letters, all
/// uppercase. Gauges, rules, and the `Account:` line never qualify.
fn is_agy_group_header(line: &str) -> bool {
    let t = line.trim();
    t.chars().any(|c| c.is_ascii_alphabetic())
        && t.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == ' ')
}

/// Searches for the `N%` gauge (remaining) and `Refreshes in ...` in the 3 lines following the label line.
/// If `Disabled: ... will fully refresh in 16 hours, 37 minutes.` is shown due to weekly limit depletion,
/// records remaining usage as 0% and parses the refresh time as the countdown.
fn agy_window(lines: &[&str], labels: &[&str]) -> Option<UsageWindow> {
    let idx = lines.iter().position(|l| {
        let t = l.trim().to_ascii_lowercase();
        labels.iter().any(|label| t.starts_with(label))
    })?;
    let mut pct = None;
    let mut reset = None;
    for line in lines.iter().skip(idx + 1).take(3) {
        let lower = line.to_ascii_lowercase();
        if lower.contains("disabled") {
            let reset = lower
                .find("refresh in")
                .and_then(|p| parse_reset_relative(&lower[p + "refresh in".len()..]));
            return Some(UsageWindow { pct_left: 0, reset });
        }
        if pct.is_none() {
            pct = first_percent(&lower).map(|left| left.clamp(0.0, 100.0).round() as u8);
        }
        if reset.is_none() {
            if let Some(p) = lower.find("refreshes in") {
                reset = parse_reset_relative(&lower[p + "refreshes in".len()..]);
            }
        }
    }
    Some(UsageWindow {
        pct_left: pct?,
        reset,
    })
}

fn snapshot_or_none(
    current: Option<UsageWindow>,
    weekly: Option<UsageWindow>,
) -> Option<UsageSnapshot> {
    if current.is_none() && weekly.is_none() {
        None
    } else {
        Some(UsageSnapshot { current, weekly })
    }
}

/// Converts reset time expressions appearing after 'resets' or 'refreshes' into countdowns relative to 'now'.
/// Supported formats (assumes lowercase inputs, based on verified screens):
/// - Relative: `in 16h 51m`, `in 2 hours`
/// - Absolute time: `5am`, `5:30pm`, `04:45` -> next occurrence
/// - Absolute day of week (+ time): `mon 14:30` -> next occurrence of that weekday
/// - Absolute date (+ time): `jul 10 at 5pm`, `mon jul 10`, `17:33 on 15 jul` -> corresponding date this/next year
///   Timezone details in parentheses (e.g. `(asia/seoul)`) are ignored and parsed in local time.
pub(super) fn parse_reset_spec(raw: &str, now: NaiveDateTime) -> Option<ResetCountdown> {
    let cleaned = raw.split('(').next().unwrap_or("");
    let s = cleaned
        .trim()
        .trim_matches(|c: char| matches!(c, ')' | ',' | '.'))
        .to_ascii_lowercase();
    if s.is_empty() {
        return None;
    }
    if let Some(rel) = s.strip_prefix("in ") {
        return parse_reset_relative(rel);
    }

    let mut weekday: Option<chrono::Weekday> = None;
    let mut month_day: Option<(u32, u32)> = None;
    let mut time: Option<NaiveTime> = None;
    let tokens: Vec<&str> = s
        .split_whitespace()
        .map(|tok| tok.trim_matches(|c: char| matches!(c, '(' | ')' | ',' | '.')))
        .filter(|tok| !tok.is_empty() && *tok != "at" && *tok != "on")
        .collect();
    for (idx, tok) in tokens.iter().enumerate() {
        if weekday.is_none() {
            if let Some(wd) = parse_weekday(tok) {
                weekday = Some(wd);
                continue;
            }
        }
        if month_day.is_none() {
            if let Some(month) = parse_month(tok) {
                let day = tokens.get(idx + 1).and_then(|d| parse_day_token(d));
                if let Some(day) = day {
                    month_day = Some((month, day));
                    continue;
                }
            }
            if let Some(day) = parse_day_token(tok) {
                if let Some(month) = tokens.get(idx + 1).and_then(|m| parse_month(m)) {
                    month_day = Some((month, day));
                    continue;
                }
            }
        }
        if time.is_none() {
            if let Some(t) = parse_time_token(tok) {
                time = Some(t);
                continue;
            }
        }
    }

    let target = if let Some((month, day)) = month_day {
        let t = time.unwrap_or(NaiveTime::MIN);
        let mut dt = NaiveDate::from_ymd_opt(now.year(), month, day)?.and_time(t);
        if dt <= now {
            dt = NaiveDate::from_ymd_opt(now.year() + 1, month, day)?.and_time(t);
        }
        dt
    } else if let Some(wd) = weekday {
        let t = time.unwrap_or(NaiveTime::MIN);
        let ahead = (wd.num_days_from_monday() as i64
            - now.weekday().num_days_from_monday() as i64)
            .rem_euclid(7);
        let mut dt = (now.date() + ChronoDuration::days(ahead)).and_time(t);
        if dt <= now {
            dt += ChronoDuration::days(7);
        }
        dt
    } else if let Some(t) = time {
        let mut dt = now.date().and_time(t);
        if dt <= now {
            dt += ChronoDuration::days(1);
        }
        dt
    } else {
        // If not an absolute time format, retry as a relative notation without a prefix (e.g. "2h 2m").
        return parse_reset_relative(&s);
    };
    countdown_between(now, target)
}

/// Parses relative notation (e.g., `1d 14h`, `16h 51m`, `2 hours`) into a countdown.
pub(super) fn parse_reset_relative(raw: &str) -> Option<ResetCountdown> {
    let mut days = 0u16;
    let mut hours = 0u16;
    let mut minutes = 0u16;
    let mut any = false;
    let mut pending: Option<u16> = None;
    for tok in raw.to_ascii_lowercase().split_whitespace() {
        let tok = tok.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        let digits: String = tok.chars().take_while(|c| c.is_ascii_digit()).collect();
        let unit = &tok[digits.len()..];
        let n = if digits.is_empty() {
            pending.take()
        } else {
            digits.parse::<u16>().ok()
        };
        let Some(n) = n else { continue };
        match unit {
            "" => pending = Some(n),
            u if u.starts_with('d') => {
                days = n;
                any = true;
            }
            u if u.starts_with('h') => {
                hours = n;
                any = true;
            }
            u if u.starts_with('m') => {
                minutes = n;
                any = true;
            }
            _ => {}
        }
    }
    if !any || (days == 0 && hours == 0 && minutes == 0) {
        None
    } else {
        let total_minutes = (days as u32 * 24 * 60) + (hours as u32 * 60) + minutes as u32;
        Some(ResetCountdown {
            days: (total_minutes / (24 * 60)) as u16,
            hours: ((total_minutes / 60) % 24) as u8,
            minutes: (total_minutes % 60) as u8,
        })
    }
}

fn countdown_between(now: NaiveDateTime, target: NaiveDateTime) -> Option<ResetCountdown> {
    let mins = (target - now).num_minutes();
    if mins <= 0 {
        return None;
    }
    Some(ResetCountdown {
        days: (mins / (24 * 60)) as u16,
        hours: ((mins / 60) % 24) as u8,
        minutes: (mins % 60) as u8,
    })
}

/// Parses `5am`, `5:30pm`, or `04:45` into a time object. Single numbers (like `10`) are not
/// considered valid times to avoid confusion with calendar days, so a `:` or am/pm suffix must be present.
fn parse_time_token(tok: &str) -> Option<NaiveTime> {
    let (body, meridiem) = if let Some(b) = tok.strip_suffix("am") {
        (b, Some(false))
    } else if let Some(b) = tok.strip_suffix("pm") {
        (b, Some(true))
    } else {
        (tok, None)
    };
    let (h_str, m_str) = match body.split_once(':') {
        Some((h, m)) => (h, m),
        None => (body, "0"),
    };
    if h_str.is_empty()
        || !h_str.chars().all(|c| c.is_ascii_digit())
        || !m_str.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let mut hours: u32 = h_str.parse().ok()?;
    let minutes: u32 = m_str.parse().ok()?;
    match meridiem {
        Some(pm) => {
            if !(1..=12).contains(&hours) {
                return None;
            }
            if hours == 12 {
                hours = 0;
            }
            if pm {
                hours += 12;
            }
        }
        None => {
            if !tok.contains(':') {
                return None;
            }
        }
    }
    NaiveTime::from_hms_opt(hours, minutes, 0)
}

fn parse_weekday(tok: &str) -> Option<chrono::Weekday> {
    use chrono::Weekday::*;
    match tok.get(..3)? {
        "mon" => Some(Mon),
        "tue" => Some(Tue),
        "wed" => Some(Wed),
        "thu" => Some(Thu),
        "fri" => Some(Fri),
        "sat" => Some(Sat),
        "sun" => Some(Sun),
        _ => None,
    }
}

fn parse_month(tok: &str) -> Option<u32> {
    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let prefix = tok.get(..3)?;
    months
        .iter()
        .position(|m| *m == prefix)
        .map(|i| i as u32 + 1)
}

fn parse_day_token(tok: &str) -> Option<u32> {
    let digits: String = tok.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let suffix = &tok[digits.len()..];
    if !matches!(suffix, "" | "st" | "nd" | "rd" | "th") {
        return None;
    }
    let day: u32 = digits.parse().ok()?;
    (1..=31).contains(&day).then_some(day)
}

fn percent_before(text: &str, suffix: &str) -> Option<f64> {
    text.find(suffix).and_then(|pos| number_before(text, pos))
}

fn first_percent(text: &str) -> Option<f64> {
    text.find('%').and_then(|pos| number_before(text, pos))
}

/// Parses a decimal number (including floating points) immediately preceding the `end` byte offset in the text.
fn number_before(text: &str, end: usize) -> Option<f64> {
    let bytes = text.as_bytes();
    let mut start = end;
    while start > 0 {
        let c = bytes[start - 1] as char;
        if c.is_ascii_digit() || c == '.' {
            start -= 1;
        } else {
            break;
        }
    }
    if start == end {
        return None;
    }
    text[start..end].parse::<f64>().ok()
}
