//! Clipboard copy (`c` key) for the Session list and Detail screens.
//!
//! The four copy targets map one-to-one onto the two screens' two focusable
//! panels, and every builder emits the *full* content regardless of the on-screen
//! omission (collapsed previews, `⋯ lines omitted ⋯`, hidden tool call/result
//! entries):
//!
//! | Screen  | Focus            | Copied content                                   |
//! | ------- | ---------------- | ------------------------------------------------ |
//! | Session | `Focus::Table`   | Selected session's basic info (Prompt pane head) |
//! | Session | `Focus::Preview` | All user turns of the selected session           |
//! | Detail  | `Questions`      | The single selected user turn                    |
//! | Detail  | `Work`           | The selected turn's whole work log + final answer |
//!
//! The pure builders below are unit-tested; `App::copy_selection` performs the
//! routing and the actual clipboard write (skipped under `cfg!(test)` so tests
//! never touch the system clipboard).

use crate::handoff::HandoffTurn;
use crate::model::{format_local_datetime_seconds, Agent, Session};
use crate::ui::{App, DetailFocus, Focus, Screen};

impl App {
    /// Copies the content of the currently focused panel to the system clipboard,
    /// reporting the outcome via the status bar. Bound to `c` on the Session list
    /// and Detail screens.
    pub(crate) fn copy_selection(&mut self) {
        let Some((label, content)) = self.build_copy() else {
            self.status_msg = Some("Nothing to copy here".to_string());
            return;
        };
        // Tests never reach the real clipboard; the routing/builders are covered
        // by the pure `build_copy` path and the builder unit tests.
        if cfg!(test) {
            self.status_msg = Some(format!("Copied {label}"));
            return;
        }
        match set_clipboard(&content) {
            Ok(()) => self.status_msg = Some(format!("Copied {label} to clipboard")),
            Err(e) => self.status_msg = Some(format!("Copy failed: {e}")),
        }
    }

    /// Resolves the focused panel to a `(label, content)` pair, or `None` when
    /// there is nothing to copy (no session/turn available).
    fn build_copy(&self) -> Option<(String, String)> {
        match self.screen {
            Screen::Session => {
                let s = self.current()?;
                match self.focus {
                    Focus::Table => Some(("session info".to_string(), session_info_text(s))),
                    Focus::Preview => {
                        if s.user_turns.is_empty() {
                            return None;
                        }
                        let label = format!("{} user turn(s)", s.user_turns.len());
                        Some((label, all_user_turns_text(s)))
                    }
                }
            }
            Screen::Detail => {
                let d = self.detail.as_ref()?;
                let turn = d.turns.get(d.selected)?;
                match d.focus {
                    DetailFocus::Questions => Some((
                        format!("Q{}", d.selected + 1),
                        single_turn_text(d.selected, turn),
                    )),
                    DetailFocus::Work => Some((
                        format!("Q{} work & answer", d.selected + 1),
                        work_answer_text(turn),
                    )),
                }
            }
            _ => None,
        }
    }
}

/// Short agent tag matching the trimmed `render::agent_tag` label.
fn agent_label(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "CLD",
        Agent::Antigravity => "AGY",
        Agent::Codex => "CDX",
    }
}

/// Plain-text mirror of the Prompt pane's top `session_meta_lines` block, with the
/// full (untruncated) project path, title, and id.
pub(crate) fn session_info_text(s: &Session) -> String {
    format!(
        "● Project: {} ({})\n● Name: {}\n● Created at: {}\n● Updated at: {}\n● Id: [{}] {}",
        s.folder,
        s.cwd.to_string_lossy(),
        s.title(),
        s.created_str(),
        s.updated_str(),
        agent_label(s.agent),
        s.id,
    )
}

/// All user turns of a session, each preceded by its `● Q{n}  {timestamp}` header,
/// in full (no preview omission).
pub(crate) fn all_user_turns_text(s: &Session) -> String {
    let mut out = String::new();
    for (idx, turn) in s.user_turns.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(&format!("● Q{}", idx + 1));
        if let Some(ts) = s
            .user_turn_timestamp_ms(idx)
            .and_then(format_local_datetime_seconds)
        {
            out.push_str(&format!("  {ts}"));
        }
        out.push('\n');
        out.push_str(turn.trim_end());
        out.push('\n');
    }
    out
}

