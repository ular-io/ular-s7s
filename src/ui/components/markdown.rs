//! Markdown body rendering: turns stored Markdown (assistant answers) into
//! pre-wrapped `Line`s (one `Line` = one row) with the markers removed and the
//! theme's `md_*` roles applied. pulldown-cmark only parses; layout is ours so
//! wrapping, prefixes, and table widths follow the app's own width rules.
//! Copy, search, and handoff keep using the raw text — this is display only.

use crate::theme::Theme;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Renders `text` for a pane `width` columns wide.
pub(crate) fn render_markdown(text: &str, width: usize, th: &Theme) -> Vec<Line<'static>> {
    let text = text.replace('\t', "    ");
    let mut r = Renderer::new(width.max(1), th);
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(&text, opts) {
        r.event(event);
    }
    r.flush();
    while r.out.last().is_some_and(|l| l.width() == 0) {
        r.out.pop();
    }
    r.out
}

/// One grapheme with its display width and style.
#[derive(Clone)]
struct Cell {
    g: String,
    w: usize,
    st: Style,
}

type Cells = Vec<Cell>;

/// Forced line break inside a run of cells (`<br>`); zero width, never drawn.
const BREAK: &str = "\n";
type Spans = Vec<Span<'static>>;

fn cells(spans: &[Span<'static>]) -> Cells {
    spans
        .iter()
        .flat_map(|s| {
            s.content.graphemes(true).map(|g| Cell {
                g: g.to_string(),
                w: if g == BREAK { 0 } else { g.width() },
                st: s.style,
            })
        })
        .collect()
}

fn width_of(cells: &[Cell]) -> usize {
    cells.iter().map(|c| c.w).sum()
}

fn spans_of(cells: &[Cell]) -> Spans {
    let mut out: Spans = Vec::new();
    for c in cells {
        match out.last_mut() {
            Some(last) if last.style == c.st => last.content.to_mut().push_str(&c.g),
            _ => out.push(Span::styled(c.g.clone(), c.st)),
        }
    }
    out
}

fn spans_width(spans: &[Span<'static>]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// Word-preferring wrap by display width; a word longer than the line breaks
/// at a grapheme boundary. Wrapped lines drop the space they broke at.
fn wrap(cells: &[Cell], width: usize) -> Vec<Cells> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line: Cells = Vec::new();
    let mut w = 0;
    for c in cells {
        if c.g == BREAK {
            lines.push(std::mem::take(&mut line));
            w = 0;
            continue;
        }
        if w + c.w > width && !line.is_empty() {
            match line.iter().rposition(|c| c.g == " ").filter(|&i| i > 0) {
                Some(space) => {
                    let rest = line.split_off(space + 1);
                    line.pop();
                    lines.push(std::mem::replace(&mut line, rest));
                }
                None => lines.push(std::mem::take(&mut line)),
            }
            w = width_of(&line);
        }
        if c.g == " " && line.is_empty() && !lines.is_empty() {
            continue;
        }
        w += c.w;
        line.push(c.clone());
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

fn is_br(html: &str) -> bool {
    matches!(
        html.trim().to_ascii_lowercase().as_str(),
        "<br>" | "<br/>" | "<br />"
    )
}

struct Table {
    aligns: Vec<Alignment>,
    rows: Vec<Vec<Spans>>,
    row: Vec<Spans>,
    head_rows: usize,
}

struct Renderer<'t> {
    th: &'t Theme,
    width: usize,
    out: Vec<Line<'static>>,
    cur: Spans,
    styles: Vec<Style>,
    lists: Vec<Option<u64>>,
    quote: usize,
    bullet: Option<String>,
    bullet_used: bool,
    heading_level: usize,
    code: Option<(String, String)>,
    table: Option<Table>,
    cell: Option<Spans>,
}

impl<'t> Renderer<'t> {
    fn new(width: usize, th: &'t Theme) -> Self {
        Self {
            th,
            width,
            out: Vec::new(),
            cur: Vec::new(),
            styles: Vec::new(),
            lists: Vec::new(),
            quote: 0,
            bullet: None,
            bullet_used: false,
            heading_level: 0,
            code: None,
            table: None,
            cell: None,
        }
    }

    fn style(&self) -> Style {
        self.styles.last().copied().unwrap_or_default()
    }

    fn push_style(&mut self, st: Style) {
        self.styles.push(self.style().patch(st));
    }

    fn border(&self) -> Style {
        Style::default().fg(self.th.md.border)
    }

    /// Quote bars, list indentation, and the item marker (first line only).
    fn prefix(&self, first: bool) -> Spans {
        let mut p = vec![Span::styled("▎ ", self.border()); self.quote];
        if let Some(depth) = self.lists.len().checked_sub(1) {
            p.push(Span::raw("  ".repeat(depth)));
            p.push(match (&self.bullet, first) {
                (Some(b), true) => Span::styled(b.clone(), Style::default().fg(self.th.md.marker)),
                (Some(b), false) => Span::raw(" ".repeat(b.width())),
                (None, _) => Span::raw("  "),
            });
        }
        p
    }

    fn emit(&mut self, cells: &[Cell], first: Spans, rest: Spans) {
        let budget = self.width.saturating_sub(spans_width(&first));
        for (i, l) in wrap(cells, budget).into_iter().enumerate() {
            let mut spans = if i == 0 { first.clone() } else { rest.clone() };
            spans.extend(spans_of(&l));
            self.out.push(Line::from(spans));
        }
    }

    fn flush(&mut self) {
        if self.cur.is_empty() {
            return;
        }
        let cells = cells(&std::mem::take(&mut self.cur));
        let (first, rest) = (self.prefix(!self.bullet_used), self.prefix(false));
        self.bullet_used = true;
        self.emit(&cells, first, rest);
    }

    fn blank(&mut self) {
        if self.out.last().is_some_and(|l| l.width() > 0) {
            self.out.push(Line::default());
        }
    }

    /// Paragraph-like block end: a blank line only between top-level blocks.
    fn end_block(&mut self) {
        self.flush();
        if self.lists.is_empty() && self.quote == 0 {
            self.blank();
        }
    }

    fn text(&mut self, t: &str, st: Style) {
        if let Some((_, code)) = &mut self.code {
            code.push_str(t);
            return;
        }
        let span = Span::styled(t.to_string(), st);
        match &mut self.cell {
            Some(cell) => cell.push(span),
            None => self.cur.push(span),
        }
    }

    /// `<br>` and hard breaks: a new row inside a table cell, a new line elsewhere.
    fn line_break(&mut self) {
        match &mut self.cell {
            Some(cell) => cell.push(Span::raw(BREAK)),
            None => self.flush(),
        }
    }

    /// Full-width bar; text indented 1 column at H1 and 2 more per level.
    fn heading_bar(&mut self) {
        let md = self.th.md;
        let bar = Style::default().fg(md.heading_fg).bg(md.heading_bg);
        let mut cells = cells(&std::mem::take(&mut self.cur));
        for c in &mut cells {
            c.st = c.st.patch(bar);
        }
        let indent = 1 + 2 * self.heading_level.saturating_sub(1);
        for l in wrap(&cells, self.width.saturating_sub(indent + 1)) {
            let pad = self.width.saturating_sub(indent + width_of(&l));
            let mut spans = vec![Span::styled(" ".repeat(indent), bar)];
            spans.extend(spans_of(&l));
            spans.push(Span::styled(" ".repeat(pad), bar));
            self.out.push(Line::from(spans));
        }
    }

    fn code_block(&mut self, lang: &str, code: &str) {
        let (base, border) = (self.prefix(false), self.border());
        let frame = |s: String| [base.clone(), vec![Span::styled(s, border)]].concat();
        self.out.push(Line::from(frame(format!("╭─ {lang}"))));
        let gutter = frame("│ ".to_string());
        let body = Style::default().fg(self.th.md.code_block_fg);
        for l in code.trim_end_matches('\n').lines() {
            let cells = cells(&[Span::styled(l.to_string(), body)]);
            self.emit(&cells, gutter.clone(), gutter.clone());
        }
        self.out.push(Line::from(frame("╰─".to_string())));
    }

    fn finish_row(&mut self) {
        if let Some(t) = &mut self.table {
            t.rows.push(std::mem::take(&mut t.row));
        }
    }

    fn start(&mut self, tag: Tag) {
        let md = self.th.md;
        match tag {
            Tag::Heading { level, .. } => {
                self.flush();
                self.heading_level = level as usize;
                self.push_style(Style::default().add_modifier(Modifier::BOLD));
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quote += 1;
                self.push_style(Style::default().fg(md.quote));
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let nested = self.lists.len() > 1;
                self.bullet = Some(match self.lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        format!("{}. ", *n - 1)
                    }
                    _ if nested => "◦ ".into(),
                    _ => "• ".into(),
                });
                self.bullet_used = false;
            }
            Tag::Table(aligns) => {
                self.flush();
                self.table = Some(Table {
                    aligns,
                    rows: vec![],
                    row: vec![],
                    head_rows: 0,
                });
            }
            Tag::TableHead => self.push_style(
                Style::default()
                    .fg(md.table_head)
                    .add_modifier(Modifier::BOLD),
            ),
            Tag::TableCell => self.cell = Some(Vec::new()),
            Tag::Emphasis => self.push_style(Style::default().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(Style::default().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => {
                self.push_style(self.th.soft_dim().add_modifier(Modifier::CROSSED_OUT))
            }
            Tag::Link { .. } => self.push_style(
                Style::default()
                    .fg(md.link)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::HtmlBlock => self.end_block(),
            TagEnd::Heading(_) => {
                self.heading_bar();
                self.styles.pop();
                self.blank();
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote = self.quote.saturating_sub(1);
                self.styles.pop();
                self.blank();
            }
            TagEnd::CodeBlock => {
                let (lang, code) = self.code.take().unwrap_or_default();
                self.code_block(&lang, &code);
                self.blank();
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Item => {
                self.flush();
                self.bullet = None;
            }
            TagEnd::TableHead => {
                self.styles.pop();
                self.finish_row();
                if let Some(t) = &mut self.table {
                    t.head_rows = t.rows.len();
                }
            }
            TagEnd::TableRow => self.finish_row(),
            TagEnd::TableCell => {
                let cell = self.cell.take().unwrap_or_default();
                if let Some(t) = &mut self.table {
                    t.row.push(cell);
                }
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.table(t);
                }
                self.blank();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                self.styles.pop();
            }
            _ => {}
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t, self.style()),
            Event::Code(t) => {
                let md = self.th.md;
                let st = self
                    .style()
                    .patch(Style::default().fg(md.code_fg).bg(md.code_bg));
                self.text(&format!("\u{a0}{t}\u{a0}"), st);
            }
            Event::SoftBreak => self.text(" ", self.style()),
            Event::HardBreak => self.line_break(),
            Event::Rule => {
                self.flush();
                let rule = "─".repeat(self.width);
                self.out.push(Line::from(Span::styled(rule, self.border())));
                self.blank();
            }
            Event::TaskListMarker(done) => {
                self.bullet = Some(if done { "[✓] " } else { "[ ] " }.into());
            }
            Event::Html(t) | Event::InlineHtml(t) if is_br(&t) => self.line_break(),
            Event::Html(t) | Event::InlineHtml(t) => self.text(&t, self.th.soft_dim()),
            _ => {}
        }
    }
}

mod table;

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
