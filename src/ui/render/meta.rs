//! Session metadata grids, context-source blocks, agent tags, and preview turn
//! omission shared by the Session and Detail screens.

use crate::theme::Theme;
use crate::ui::components::text::truncate_w_with_ellipsis;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

/// Shared helper for preview/detail left panels: constructs 5 session metadata rows (Project, Name, Created, Modified, ID).
/// `dimmed` renders the accent-colored values in soft-dim too (whole panel unfocused).
pub(crate) fn session_meta_lines(
    s: &crate::model::Session,
    marks: crate::ui::bookmarks::TitleMarks,
    show_legend: bool,
    inner_w: usize,
    th: &Theme,
    dimmed: bool,
) -> Vec<Line<'static>> {
    let (tag, _) = agent_tag(s.agent, th);
    meta_grid(
        MetaGrid {
            heading: "Session",
            heading_hint: None,
            folder: crate::scratch::folder_label(&s.cwd, &s.folder),
            full_path: &s.cwd.to_string_lossy(),
            title: &crate::ui::bookmarks::display_title(s, marks),
            legend: if show_legend {
                marks.legend()
            } else {
                Vec::new()
            },
            extra_rows: &[
                ("Created at: ", s.created_str()),
                ("Updated at: ", s.updated_str()),
            ],
            id_value: Some(format!("[{}] {}", tag.trim(), s.id)),
        },
        inner_w,
        th,
        dimmed,
    )
}

/// Minimum blank columns kept between a grid heading and its right-aligned hotkey
/// hint. Below this the hint is dropped rather than crowding the heading.
const HEADING_HINT_MIN_GAP: usize = 2;

/// Hotkey hint on the `● Context Source` heading, in the same `<key>` notation the
/// header and `?` help use. Bound in `ui/context_jump.rs`.
const CONTEXT_SOURCE_JUMP_HINT: &str = "<ctrl+o>";

/// Renders the `● Context Source` block shown above `Q1` for a session launched
/// via "New Session with Context". `resolved` is the source session found in the
/// scanned set (supplies Project/Name); when `None` (source deleted or from an
/// unscanned profile) only the Id line is shown with a `(source unavailable)`
/// marker and the jump hint is dropped, since `ctrl+o` cannot reach it. Same tone
/// as the current-session block (§ui-style-guide).
pub(crate) fn context_source_lines(
    src: &crate::model::ContextSource,
    resolved: Option<&crate::model::Session>,
    marks: crate::ui::bookmarks::TitleMarks,
    inner_w: usize,
    th: &Theme,
    dimmed: bool,
) -> Vec<Line<'static>> {
    let (tag, _) = agent_tag(src.agent, th);
    match resolved {
        Some(s) => meta_grid(
            MetaGrid {
                heading: "Context Source",
                // The block is the affordance for the jump key: it appears only for
                // a context-derived session, and the hint only when the source is
                // actually reachable (the unresolved arm below passes None).
                heading_hint: Some(CONTEXT_SOURCE_JUMP_HINT),
                folder: crate::scratch::folder_label(&s.cwd, &s.folder),
                full_path: &s.cwd.to_string_lossy(),
                title: &crate::ui::bookmarks::display_title(s, marks),
                legend: Vec::new(),
                extra_rows: &[],
                id_value: Some(format!("[{}] {} · {}", tag.trim(), src.id, src.profile)),
            },
            inner_w,
            th,
            dimmed,
        ),
        None => {
            // Unresolved source (deleted or from an unscanned profile): keep only
            // the heading and the Id row (the empty Project/Name rows are dropped).
            let full = meta_grid(
                MetaGrid {
                    heading: "Context Source",
                    heading_hint: None,
                    folder: "",
                    full_path: "",
                    title: "",
                    legend: Vec::new(),
                    extra_rows: &[],
                    id_value: Some(format!(
                        "[{}] {} · {}  (source unavailable)",
                        tag.trim(),
                        src.id,
                        src.profile
                    )),
                },
                inner_w,
                th,
                dimmed,
            );
            full.first()
                .into_iter()
                .chain(full.last())
                .cloned()
                .collect()
        }
    }
}

/// Content of a `● <Heading>` + `- Field: value` metadata grid (see [`meta_grid`]).
struct MetaGrid<'a> {
    heading: &'a str,
    /// Hotkey hint pushed to the right end of the heading line (e.g. `<ctrl+o>`).
    /// Dropped when the pane is too narrow to keep [`HEADING_HINT_MIN_GAP`] between
    /// the heading and the hint — the heading itself is never truncated for it.
    heading_hint: Option<&'a str>,
    folder: &'a str,
    full_path: &'a str,
    title: &'a str,
    /// `<marker> : <meaning>` rows explaining the title markers, under Name.
    legend: Vec<(char, &'static str)>,
    /// Soft-dim `- label + value` rows rendered between Name and Id.
    extra_rows: &'a [(&'a str, String)],
    /// Id row value; omitted when `None`.
    id_value: Option<String>,
}

