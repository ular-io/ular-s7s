//! Deterministic handoff Markdown generation (optional consumer of the shared
//! session-context model) and the `HandoffTurn` compatibility adapter used by
//! the TUI Detail screen.
//!
//! Detailed turn parsing lives in [`crate::session_context`]; this module only
//! converts [`crate::session_context::ContextTurn`] into the legacy
//! `HandoffTurn` shape and renders Markdown from it.

use crate::model::{Agent, Session};
use crate::session_context::{self, redact::redact, ContextEntryKind, ContextTurn};
use anyhow::Result;
use chrono::Local;
use std::path::{Path, PathBuf};

const TARGET_TURNS: i64 = 8;
const TARGET_CHARS: i64 = 7_000;
const MAX_MAIN_CHARS: usize = 1_200;
const MAX_WORK_CHARS: usize = 8_000;

pub struct HandoffReport {
    pub agent: Agent,
    pub title: String,
    pub turn_count: usize,
    pub path: PathBuf,
}

/// A single user question + corresponding agent actions/answers. Legacy adapter
/// over [`ContextTurn`] shared between the handoff document and the session
/// detail screen (TUI) until both migrate to the context model directly.
#[derive(Debug, Clone, Default)]
pub struct HandoffTurn {
    pub user: String,
    pub submitted_at_ms: Option<i64>,
    pub final_answer: Option<String>,
    pub work_entries: Vec<WorkEntry>,
}

#[derive(Debug, Clone)]
pub struct WorkEntry {
    pub kind: WorkKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkKind {
    AssistantText,
    ToolCall,
    ToolResult,
}

impl WorkKind {
    pub fn heading(self) -> &'static str {
        match self {
            WorkKind::AssistantText => "Assistant Text",
            WorkKind::ToolCall => "Tool Call",
            WorkKind::ToolResult => "Tool Result",
        }
    }
}

impl From<ContextTurn> for HandoffTurn {
    fn from(turn: ContextTurn) -> Self {
        HandoffTurn {
            user: turn.user,
            submitted_at_ms: turn.submitted_at_ms,
            final_answer: turn.last_assistant_text,
            work_entries: turn
                .entries
                .into_iter()
                .map(|e| WorkEntry {
                    kind: match e.kind {
                        ContextEntryKind::AssistantText => WorkKind::AssistantText,
                        ContextEntryKind::ToolCall => WorkKind::ToolCall,
                        // SessionReference is reserved and not produced yet; a tool
                        // result is the closest legacy rendering if it ever appears.
                        ContextEntryKind::ToolResult | ContextEntryKind::SessionReference => {
                            WorkKind::ToolResult
                        }
                    },
                    text: e.text,
                })
                .collect(),
        }
    }
}

pub fn write_agent_samples(sessions: &[Session], out_dir: &Path) -> Result<Vec<HandoffReport>> {
    std::fs::create_dir_all(out_dir)?;

    let mut reports = Vec::new();
    for agent in Agent::all() {
        let Some(session) = select_sample(sessions, agent) else {
            continue;
        };
        let turns = load_turns(session);
        let sample_dir = out_dir.join(sample_dir_name(session));
        let work_dir = sample_dir.join("work");
        std::fs::create_dir_all(&work_dir)?;

        let mut work_paths = Vec::new();
        for (idx, handoff_turn) in turns.iter().enumerate() {
            let path = work_dir.join(format!("turn-{:03}.md", idx + 1));
            std::fs::write(&path, render_work_log(idx + 1, handoff_turn, session))?;
            work_paths.push(path);
        }

        let handoff_path = sample_dir.join("handoff.md");
        std::fs::write(
            &handoff_path,
            render_main_handoff(session, &turns, &work_paths),
        )?;
        reports.push(HandoffReport {
            agent,
            title: session.title(),
            turn_count: turns.len(),
            path: handoff_path,
        });
    }
    Ok(reports)
}

