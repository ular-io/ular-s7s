//! Neutral session-context data model.
//!
//! Terminology (kept consistent across code, UI, help, and docs):
//! - Session context: parsed historical content exposed for reference.
//! - User turn: human-authored input plus promoted question/answer interactions.
//! - Last assistant text: last extracted assistant text for a turn; NOT guaranteed
//!   to be a semantic final answer (hence not named `final_answer`).

use crate::model::Agent;
use std::path::PathBuf;

/// Full parsed context of one source session.
#[derive(Debug, Clone)]
pub struct SessionContext {
    pub source: SessionContextSource,
    pub completeness: ContextCompleteness,
    pub turns: Vec<ContextTurn>,
}

/// Identity of the source session the context was read from.
#[derive(Debug, Clone)]
pub struct SessionContextSource {
    pub agent: Agent,
    pub profile_id: String,
    pub session_id: String,
    pub title: String,
    pub cwd: PathBuf,
}

/// One user turn: the (redacted) user text, ordered work entries, and the last
/// assistant text extracted for the turn.
#[derive(Debug, Clone, Default)]
pub struct ContextTurn {
    pub user: String,
    pub last_assistant_text: Option<String>,
    pub entries: Vec<ContextEntry>,
}

/// One intermediate work record inside a turn (assistant text, tool call/result).
#[derive(Debug, Clone)]
pub struct ContextEntry {
    pub kind: ContextEntryKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextEntryKind {
    AssistantText,
    ToolCall,
    ToolResult,
    /// Reserved for recognizing nested `s7s session` calls later so future context
    /// exports never recursively embed entire referenced sessions. Not produced yet.
    #[allow(dead_code)]
    SessionReference,
}

impl ContextEntryKind {
    pub fn heading(self) -> &'static str {
        match self {
            ContextEntryKind::AssistantText => "Assistant Text",
            ContextEntryKind::ToolCall => "Tool Call",
            ContextEntryKind::ToolResult => "Tool Result",
            ContextEntryKind::SessionReference => "Session Reference",
        }
    }
}

/// How much of the session could actually be parsed. A non-`Full` value means the
/// turns are the pre-extracted user turns only (no assistant/work entries), and the
/// consumer must say so instead of claiming complete context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextCompleteness {
    /// Detailed parsing succeeded: turns carry assistant text and work entries.
    Full,
    /// Only pre-extracted user turns are available (e.g. Antigravity transcript
    /// missing while the conversation DB itself is fine).
    UserTurnsOnly,
    /// The source transcript file no longer exists.
    SourceUnavailable,
    /// The source exists but detailed parsing failed or produced nothing.
    ParseFailed,
}

impl ContextCompleteness {
    /// Short human-readable label for CLI output.
    pub fn label(self) -> &'static str {
        match self {
            ContextCompleteness::Full => "full context",
            ContextCompleteness::UserTurnsOnly => {
                "user turns only (assistant/work entries unavailable)"
            }
            ContextCompleteness::SourceUnavailable => {
                "user turns only (source transcript unavailable)"
            }
            ContextCompleteness::ParseFailed => "user turns only (detailed parsing failed)",
        }
    }
}
