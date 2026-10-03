//! Bookmark actions shared by the Session and Detail screens and the palette.

use crate::model::Session;
use crate::ui::{effect::AppEffect, App};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

/// Display-only markers placed before a session title: the bookmark `♥`, then
/// the Claude live status (`Ⓑ` background / `Ⓞ` open). Markers are separated by one
/// space. The title follows a circled letter after two spaces, since terminals
/// may draw those ambiguous-width glyphs two cells wide, and a lone `♥` after one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TitleMarks {
    pub bookmarked: bool,
    pub live_status: Option<crate::agent_status::LiveStatus>,
}

impl TitleMarks {
    fn glyphs(self) -> impl Iterator<Item = char> {
        self.bookmarked
            .then_some('♥')
            .into_iter()
            .chain(self.live_status.map(|s| s.glyph()))
    }

    /// `(marker, meaning)` for each marker present, in prefix order.
    pub(crate) fn legend(self) -> Vec<(char, &'static str)> {
        self.bookmarked
            .then_some(('♥', "Bookmarked"))
            .into_iter()
            .chain(self.live_status.map(|s| (s.glyph(), s.description())))
            .collect()
    }

    fn prefix(self) -> Option<String> {
        let glyphs: Vec<String> = self.glyphs().map(String::from).collect();
        (!glyphs.is_empty()).then(|| glyphs.join(" "))
    }
}

pub(crate) fn display_title(session: &Session, marks: TitleMarks) -> String {
    match marks.prefix() {
        Some(prefix) if prefix.ends_with('♥') => format!("{prefix} {}", session.title()),
        Some(prefix) => format!("{prefix}  {}", session.title()),
        None => session.title(),
    }
}

/// Apply bold to the markers after truncation, preserving the title's normal tone.
pub(crate) fn styled_title(title: String, marks: TitleMarks, style: Style) -> Line<'static> {
    let glyphs: Vec<char> = marks.glyphs().collect();
    if glyphs.is_empty() {
        return Line::from(Span::styled(title, style));
    }
    let mut spans = Vec::new();
    let mut rest = title.as_str();
    for (i, glyph) in glyphs.iter().enumerate() {
        let Some(after) = rest.strip_prefix(*glyph) else {
            break;
        };
        spans.push(Span::styled(
            glyph.to_string(),
            style.add_modifier(Modifier::BOLD),
        ));
        rest = after;
        if i + 1 < glyphs.len() {
            if let Some(after) = rest.strip_prefix(' ') {
                spans.push(Span::styled(" ", style));
                rest = after;
            }
        }
    }
    spans.push(Span::styled(rest.to_string(), style));
    Line::from(spans)
}

impl App {
    pub(crate) fn toggle_focused_bookmark(&mut self) {
        let Some(idx) = self.focused_session_index() else {
            self.status_msg = Some("Select a session first".to_string());
            return;
        };
        self.pending_effect = Some(AppEffect::ToggleBookmark { idx });
    }

    pub(crate) fn title_marks(&self, session: &Session) -> TitleMarks {
        TitleMarks {
            bookmarked: self.bookmarks.contains(session),
            live_status: self.session_live_status(session),
        }
    }

    pub(crate) fn session_display_title(&self, session: &Session) -> String {
        display_title(session, self.title_marks(session))
    }

    pub(crate) fn run_toggle_bookmark(&mut self, idx: usize) {
        let Some(session) = self.sessions.get(idx) else {
            self.status_msg = Some("Bookmark target no longer exists".to_string());
            return;
        };
        match crate::bookmarks::BookmarkStore::toggle(&self.bookmarks_path, session) {
            Ok(store) => {
                let current = self.filtered.get(self.selected).copied();
                let bookmarked = store.contains(session);
                self.bookmarks = store;
                self.status_msg = Some(if bookmarked {
                    "Session bookmarked".to_string()
                } else {
                    "Session bookmark removed".to_string()
                });
                // Reorder immediately, following the same session when it moves.
                // Only a removed filtered row resets the preview's state.
                self.rebuild_filtered();
                let position = current.and_then(|idx| self.filtered.iter().position(|&i| i == idx));
                if let Some(position) = position {
                    self.selected = position;
                } else {
                    self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
                    self.preview_scroll = 0;
                    self.preview_expanded = false;
                }
            }
            Err(err) => self.status_msg = Some(format!("Bookmark failed: {err}")),
        }
    }