fn select_sample(sessions: &[Session], agent: Agent) -> Option<&Session> {
    let candidate = |s: &&Session| s.agent == agent && !s.user_turns.is_empty();
    let useful_last = |s: &&Session| {
        s.user_turns
            .last()
            .map(|t| !is_low_value_last_turn(t))
            .unwrap_or(false)
    };

    sessions
        .iter()
        .filter(candidate)
        .filter(useful_last)
        .min_by_key(|s| sample_score(s))
        .or_else(|| {
            sessions
                .iter()
                .filter(candidate)
                .min_by_key(|s| sample_score(s))
        })
}

fn sample_score(session: &Session) -> i64 {
    let turns = session.user_turns.len() as i64;
    let chars = session
        .user_turns
        .iter()
        .map(|t| t.chars().count() as i64)
        .sum::<i64>();

    let turn_penalty = (turns - TARGET_TURNS).abs() * 1_000;
    let char_penalty = (chars - TARGET_CHARS).abs() / 10;
    let too_short_penalty = if turns < 3 || chars < 1_200 {
        15_000
    } else {
        0
    };
    let too_long_penalty = if turns > 18 || chars > 20_000 {
        10_000
    } else {
        0
    };
    let last_turn_penalty = session
        .user_turns
        .last()
        .map(|t| if is_low_value_last_turn(t) { 20_000 } else { 0 })
        .unwrap_or(25_000);

    turn_penalty + char_penalty + too_short_penalty + too_long_penalty + last_turn_penalty
}

/// Parses the turn (question/action/answer) list from the raw session file via the
/// shared session-context loader. Falls back to pre-extracted user_turns only
/// (no actions/answers) if detailed parsing fails — the completeness distinction
/// is exposed through `session_context::load` for callers that need it.
pub fn load_turns(session: &Session) -> Vec<HandoffTurn> {
    session_context::load(session)
        .turns
        .into_iter()
        .map(HandoffTurn::from)
        .collect()
}

fn render_main_handoff(session: &Session, turns: &[HandoffTurn], work_paths: &[PathBuf]) -> String {
    let generated_at = Local::now().format("%Y-%m-%d %H:%M:%S %z").to_string();
    let mut out = String::new();

    push_line(&mut out, "# Handoff Prompt");
    push_line(&mut out, "");
    push_line(
        &mut out,
        "Use this as the first prompt in a new agent session. Each turn links to a separate work log with the intermediate evidence.",
    );
    push_line(&mut out, "");
    push_line(&mut out, "## Source Session");
    push_kv(&mut out, "Agent", session.agent.label());
    push_kv(&mut out, "Session ID", &session.id);
    push_kv(&mut out, "Title", &session.title());
    push_kv(&mut out, "Profile ID", &session.profile_id);
    push_kv(&mut out, "Created", &session.created_str());
    push_kv(&mut out, "Updated", &session.updated_str());
    push_kv(
        &mut out,
        "Previous working directory",
        &path_str(&session.cwd),
    );
    if let Some(source) = &session.source_path {
        push_kv(&mut out, "Source transcript", &path_str(source));
    }
    push_kv(&mut out, "Turns", &turns.len().to_string());
    push_kv(&mut out, "Generated at", &generated_at);
    push_line(&mut out, "");

    push_line(&mut out, "## Workspace Context");
    push_line(
        &mut out,
        "- The original conversation happened in the previous working directory above.",
    );
    push_line(
        &mut out,
        "- Treat old absolute paths as historical references if this handoff is used from another directory.",
    );
    push_line(
        &mut out,
        "- Read linked work logs only when the main user/final-answer flow is not enough.",
    );
    push_line(&mut out, "");

    push_line(&mut out, "## Conversation Digest");
    for (idx, handoff_turn) in turns.iter().enumerate() {
        let work_rel = work_paths
            .get(idx)
            .and_then(|p| p.strip_prefix(work_paths[idx].parent()?.parent()?).ok())
            .map(path_str)
            .unwrap_or_else(|| format!("work/turn-{:03}.md", idx + 1));

        push_line(&mut out, &format!("### Turn {}", idx + 1));
        push_line(&mut out, "");
        push_line(&mut out, "#### User");
        push_blockquote(&mut out, &snippet(&handoff_turn.user, MAX_MAIN_CHARS));
        push_line(&mut out, "");
        push_line(&mut out, "#### Work");
        push_line(&mut out, &format!("- [{}]({})", work_rel, work_rel));
        push_line(&mut out, "");
        push_line(&mut out, "#### Final Answer");
        match handoff_turn.final_answer.as_deref() {
            Some(answer) if !answer.trim().is_empty() => {
                push_blockquote(&mut out, &snippet(answer, MAX_MAIN_CHARS));
            }
            _ => push_line(&mut out, "> _No final assistant answer extracted._"),
        }
        push_line(&mut out, "");
    }

    push_line(&mut out, "## Instructions For The Next Agent");
    push_line(
        &mut out,
        "- Continue from the latest user intent shown above; use work logs only as supporting evidence.",
    );
    push_line(
        &mut out,
        "- Verify current files directly before editing because this handoff may come from an older workspace.",
    );
    push_line(
        &mut out,
        "- Do not mutate original session files or transcripts as part of using this handoff.",
    );

    out
}

