//! Table layout for the Markdown renderer: column widths, cell wrapping, borders.

use super::{cells, spans_of, spans_width, width_of, wrap, Cells, Renderer, Table, BREAK};
use pulldown_cmark::Alignment;
use ratatui::text::{Line, Span};

impl Renderer<'_> {
    pub(super) fn table(&mut self, t: Table) {
        let n = t.rows.iter().map(Vec::len).max().unwrap_or(0);
        if n == 0 {
            return;
        }
        let grid: Vec<Vec<Cells>> = t
            .rows
            .iter()
            .map(|r| r.iter().map(|c| cells(c)).collect())
            .collect();
        let mut w: Vec<usize> = (0..n)
            .map(|i| {
                grid.iter()
                    .filter_map(|r| r.get(i))
                    .filter_map(|c| c.split(|x| x.g == BREAK).map(width_of).max())
                    .max()
                    .unwrap_or(0)
                    .max(1)
            })
            .collect();
        let base = self.prefix(false);
        let budget = self.width.saturating_sub(spans_width(&base) + 3 * n + 1);
        while w.iter().sum::<usize>() > budget {
            let (i, &widest) = w
                .iter()
                .enumerate()
                .max_by_key(|(_, v)| **v)
                .expect("n > 0");
            if widest <= 4 {
                break;
            }
            w[i] -= 1;
        }
        let border = self.border();
        let rule = |l: &str, m: &str, r: &str| {
            let mut s = l.to_string();
            for (i, cw) in w.iter().enumerate() {
                s += &"─".repeat(cw + 2);
                s += if i + 1 == n { r } else { m };
            }
            Line::from([base.clone(), vec![Span::styled(s, border)]].concat())
        };
        self.out.push(rule("┌", "┬", "┐"));
        for (ri, row) in grid.iter().enumerate() {
            let parts: Vec<Vec<Cells>> = (0..n)
                .map(|i| wrap(row.get(i).map_or(&[][..], Vec::as_slice), w[i]))
                .collect();
            let height = parts.iter().map(Vec::len).max().unwrap_or(1);
            for k in 0..height {
                let mut spans = base.clone();
                spans.push(Span::styled("│", border));
                for (i, part) in parts.iter().enumerate() {
                    let part = part.get(k).map_or(&[][..], Vec::as_slice);
                    let pad = w[i].saturating_sub(width_of(part));
                    let (l, r) = match t.aligns.get(i) {
                        Some(Alignment::Right) => (pad, 0),
                        Some(Alignment::Center) => (pad / 2, pad - pad / 2),
                        _ => (0, pad),
                    };
                    spans.push(Span::raw(" ".repeat(l + 1)));
                    spans.extend(spans_of(part));
                    spans.push(Span::raw(" ".repeat(r + 1)));
                    spans.push(Span::styled("│", border));
                }
                self.out.push(Line::from(spans));
            }
            if ri + 1 == t.head_rows {
                self.out.push(rule("├", "┼", "┤"));
            }
        }
        self.out.push(rule("└", "┴", "┘"));
    }
}