    pub(crate) fn toggle_bookmark_filter(&mut self) {
        let current = self.filtered.get(self.selected).copied();
        self.filter.bookmarked_only = !self.filter.bookmarked_only;
        self.recompute();
        if let Some(pos) = current.and_then(|idx| self.filtered.iter().position(|&i| i == idx)) {
            self.selected = pos;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::bookmarks::BookmarkStore;
    use crate::ui::effect::AppEffect;
    use crate::ui::quick::registry::{CommandId, COMMANDS};
    use crate::ui::test_support::*;
    use crate::ui::{App, Focus, Screen, UiMode};
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::{backend::TestBackend, Terminal};

    fn ctrl_b() -> crossterm::event::KeyEvent {
        key(KeyCode::Char('b'), KeyModifiers::CONTROL)
    }

    fn palette(app: &mut App, query: &str) {
        app.open_quick_command();
        for c in query.chars() {
            app.on_key_quick(key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.on_key_quick(key(KeyCode::Enter, KeyModifiers::NONE));
    }

    fn frame(app: &App, width: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
        terminal.draw(|f| crate::ui::render::draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..30)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn ctrl_b_toggles_durably_without_returning_or_resetting_the_preview() {
        let temp = TempBookmarkStore::new();
        let mut app = app_with_context_chain();
        app.bookmarks_path = temp.path.clone();
        app.on_key_table(key(KeyCode::Char('o'), KeyModifiers::CONTROL));
        app.preview_expanded = true;
        app.preview_scroll = 2;
        let title = app.current().unwrap().title();

        app.on_key_table(ctrl_b());
        assert_eq!(
            app.pending_effect,
            Some(AppEffect::ToggleBookmark { idx: 1 })
        );
        assert!(
            !temp.path.exists(),
            "the key handler must not write to disk"
        );
        app.apply_effect();

        assert_eq!(app.current().unwrap().id, "middle");
        assert!(
            app.can_return_to_jump_origin(),
            "bookmarking must preserve the return stack"
        );
        assert!(app.preview_expanded);
        assert_eq!(app.preview_scroll, 2);
        assert_eq!(
            app.current().unwrap().title(),
            title,
            "stored titles are unchanged"
        );
        assert_eq!(
            app.session_display_title(app.current().unwrap()),
            format!("♥ {title}")
        );
        let reloaded = BookmarkStore::load(&temp.path).unwrap();
        assert!(reloaded.contains(app.current().unwrap()));

        app.focus = Focus::Preview;
        app.on_key_table(ctrl_b());
        app.apply_effect();
        assert!(!app.bookmarks.contains(app.current().unwrap()));
        assert_eq!(app.session_display_title(app.current().unwrap()), title);
        assert!(!BookmarkStore::load(&temp.path)
            .unwrap()
            .contains(app.current().unwrap()));
    }

    #[test]
    fn detail_hotkey_and_palette_bookmark_the_detail_target() {
        let temp = TempBookmarkStore::new();
        let mut app = app_with_context_chain();
        app.bookmarks_path = temp.path.clone();
        app.open_session_detail();
        assert_eq!(app.screen, Screen::Detail);
        app.selected = 2; // The main list cursor must not choose the Detail target.

        app.on_key_detail(ctrl_b());
        assert_eq!(
            app.pending_effect,
            Some(AppEffect::ToggleBookmark { idx: 0 })
        );
        app.apply_effect();
        assert!(app.bookmarks.contains(&app.sessions[0]));
        assert!(!app.bookmarks.contains(&app.sessions[2]));
        assert!(frame(&app, 160).contains("♥ leaf"));

        palette(&mut app, "toggle bookmark");
        assert_eq!(app.mode, UiMode::Table);
        app.apply_effect();
        assert!(!app.bookmarks.contains(&app.sessions[0]));
    }

    #[test]
    fn palette_bookmark_filter_combines_with_search_and_clears_with_zero() {
        let temp = TempBookmarkStore::new();
        let mut app = app_with_context_chain();
        app.bookmarks_path = temp.path.clone();
        palette(&mut app, "toggle bookmark");
        app.apply_effect();
        app.selected = 1;
        app.on_key_table(ctrl_b());
        app.apply_effect();

        palette(&mut app, "filter bookmarked");
        assert!(app.filter.bookmarked_only);
        assert_eq!(app.filtered, vec![0, 1]);
        assert_eq!(app.current().unwrap().id, "middle");
        assert_eq!(app.filter.describe_with(|_| None), "bookmarked");
        app.filter.keyword = "leaf".to_string();
        app.recompute();
        assert_eq!(app.filtered, vec![0]);

        // Removing the only result leaves a valid empty list; the filter remains
        // clearable through the ordinary key, with no stale selected row.
        app.on_key_table(ctrl_b());
        app.apply_effect();
        assert!(app.filtered.is_empty());
        app.on_key_table(ctrl_b());
        assert!(app.pending_effect.is_none());
        app.on_key_table(key(KeyCode::Char('0'), KeyModifiers::NONE));
        assert!(!app.filter.is_active());
        assert_eq!(app.filtered.len(), 3);
        assert!(app.bookmarks.contains(&app.sessions[1]));

        palette(&mut app, "filter bookmarked");
        assert_eq!(app.filtered, vec![1]);
        palette(&mut app, "filter bookmarked");
        assert_eq!(app.filtered.len(), 3);
    }

    #[test]
    fn palette_disables_bookmark_without_a_session_and_back_has_no_hotkey() {
        let mut app = empty_app();
        app.on_key_table(ctrl_b());
        assert!(app.pending_effect.is_none());
        for screen in [Screen::Session, Screen::Profile] {
            app.screen = screen;
            palette(&mut app, "toggle bookmark");
            assert_eq!(app.mode, UiMode::QuickCommand);
            assert!(!app.quick.as_ref().unwrap().items[0].enabled);
        }
        let bound: Vec<_> = COMMANDS
            .iter()
            .filter(|c| c.shortcut == Some("ctrl+b"))
            .collect();
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].id, CommandId::ToggleBookmark);
        assert_eq!(
            COMMANDS
                .iter()
                .find(|c| c.id == CommandId::BackToPreviousSession)
                .unwrap()
                .shortcut,
            None
        );
    }

    #[test]
    fn failed_bookmark_write_keeps_the_marker_and_existing_store_unchanged() {
        let temp = TempBookmarkStore::new();
        let mut app = app_with_session();
        app.bookmarks_path = temp.path.clone();
        app.on_key_table(ctrl_b());
        app.apply_effect();
        assert!(app.bookmarks.contains(app.current().unwrap()));
        std::fs::write(&temp.path, "broken json").unwrap();
        app.on_key_table(ctrl_b());
        app.apply_effect();
        assert!(app.bookmarks.contains(app.current().unwrap()));
        assert!(app
            .status_msg
            .as_deref()
            .unwrap()
            .starts_with("Bookmark failed:"));
        assert_eq!(std::fs::read_to_string(&temp.path).unwrap(), "broken json");
    }

    #[test]
    fn cancelling_or_failing_deletion_keeps_the_session_and_bookmark() {
        let (mut app, root) = app_with_two_deletable_sessions();
        app.on_key_table(ctrl_b());
        app.apply_effect();
        let target = app.sessions[0].clone();
        let saved = std::fs::read(&app.bookmarks_path).unwrap();

        app.on_key_table(key(KeyCode::Char('d'), KeyModifiers::CONTROL));
        // Cancel is the default button.
        app.on_key_delete_confirm(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.pending_effect.is_none());
        assert_eq!(std::fs::read(&app.bookmarks_path).unwrap(), saved);
        assert_eq!(app.sessions.len(), 2);

        let source = target.source_path.as_ref().unwrap();
        std::fs::remove_file(source).unwrap();
        std::fs::create_dir(source).unwrap();
        app.pending_effect = Some(AppEffect::DeleteSession { idx: 0 });
        app.apply_effect();
        assert!(app
            .status_msg
            .as_deref()
            .unwrap()
            .starts_with("Delete failed:"));
        assert_eq!(app.sessions.len(), 2);
        assert!(app.bookmarks.contains(&target));
        assert_eq!(std::fs::read(&app.bookmarks_path).unwrap(), saved);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bookmarks_lead_in_activity_order_and_toggles_follow_the_same_session() {
        let temp = TempBookmarkStore::new();
        let mut app = app_with_context_chain();
        app.bookmarks_path = temp.path.clone();
        for (idx, session) in app.sessions.iter_mut().enumerate() {
            session.updated_at_ms = 300 - idx as i64 * 100;
        }
        app.selected = 2;
        app.on_key_table(ctrl_b());
        app.apply_effect();
        assert_eq!(app.filtered, vec![2, 0, 1]);
        assert_eq!(app.selected, 0);
        assert_eq!(app.current().unwrap().id, "root");

        // The newer bookmark leads even though the older one was marked first.
        app.selected = 2;
        app.on_key_table(ctrl_b());
        app.apply_effect();
        assert_eq!(app.filtered, vec![1, 2, 0]);
        assert_eq!(app.current().unwrap().id, "middle");
        assert_eq!(app.selected, 0);
        // The parsed index remains unchanged; Detail/source indices stay valid.
        assert_eq!(
            app.sessions
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            vec!["leaf", "middle", "root"]
        );

        app.filter.keyword = "root".to_string();
        app.recompute();
        assert_eq!(app.filtered, vec![2]);
        app.on_key_table(key(KeyCode::Char('0'), KeyModifiers::NONE));
        assert_eq!(app.filtered, vec![1, 2, 0]);
        app.selected = 0;
        app.on_key_table(ctrl_b());
        app.apply_effect();
        assert_eq!(app.filtered, vec![2, 0, 1]);
        assert_eq!(app.current().unwrap().id, "middle");
        assert_eq!(app.selected, 2);

        app.bookmarks = BookmarkStore::load(&temp.path).unwrap();
        app.recompute();
        assert_eq!(app.filtered, vec![2, 0, 1]);
    }

    #[test]
    fn unselected_bookmark_marker_is_bold_without_bolding_the_title() {
        use ratatui::style::Modifier;
        let mut app = app_with_context_chain();
        app.bookmarks.set(&app.sessions[2], true);
        app.recompute();
        app.selected = 1;
        for focus in [Focus::Table, Focus::Preview] {
            app.focus = focus;
            let mut terminal = Terminal::new(TestBackend::new(100, 10)).unwrap();
            terminal
                .draw(|f| crate::ui::session::render::draw_table(f, &app, f.area()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let (x, y) = (0..10)
                .flat_map(|y| (0..100).map(move |x| (x, y)))
                .find(|&(x, y)| buffer[(x, y)].symbol() == "♥")
                .expect("bookmark cell");
            assert!(buffer[(x, y)].modifier.contains(Modifier::BOLD));
            assert_eq!(buffer[(x + 1, y)].symbol(), " ");
            assert_eq!(buffer[(x + 2, y)].symbol(), "r");
            assert!(!buffer[(x + 2, y)].modifier.contains(Modifier::BOLD));
        }
    }

    #[test]
    fn bookmark_fill_covers_the_row_and_yields_to_selection_in_every_theme() {
        let mut app = app_with_context_chain();
        app.bookmarks.set(&app.sessions[2], true);
        app.recompute();
        for theme in crate::theme::builtin_themes() {
            app.theme = theme;
            for width in [160, 60] {
                for focus in [Focus::Table, Focus::Preview] {
                    app.focus = focus;
                    for selected in [0, 1] {
                        app.selected = selected;
                        let mut terminal = Terminal::new(TestBackend::new(width, 10)).unwrap();
                        terminal
                            .draw(|f| crate::ui::session::render::draw_table(f, &app, f.area()))
                            .unwrap();
                        let buffer = terminal.backend().buffer();
                        for row in 0..3 {
                            let expected = if row == selected {
                                if focus == Focus::Table {
                                    app.theme.selection_bg
                                } else {
                                    app.theme.selection_inactive_bg
                                }
                            } else if row == 0 {
                                app.theme.bookmark_bg
                            } else {
                                // This isolated table has no root background fill.
                                ratatui::style::Color::Reset
                            };
                            for x in 1..width - 1 {
                                assert_eq!(
                                    buffer[(x, row as u16 + 2)].bg,
                                    expected,
                                    "theme={} focus={focus:?} selected={selected} row={row} x={x}",
                                    app.theme.key
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn title_marker_has_spacing_and_width_aware_truncation_in_both_themes() {
        use crate::theme::{builtin_themes, default_theme};
        let mut app = app_with_context_chain();
        app.sessions[0].title_hint =
            Some("Session title with a long suffix for truncation".to_string());
        app.bookmarks.set(&app.sessions[0], true);
        app.bookmarks.set(&app.sessions[1], true);
        let light = builtin_themes()
            .into_iter()
            .find(|theme| !theme.dark)
            .unwrap();
        for theme in [default_theme(), light] {
            app.theme = theme;
            for width in [200, 120, 100, 80, 60, 40] {
                for focus in [Focus::Table, Focus::Preview] {
                    app.focus = focus;
                    let text = frame(&app, width);
                    assert!(text.contains("♥ "), "marker missing at width {width}");
                    assert!(
                        !text.contains("♥Session"),
                        "marker must be separated from title"
                    );
                }
            }
        }
        let text = frame(&app, 200);
        assert!(
            text.contains("♥ middle"),
            "context source title is also decorated"
        );
        app.bookmarks.set(&app.sessions[0], false);
        assert!(!frame(&app, 200).contains("♥ Session"));
    }
}
