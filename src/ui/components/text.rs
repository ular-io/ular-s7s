//! Width-aware string helpers. Terminal cells are one or two columns wide, so
//! layout must measure display width (via `unicode-width`) rather than `char`
//! or byte counts; `format!` width specifiers count chars and misalign CJK and
//! other double-width text.

use unicode_width::UnicodeWidthStr;

/// Truncates a string to `max_w` display columns, appending `…` when clipped.
pub(crate) fn truncate_w(s: &str, max_w: usize) -> String {
    truncate_w_with_ellipsis(s, max_w, "…")
}

/// Right-pads a string with spaces to `w` display columns (no-op if already wider).
pub(crate) fn pad_w(s: &str, w: usize) -> String {
    let cur = s.width();
    if cur >= w {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(w - cur))
    }
}

/// Truncates to `max_w` display columns using a custom ellipsis marker.
pub(crate) fn truncate_w_with_ellipsis(s: &str, max_w: usize, ellipsis: &str) -> String {
    if s.width() <= max_w {
        return s.to_string();
    }
    if max_w == 0 {
        return String::new();
    }

    let ellipsis_w = ellipsis.width();
    if max_w <= ellipsis_w {
        return ellipsis
            .chars()
            .scan(0usize, |w, ch| {
                let cw = UnicodeWidthStr::width(ch.to_string().as_str());
                if *w + cw > max_w {
                    return None;
                }
                *w += cw;
                Some(ch)
            })
            .collect();
    }

    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthStr::width(ch.to_string().as_str());
        if w + cw + ellipsis_w > max_w {
            out.push_str(ellipsis);
            break;
        }
        out.push(ch);
        w += cw;
    }
    out
}

/// Wraps text to a `max_w` display-column limit (no ellipsis).
pub(crate) fn wrap_w(s: &str, max_w: usize) -> Vec<String> {
    // Tab stops (space expansion width). unicode-width counts `\t` as 0 width, but terminals expand it.
    // This discrepancy causes text to overflow past frames. Expands tabs to spaces before calculation to match widths.
    const TAB_STOP: usize = 4;
    if max_w == 0 {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_w = 0usize;

    for ch in s.chars() {
        // Tab: fills with space up to next tab stop (wraps if exceeding max width).
        if ch == '\t' {
            for _ in 0..(TAB_STOP - (current_w % TAB_STOP)) {
                if current_w + 1 > max_w {
                    lines.push(std::mem::take(&mut current));
                    current_w = 0;
                }
                current.push(' ');
                current_w += 1;
            }
            continue;
        }
        let cw = UnicodeWidthStr::width(ch.to_string().as_str());
        if current_w > 0 && current_w + cw > max_w {
            lines.push(current);
            current = String::new();
            current_w = 0;
        }
        if cw > max_w {
            // Extremely rare case where a single character width exceeds max width; handle safely on its own line.
            if !current.is_empty() {
                lines.push(current);
                current = String::new();
                current_w = 0;
            }
            lines.push(ch.to_string());
            continue;
        }
        current.push(ch);
        current_w += cw;
    }

    if !current.is_empty() {
        lines.push(current);
    }

    lines
}