/// A single user turn preceded by its `● Q{n}  {timestamp}` header (same format as
/// the Prompt panel), with the full turn body.
pub(crate) fn single_turn_text(index: usize, turn: &HandoffTurn) -> String {
    let mut out = format!("● Q{}", index + 1);
    if let Some(ts) = turn.submitted_at_ms.and_then(format_local_datetime_seconds) {
        out.push_str(&format!("  {ts}"));
    }
    out.push('\n');
    out.push_str(turn.user.trim_end());
    out
}

/// The whole work log (all entries, including tool call/result entries hidden in
/// the UI) followed by the final answer, in full length (no per-entry caps).
pub(crate) fn work_answer_text(turn: &HandoffTurn) -> String {
    let mut out = String::new();
    for (i, entry) in turn.work_entries.iter().enumerate() {
        out.push_str(&format!("● {} {}\n", entry.kind.heading(), i + 1));
        out.push_str(entry.text.trim_end());
        out.push_str("\n\n");
    }
    out.push_str("● Final Answer\n");
    match turn.final_answer.as_deref() {
        Some(answer) if !answer.trim().is_empty() => out.push_str(answer.trim_end()),
        _ => out.push_str("(no final answer)"),
    }
    out
}

/// Writes `text` to the system clipboard via `arboard`.
fn set_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_text(text.to_owned())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handoff::{WorkEntry, WorkKind};
    use crate::model::Agent;
    use std::path::PathBuf;

    fn sample_session() -> Session {
        Session {
            agent: Agent::Claude,
            profile_id: String::new(),
            id: "abc-123".to_string(),
            source_path: None,
            cwd: PathBuf::from("/home/dev/projects/demo"),
            folder: "demo".to_string(),
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["first prompt".to_string(), "second\nmulti-line".to_string()],
            user_turn_timestamps_ms: vec![None, None],
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: Some("Demo title".to_string()),
            title_fixed: true,
        }
    }

    #[test]
    fn session_info_has_full_path_and_all_fields() {
        let text = session_info_text(&sample_session());
        assert!(text.contains("● Project: demo (/home/dev/projects/demo)"));
        assert!(text.contains("● Name: Demo title"));
        assert!(text.contains("● Id: [CLD] abc-123"));
        assert!(text.contains("● Created at:"));
        assert!(text.contains("● Updated at:"));
    }

    #[test]
    fn all_user_turns_includes_every_turn_in_full() {
        let text = all_user_turns_text(&sample_session());
        assert!(text.contains("● Q1\nfirst prompt"));
        // Multi-line turns are copied whole (no preview omission).
        assert!(text.contains("● Q2\nsecond\nmulti-line"));
    }

    #[test]
    fn single_turn_prepends_q_header() {
        let turn = HandoffTurn {
            user: "hello\nworld".to_string(),
            submitted_at_ms: None,
            final_answer: None,
            work_entries: vec![],
        };
        let text = single_turn_text(1, &turn);
        assert!(text.starts_with("● Q2\nhello\nworld"));
    }

    #[test]
    fn work_answer_includes_all_entries_and_final_answer() {
        let turn = HandoffTurn {
            user: "q".to_string(),
            submitted_at_ms: None,
            final_answer: Some("the answer".to_string()),
            work_entries: vec![
                WorkEntry {
                    kind: WorkKind::AssistantText,
                    text: "thinking".to_string(),
                },
                WorkEntry {
                    kind: WorkKind::ToolCall,
                    text: "call payload".to_string(),
                },
            ],
        };
        let text = work_answer_text(&turn);
        assert!(text.contains("● Assistant Text 1\nthinking"));
        // Tool entries hidden in the UI are still copied.
        assert!(text.contains("● Tool Call 2\ncall payload"));
        assert!(text.contains("● Final Answer\nthe answer"));
    }

    #[test]
    fn work_answer_without_final_answer_notes_absence() {
        let turn = HandoffTurn {
            user: "q".to_string(),
            submitted_at_ms: None,
            final_answer: None,
            work_entries: vec![],
        };
        let text = work_answer_text(&turn);
        assert!(text.contains("● Final Answer\n(no final answer)"));
    }
}