/// Shared `● <Heading>` + `- Field: value` metadata grid used by both the current
/// session block and the Context Source block.
fn meta_grid(g: MetaGrid, inner_w: usize, th: &Theme, dimmed: bool) -> Vec<Line<'static>> {
    let MetaGrid {
        heading,
        heading_hint,
        folder,
        full_path,
        title,
        legend,
        extra_rows,
        id_value,
    } = g;
    let accent_style = if dimmed {
        th.soft_dim().add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(th.accent).add_modifier(Modifier::BOLD)
    };
    let mut lines: Vec<Line> = Vec::new();
    let heading_text = format!("● {heading}");
    let mut heading_spans = vec![Span::styled(heading_text.clone(), accent_style)];
    if let Some(hint) = heading_hint {
        let gap = inner_w.saturating_sub(heading_text.width() + hint.width());
        if gap >= HEADING_HINT_MIN_GAP {
            heading_spans.push(Span::raw(" ".repeat(gap)));
            heading_spans.push(Span::styled(
                hint.to_string(),
                if dimmed {
                    th.soft_dim()
                } else {
                    th.key_style()
                },
            ));
        }
    }
    lines.push(Line::from(heading_spans));

    let project_prefix = "- Project: ";
    // Allocates residual width to brackets and full paths: total - prefix - folder - " (" - ")".
    let path_avail = inner_w
        .saturating_sub(project_prefix.width())
        .saturating_sub(folder.width())
        .saturating_sub(3);
    lines.push(Line::from(vec![
        Span::styled(project_prefix, th.soft_dim()),
        Span::styled(folder.to_string(), accent_style),
        Span::styled(
            format!(
                " ({})",
                truncate_w_with_ellipsis(full_path, path_avail, "...")
            ),
            th.soft_dim(),
        ),
    ]));

    let name_prefix = "- Name: ";
    let name_w = inner_w.saturating_sub(name_prefix.width());
    lines.push(Line::from(vec![
        Span::styled(name_prefix, th.soft_dim()),
        Span::styled(truncate_w_with_ellipsis(title, name_w, "..."), accent_style),
    ]));

    // Same `- ` bullet as the other rows; the marker keeps its bold title style.
    for (glyph, meaning) in legend {
        let glyph = glyph.to_string();
        let rest_w = inner_w.saturating_sub(2 + glyph.width() + 3);
        lines.push(Line::from(vec![
            Span::styled("- ", th.soft_dim()),
            Span::styled(glyph, th.soft_dim().add_modifier(Modifier::BOLD)),
            Span::styled(" : ", th.soft_dim()),
            Span::styled(
                truncate_w_with_ellipsis(meaning, rest_w, "..."),
                th.soft_dim(),
            ),
        ]));
    }

    for (label, value) in extra_rows {
        lines.push(Line::from(vec![
            Span::styled(format!("- {label}"), th.soft_dim()),
            Span::styled(value.clone(), th.soft_dim()),
        ]));
    }

    if let Some(id_value) = id_value {
        let id_prefix = "- Id: ";
        let id_w = inner_w.saturating_sub(id_prefix.width());
        lines.push(Line::from(vec![
            Span::styled(id_prefix, th.soft_dim()),
            Span::styled(
                truncate_w_with_ellipsis(&id_value, id_w, "..."),
                th.soft_dim(),
            ),
        ]));
    }
    lines
}

// ---- Helpers ----

pub(crate) fn agent_tag(agent: crate::model::Agent, th: &Theme) -> (&'static str, Color) {
    use crate::model::Agent;
    match agent {
        Agent::Claude => ("CLD ", th.agent_claude),
        Agent::Antigravity => ("AGY ", th.agent_antigravity),
        Agent::Codex => ("CDX ", th.agent_codex),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PreviewTurnLine<'a> {
    Content(&'a str),
    Omission(usize),
}

/// Max user-turn lines shown in full before `preview_turn_lines` collapses the middle.
pub(crate) const PREVIEW_TURN_MAX_LINES: usize = 8;

/// Whether a user turn is long enough that `preview_turn_lines` omits part of it — i.e.
/// whether expanding it via `.` would actually reveal more content.
pub(crate) fn preview_turn_is_truncated(turn: &str) -> bool {
    turn.lines().count() > PREVIEW_TURN_MAX_LINES
}

/// User-turn preview lines for both the Session preview and Detail Prompt panels:
/// the full turn (no omission) when `expanded`, otherwise the abbreviated
/// `preview_turn_lines` output. Shared so the two panels stay in sync.
pub(crate) fn preview_turn_display(turn: &str, expanded: bool) -> Vec<PreviewTurnLine<'_>> {
    if expanded {
        turn.lines().map(PreviewTurnLine::Content).collect()
    } else {
        preview_turn_lines(turn)
    }
}

/// If user query exceeds 8 lines, preserves first 4 and last 4 lines, rendering an omission placeholder for the rest.
pub(crate) fn preview_turn_lines(turn: &str) -> Vec<PreviewTurnLine<'_>> {
    let lines: Vec<&str> = turn.lines().collect();
    if lines.len() <= PREVIEW_TURN_MAX_LINES {
        return lines.into_iter().map(PreviewTurnLine::Content).collect();
    }

    lines[..4]
        .iter()
        .copied()
        .map(PreviewTurnLine::Content)
        .chain(std::iter::once(PreviewTurnLine::Omission(lines.len() - 8)))
        .chain(
            lines[lines.len() - 4..]
                .iter()
                .copied()
                .map(PreviewTurnLine::Content),
        )
        .collect()
}
