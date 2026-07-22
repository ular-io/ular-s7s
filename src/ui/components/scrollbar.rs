//! Persistent vertical scrollbar drawn over a panel's right border. Reused by
//! the session/profile tables, the preview, and scrolling modals.

use crate::theme::Theme;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    Frame,
};

/// Renders scrollbar tracks over the right vertical border of focused panels.
/// Since borders cannot load text via standard title APIs, this directly overwrites
/// target buffer cells after the widget renders.
///
/// - Places `↑` directly under the top border cell, and `↓` directly above the bottom border cell.
/// - Draws a solid block (`█`) representing the thumb position and ratio inside the track.
///   Thumb height scales to `viewport / total`; position correlates with `offset / max_offset`.
///   If scroll is not required (`total <= viewport`), renders arrows only without the solid block.
pub(crate) fn draw_vscrollbar(
    f: &mut Frame,
    area: Rect,
    focused: bool,
    offset: usize,
    total: usize,
    viewport: usize,
    th: &Theme,
) {
    if !focused || area.width == 0 || area.height < 4 {
        return;
    }
    let style = Style::default().fg(th.accent).add_modifier(Modifier::BOLD);
    let x = area.x + area.width - 1;
    let top_y = area.y + 1;
    let bottom_y = area.y + area.height - 2;
    {
        let buf = f.buffer_mut();
        buf[(x, top_y)].set_symbol("↑").set_style(style);
        buf[(x, bottom_y)].set_symbol("↓").set_style(style);
    }

    // Draws thumb inside track (between the bottom of top arrow and top of bottom arrow).
    if area.height < 5 || total <= viewport || viewport == 0 {
        return;
    }
    let track_start = top_y + 1;
    let track_len = (bottom_y - 1 - track_start + 1) as usize; // = height - 4
    let max_offset = total - viewport;
    let offset = offset.min(max_offset);
    let thumb_len = (track_len * viewport / total).max(1).min(track_len);
    // Rounding alignment: sticks to the top of track if at maximum, bottom if at minimum.
    let thumb_off = ((track_len - thumb_len) * offset + max_offset / 2) / max_offset;
    let buf = f.buffer_mut();
    for i in 0..thumb_len {
        let y = track_start + (thumb_off + i) as u16;
        buf[(x, y)].set_symbol("█").set_style(style);
    }
}