fn render_work_log(index: usize, turn: &HandoffTurn, session: &Session) -> String {
    let mut out = String::new();
    push_line(&mut out, &format!("# Work Log - Turn {index}"));
    push_line(&mut out, "");
    push_kv(&mut out, "Agent", session.agent.label());
    push_kv(&mut out, "Session ID", &session.id);
    push_kv(&mut out, "Title", &session.title());
    push_line(&mut out, "");
    push_line(&mut out, "## User");
    push_blockquote(&mut out, &turn.user);
    push_line(&mut out, "");

    push_line(&mut out, "## Work Entries");
    if turn.work_entries.is_empty() {
        push_line(&mut out, "_No intermediate work entries extracted._");
        push_line(&mut out, "");
    } else {
        for (entry_idx, entry) in turn.work_entries.iter().enumerate() {
            push_line(
                &mut out,
                &format!("### {} {}", entry.kind.heading(), entry_idx + 1),
            );
            push_fenced(&mut out, "text", &snippet(&entry.text, MAX_WORK_CHARS));
            push_line(&mut out, "");
        }
    }

    push_line(&mut out, "## Final Answer");
    match turn.final_answer.as_deref() {
        Some(answer) if !answer.trim().is_empty() => push_blockquote(&mut out, answer),
        _ => push_line(&mut out, "> _No final assistant answer extracted._"),
    }
    out
}

fn sample_dir_name(session: &Session) -> String {
    let date = session.updated_str().replace([' ', ':'], "-");
    let title = safe_file_part(&session.title());
    let short_id: String = session.id.chars().take(8).collect();
    format!(
        "handoff-{}-{}-{}-{}",
        session.agent.key(),
        date,
        title,
        short_id
    )
}

fn safe_file_part(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() || matches!(ch, '-' | '_' | '.') {
            out.push('-');
        }
        if out.len() >= 48 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "untitled".to_string()
    } else {
        collapse_dashes(&out)
    }
}

fn collapse_dashes(s: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in s.chars() {
        if ch == '-' {
            if !last_dash {
                out.push(ch);
            }
            last_dash = true;
        } else {
            out.push(ch);
            last_dash = false;
        }
    }
    out
}

fn is_low_value_last_turn(turn: &str) -> bool {
    let text = one_line(turn).to_lowercase();
    matches!(
        text.as_str(),
        "commit" | "push" | "commit push" | "커밋" | "푸시" | "커밋 푸시"
    ) || text.starts_with("<bash-")
        || (text.chars().count() < 16 && !text.contains('?') && !text.contains("해줘"))
}

fn push_kv(out: &mut String, key: &str, value: &str) {
    push_line(out, &format!("- **{key}:** {}", redact(value)));
}

fn push_blockquote(out: &mut String, text: &str) {
    let text = redact(text);
    if text.trim().is_empty() {
        push_line(out, "> _No content._");
        return;
    }
    for line in text.lines() {
        if line.trim().is_empty() {
            push_line(out, ">");
        } else {
            push_line(out, &format!("> {}", line));
        }
    }
}

fn push_fenced(out: &mut String, info: &str, text: &str) {
    push_line(out, &format!("```{info}"));
    push_line(out, &text.replace("```", "` ` `"));
    push_line(out, "```");
}

fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push('\n');
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn snippet(s: &str, max_chars: usize) -> String {
    let redacted = redact(s);
    trim_to(&redacted, max_chars)
}

fn trim_to(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(turns: Vec<&str>) -> Session {
        Session {
            agent: Agent::Codex,
            profile_id: "builtin-codex".to_string(),
            id: "01234567-89ab-cdef".to_string(),
            source_path: Some(PathBuf::from("/tmp/source.jsonl")),
            cwd: PathBuf::from("/tmp/demo"),
            folder: "demo".to_string(),
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: turns.into_iter().map(str::to_string).collect(),
            user_turn_timestamps_ms: Vec::new(),
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: Some("Demo Handoff".to_string()),
            title_fixed: true,
        }
    }

    #[test]
    fn renders_main_digest_with_work_link() {
        let s = session(vec!["처음 목표는 `src/main.rs`를 수정하는 거야."]);
        let turns = vec![HandoffTurn {
            user: s.user_turns[0].clone(),
            submitted_at_ms: None,
            final_answer: Some("수정했습니다.".to_string()),
            work_entries: Vec::new(),
        }];
        let work_paths = vec![PathBuf::from("/tmp/sample/work/turn-001.md")];
        let md = render_main_handoff(&s, &turns, &work_paths);

        assert!(md.contains("# Handoff Prompt"));
        assert!(md.contains("#### User"));
        assert!(md.contains("[work/turn-001.md](work/turn-001.md)"));
        assert!(md.contains("수정했습니다."));
    }

    #[test]
    fn renders_work_log() {
        let s = session(vec!["빌드해줘"]);
        let turn = HandoffTurn {
            user: "빌드해줘".to_string(),
            submitted_at_ms: None,
            final_answer: Some("빌드 성공".to_string()),
            work_entries: vec![WorkEntry {
                kind: WorkKind::ToolCall,
                text: "cargo build --release".to_string(),
            }],
        };
        let md = render_work_log(1, &turn, &s);

        assert!(md.contains("# Work Log - Turn 1"));
        assert!(md.contains("cargo build --release"));
        assert!(md.contains("빌드 성공"));
    }

    #[test]
    fn parses_antigravity_transcript_into_turns() {
        let root = std::env::temp_dir().join(format!(
            "s7s-agy-transcript-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let conv_dir = root.join("conversations");
        let logs_dir = root.join("brain/conv-1/.system_generated/logs");
        std::fs::create_dir_all(&conv_dir).expect("conversations dir");
        std::fs::create_dir_all(&logs_dir).expect("logs dir");
        // The .db file must exist so the context loader treats the source as present.
        std::fs::write(conv_dir.join("conv-1.db"), b"").expect("touch db");
        std::fs::write(
            logs_dir.join("transcript_full.jsonl"),
            concat!(
                r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"<USER_REQUEST>\nglab 레포 목록 조회해줘\n</USER_REQUEST>\n<ADDITIONAL_METADATA>time</ADDITIONAL_METADATA>"}"#, "\n",
                r#"{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","content":"목록을 조회하겠습니다.","tool_calls":[{"name":"run_command","args":{"CommandLine":"glab repo list"}}]}"#, "\n",
                r#"{"step_index":2,"source":"MODEL","type":"RUN_COMMAND","content":"Tool is running as a background task"}"#, "\n",
                r#"{"step_index":3,"source":"SYSTEM","type":"SYSTEM_MESSAGE","content":"system noise"}"#, "\n",
                r#"{"step_index":4,"source":"MODEL","type":"PLANNER_RESPONSE","content":"조회 결과 30개 레포가 있습니다."}"#, "\n",
                r#"{"step_index":5,"source":"MODEL","type":"PLANNER_RESPONSE","content":"","tool_calls":[{"name":"ask_question","args":{"questions":[{"question":"계속 진행할까요?"}]}}]}"#, "\n",
                r#"{"step_index":6,"source":"MODEL","type":"ASK_QUESTION","content":"Created At: t\nA1: User Skipped"}"#, "\n",
                r#"{"step_index":7,"source":"MODEL","type":"PLANNER_RESPONSE","content":"","tool_calls":[{"name":"ask_question","args":{"questions":[{"question":"배포할까요?"}]}}]}"#, "\n",
                r#"{"step_index":8,"source":"MODEL","type":"ASK_QUESTION","content":"Created At: t\nA1: 네 배포해주세요."}"#, "\n",
                r#"{"step_index":9,"source":"MODEL","type":"PLANNER_RESPONSE","content":"배포했습니다."}"#, "\n",
                r#"{"step_index":10,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"<USER_REQUEST>고마워 정리해줘</USER_REQUEST>"}"#, "\n",
                r#"{"step_index":11,"source":"MODEL","type":"PLANNER_RESPONSE","content":"정리했습니다."}"#, "\n",
            ),
        )
        .expect("write transcript");

        let mut s = session(vec!["glab 레포 목록 조회해줘"]);
        s.agent = Agent::Antigravity;
        s.id = "conv-1".to_string();
        s.source_path = Some(conv_dir.join("conv-1.db"));

        let turns = load_turns(&s);

        // Answered ask_question calls are promoted to virtual user turns (excluding skipped ones).
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].user, "glab 레포 목록 조회해줘");
        // The final answer is the last PLANNER_RESPONSE in the turn.
        assert_eq!(
            turns[0].final_answer.as_deref(),
            Some("조회 결과 30개 레포가 있습니다.")
        );
        // Action records: answer text + tool_calls + tool execution results.
        // Excludes SYSTEM_MESSAGE; skipped ASK_QUESTION turns remain as ToolResult.
        // The trailing assistant text equal to the final answer is stripped
        // (rendered once as Final Answer instead of twice).
        let kinds: Vec<WorkKind> = turns[0].work_entries.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                WorkKind::AssistantText,
                WorkKind::ToolCall,
                WorkKind::ToolResult,
                WorkKind::ToolCall,
                WorkKind::ToolResult,
                WorkKind::ToolCall,
            ]
        );
        assert!(turns[0].work_entries[1].text.contains("glab repo list"));
        assert!(turns[0].work_entries[2].text.starts_with("[RUN_COMMAND]"));
        assert!(turns[0].work_entries[4].text.contains("User Skipped"));

        // Promoted Q&A turn: formatted as `· Question -> Answer`, matching the list view.
        assert_eq!(turns[1].user, "· 배포할까요? → 네 배포해주세요.");
        assert_eq!(turns[1].final_answer.as_deref(), Some("배포했습니다."));

        assert_eq!(turns[2].user, "고마워 정리해줘");
        assert_eq!(turns[2].final_answer.as_deref(), Some("정리했습니다."));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn claude_question_answers_become_virtual_turns() {
        let root = std::env::temp_dir().join(format!(
            "s7s-claude-qa-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("temp dir");
        let path = root.join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"user","origin":{"kind":"human"},"message":{"role":"user","content":"버튼 색을 바꿔줘"}}"#, "\n",
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"어떤 색으로 바꿀까요?"}]}}"#, "\n",
                r#"{"type":"user","toolUseResult":{"questions":[{"question":"색상 선택?"}],"answers":{"색상 선택?":"파랑"}}}"#, "\n",
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"파랑으로 변경했습니다."}]}}"#, "\n",
            ),
        )
        .expect("write claude jsonl");

        let mut s = session(vec!["버튼 색을 바꿔줘"]);
        s.agent = Agent::Claude;
        s.source_path = Some(path);

        let turns = load_turns(&s);

        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user, "버튼 색을 바꿔줘");
        assert_eq!(
            turns[0].final_answer.as_deref(),
            Some("어떤 색으로 바꿀까요?")
        );
        // AskUserQuestion response is promoted to a virtual user turn, not a ToolResult.
        assert_eq!(turns[1].user, "· 색상 선택? → 파랑");
        assert_eq!(
            turns[1].final_answer.as_deref(),
            Some("파랑으로 변경했습니다.")
        );
        assert!(turns[1]
            .work_entries
            .iter()
            .all(|e| e.kind != WorkKind::ToolResult));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn claude_task_notification_stays_in_current_turn() {
        let root = std::env::temp_dir().join(format!(
            "s7s-claude-task-notif-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("temp dir");
        let path = root.join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"user","origin":{"kind":"human"},"message":{"role":"user","content":"백그라운드로 분석해줘"}}"#, "\n",
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"백그라운드 작업을 시작했습니다."}]}}"#, "\n",
                r#"{"type":"user","origin":{"kind":"task-notification"},"promptSource":"system","message":{"role":"user","content":"<task-notification>\n<task-id>abc123</task-id>\n<result>subagent findings</result>\n</task-notification>"}}"#, "\n",
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"분석 결과를 정리했습니다."}]}}"#, "\n",
            ),
        )
        .expect("write claude jsonl");

        let mut s = session(vec!["백그라운드로 분석해줘"]);
        s.agent = Agent::Claude;
        s.source_path = Some(path);

        let turns = load_turns(&s);

        // The notification must not open a new turn nor orphan the follow-up answer.
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user, "백그라운드로 분석해줘");
        assert_eq!(
            turns[0].final_answer.as_deref(),
            Some("분석 결과를 정리했습니다.")
        );
        // The notification body is kept as a ToolResult work entry (shown via `.`).
        assert!(turns[0]
            .work_entries
            .iter()
            .any(|e| e.kind == WorkKind::ToolResult && e.text.contains("<task-notification>")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn claude_final_answer_is_not_duplicated_as_work_entry() {
        // Regression: a plain text-only reply used to appear both as
        // "Assistant Text 1" and as "Final Answer" on the Detail screen.
        let root = std::env::temp_dir().join(format!(
            "s7s-claude-final-echo-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("temp dir");
        let path = root.join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"user","origin":{"kind":"human"},"message":{"role":"user","content":"안녕"}}"#, "\n",
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"안녕하세요. 무엇을 도와드릴까요?"}]}}"#, "\n",
            ),
        )
        .expect("write claude jsonl");

        let mut s = session(vec!["안녕"]);
        s.agent = Agent::Claude;
        s.source_path = Some(path);

        let turns = load_turns(&s);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].final_answer.as_deref(),
            Some("안녕하세요. 무엇을 도와드릴까요?")
        );
        assert!(turns[0].work_entries.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn antigravity_without_transcript_falls_back_to_user_turns() {
        let mut s = session(vec!["질문 하나"]);
        s.agent = Agent::Antigravity;
        s.id = "no-transcript".to_string();
        s.source_path = Some(PathBuf::from("/no/such/dir/conversations/no-transcript.db"));

        let turns = load_turns(&s);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user, "질문 하나");
        assert!(turns[0].final_answer.is_none());
        assert!(turns[0].work_entries.is_empty());
    }

    #[test]
    fn redacts_obvious_secrets() {
        let s = session(vec!["API_KEY=sk-abc1234567890"]);
        let turns = load_turns(&s);
        let md = render_work_log(1, &turns[0], &s);
        assert!(md.contains("[REDACTED]"));
        assert!(!md.contains("sk-abc"));
    }
}
