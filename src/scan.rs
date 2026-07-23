//! Scanner: incrementally collects sessions per profile, utilizing the cache.

use crate::cache::Cache;
use crate::config;
use crate::model::{Agent, Session};
use crate::parser;
use crate::profile::Profile;
use std::path::Path;
use walkdir::WalkDir;

/// Summary of scanning results.
pub struct ScanResult {
    pub sessions: Vec<Session>,
    pub scanned_files: usize,
    pub reparsed_files: usize,
}

/// Performs a full scan and persists the updated cache.
///
/// Iterates over session directories for each profile. Inactive profiles are also scanned
/// (as per spec to show all sessions). If `rebuild_cache` is `true`, it ignores the existing
/// cache completely and re-parses all files.
pub fn scan(profiles: &[Profile], rebuild_cache: bool) -> ScanResult {
    scan_at(profiles, rebuild_cache, &config::cache_path())
}

/// Same as `scan` but with an explicit cache file path. Split out so tests can
/// scan generated fixtures without touching the user's real cache.
pub(crate) fn scan_at(profiles: &[Profile], rebuild_cache: bool, cache_path: &Path) -> ScanResult {
    let old = if rebuild_cache {
        Cache::default()
    } else {
        Cache::load(cache_path)
    };
    let mut new = Cache::default();

    let mut sessions: Vec<Session> = Vec::new();
    let mut scanned = 0usize;
    let mut reparsed = 0usize;

    for profile in profiles {
        let before = sessions.len();
        match profile.agent {
            Agent::Claude => {
                let dir = profile.sessions_dir();
                // Title meta lives under the profile root (`<root>/sessions/*.json`),
                // not under the scanned `<root>/projects` tree.
                let claude_meta = parser::claude::load_title_meta(&profile.path);
                scan_jsonl_tree(
                    &dir,
                    &old,
                    &mut new,
                    &mut sessions,
                    &mut scanned,
                    &mut reparsed,
                    |path, mtime| {
                        let id = path.file_stem().and_then(|s| s.to_str());
                        let meta = id.and_then(|id| claude_meta.get(id));
                        parser::claude::parse_file(path, mtime, meta)
                    },
                    |_name| true,
                    |path, sessions| {
                        apply_claude_title_meta(path, sessions, &claude_meta);
                    },
                );
            }
            Agent::Codex => {
                let dir = profile.sessions_dir();
                let codex_meta = parser::codex::load_title_meta(&dir);
                scan_jsonl_tree(
                    &dir,
                    &old,
                    &mut new,
                    &mut sessions,
                    &mut scanned,
                    &mut reparsed,
                    |path, mtime| parser::codex::parse_file(path, mtime, Some(&codex_meta)),
                    |name| name.starts_with("rollout-"),
                    |_, sessions| apply_codex_title_meta(sessions, &codex_meta),
                );
            }
            Agent::Antigravity => scan_antigravity(
                &profile.path,
                &old,
                &mut new,
                &mut sessions,
                &mut scanned,
                &mut reparsed,
            ),
        }
        // The profile_id stored in the cache is untrusted and always reassigned to the current profile
        // (to prevent stale IDs when profiles are deleted or recreated).
        for s in &mut sessions[before..] {
            s.profile_id = profile.id.clone();
        }
    }

    // Sort by meaningful conversation activity, not physical storage writes
    // such as a resume-without-input `last-prompt` append.
    sessions.sort_by_key(|s| std::cmp::Reverse(s.updated_at_ms));

    let _ = new.save(cache_path);

    ScanResult {
        sessions,
        scanned_files: scanned,
        reparsed_files: reparsed,
    }
}

/// Recursively scans for *.jsonl files and parses them into one session per file (with cache applied).
#[allow(clippy::too_many_arguments)]
fn scan_jsonl_tree<P, F>(
    root: &Path,
    old: &Cache,
    new: &mut Cache,
    sessions: &mut Vec<Session>,
    scanned: &mut usize,
    reparsed: &mut usize,
    parse: P,
    name_filter: F,
    refresh_cached: impl Fn(&Path, &mut Vec<Session>),
) where
    P: Fn(&Path, i64) -> Option<Session>,
    F: Fn(&str) -> bool,
{
    if !root.exists() {
        return;
    }
    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.ends_with(".jsonl") || !name_filter(name) {
            continue;
        }
        *scanned += 1;
        let mtime = match file_mtime_ms(path) {
            Some(m) => m,
            None => continue,
        };
        let ctime = file_ctime_ms(path, mtime);
        let size = file_size_bytes(path);
        let key = path.to_string_lossy().to_string();

        if let Some(cached) = old.get_fresh(&key, mtime) {
            let mut cached = cached.clone();
            refresh_cached(path, &mut cached);
            set_ctime(&mut cached, ctime);
            set_size(&mut cached, size);
            sessions.extend(cached.iter().cloned());
            new.put(key, mtime, cached);
        } else {
            *reparsed += 1;
            let mut parsed: Vec<Session> = parse(path, mtime).into_iter().collect();
            refresh_cached(path, &mut parsed);
            set_ctime(&mut parsed, ctime);
            set_size(&mut parsed, size);
            sessions.extend(parsed.iter().cloned());
            new.put(key, mtime, parsed);
        }
    }
}

fn apply_claude_title_meta(
    path: &Path,
    sessions: &mut [Session],
    meta: &std::collections::HashMap<String, parser::claude::TitleMeta>,
) {
    let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
        return;
    };
    let Some(meta) = meta.get(id) else {
        return;
    };
    for session in sessions {
        // Body events (custom-title/agent-name/ai-title) are the authoritative
        // claude title source; the meta json only fills the gap, mirroring
        // parse_file's precedence. Every supported rename path also appends a
        // body event (see docs/session-title-compat.md), so cache-hit refreshes
        // must not let stale meta names clobber body-derived titles.
        if session.title_hint.is_none() {
            session.title_hint = meta.title.clone();
        }
        session.title_fixed = session.title_fixed || meta.fixed;
        parser::reindex_search_blob(session);
    }
}

fn apply_codex_title_meta(
    sessions: &mut [Session],
    meta: &std::collections::HashMap<String, parser::codex::TitleMeta>,
) {
    for session in sessions {
        if let Some(meta) = meta.get(&session.id) {
            session.title_hint = meta.title.clone().or_else(|| session.title_hint.clone());
            session.title_fixed = meta.title.is_some();
        }
        parser::reindex_search_blob(session);
    }
}

fn apply_antigravity_title_meta(
    sessions: &mut [Session],
    meta: &std::collections::HashMap<String, parser::antigravity::Meta>,
) {
    for session in sessions {
        if let Some(meta) = meta.get(&session.id) {
            if let Some(title) = meta.title.clone() {
                session.title_hint = Some(title);
                session.title_fixed = true;
            } else {
                session.title_hint = meta.preview.clone().or_else(|| session.title_hint.clone());
                session.title_fixed = false;
            }
        }
        parser::reindex_search_blob(session);
    }
}

fn scan_antigravity(
    cli_dir: &Path,
    old: &Cache,
    new: &mut Cache,
    sessions: &mut Vec<Session>,
    scanned: &mut usize,
    reparsed: &mut usize,
) {
    let conv_dir = parser::antigravity::conversations_dir(cli_dir);
    if !conv_dir.exists() {
        return;
    }
    // Load metadata only once for fallbacks (workspace, preview, timestamps).
    let meta = parser::antigravity::load_metadata(cli_dir);

    for entry in WalkDir::new(&conv_dir)
        .max_depth(1)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.ends_with(".db") {
            continue;
        }
        *scanned += 1;
        let db_mtime = match file_mtime_ms(path) {
            Some(m) => m,
            None => continue,
        };
        // Assistant answers are read from the JSONL transcript, a file separate from
        // the DB. Fold the transcript's mtime into the cache-freshness key only,
        // so a new answer reparses the session even when the DB is unchanged.
        // Display ordering still comes from semantic activity parsed from the records.
        let transcript_mtime = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|id| crate::session_context::antigravity::transcript_path(path, id))
            .and_then(|tp| file_mtime_ms(&tp))
            .unwrap_or(0);
        let freshness = db_mtime.max(transcript_mtime);
        let ctime = file_ctime_ms(path, db_mtime);
        let size = file_size_bytes(path);
        let key = path.to_string_lossy().to_string();

        if let Some(cached) = old.get_fresh(&key, freshness) {
            let mut cached = cached.clone();
            apply_antigravity_title_meta(&mut cached, &meta);
            set_ctime(&mut cached, ctime);
            set_size(&mut cached, size);
            sessions.extend(cached.iter().cloned());
            new.put(key, freshness, cached);
        } else {
            *reparsed += 1;
            let mut parsed: Vec<Session> = parser::antigravity::parse_db(path, db_mtime, &meta)
                .into_iter()
                .collect();
            apply_antigravity_title_meta(&mut parsed, &meta);
            set_ctime(&mut parsed, ctime);
            set_size(&mut parsed, size);
            sessions.extend(parsed.iter().cloned());
            new.put(key, freshness, parsed);
        }
    }
}

fn file_mtime_ms(path: &Path) -> Option<i64> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let dur = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(dur.as_millis() as i64)
}

/// Returns the file creation time (birth time) in epoch milliseconds, clamped to `mtime`:
/// a session cannot have been created after its last modification, and birth time cannot be
/// backdated the way mtime can (e.g. demo sandbox files), so mtime wins when it is earlier.
/// Falls back to `mtime` if the filesystem does not support birth time.
fn file_ctime_ms(path: &Path, mtime: i64) -> i64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|meta| meta.created().ok())
        .and_then(|created| created.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|dur| (dur.as_millis() as i64).min(mtime))
        .unwrap_or(mtime)
}

/// Returns the file size in bytes (0 if metadata is unavailable).
fn file_size_bytes(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Populates the creation time for all sessions generated from a single file.
fn set_ctime(sessions: &mut [Session], ctime: i64) {
    for s in sessions {
        s.ctime_ms = ctime.min(s.updated_at_ms);
    }
}

/// Populates the source file size for all sessions generated from a single file.
/// Applied on both cache hits and reparses so the value never requires a cache version bump.
fn set_size(sessions: &mut [Session], size: u64) {
    for s in sessions {
        s.size_bytes = size;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Agent, Session};
    use std::collections::HashMap;

    #[test]
    fn apply_codex_title_meta_updates_cached_sessions_by_session_id() {
        let mut sessions = vec![Session {
            agent: Agent::Codex,
            profile_id: String::new(),
            id: "019f36e8-9157-7c63-bee8-8937a6314982".to_string(),
            source_path: None,
            cwd: "/tmp/demo".into(),
            folder: "demo".to_string(),
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["첫 질문".to_string()],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: Some("첫 질문".to_string()),
            title_fixed: false,
        }];
        let mut meta = HashMap::new();
        meta.insert(
            "019f36e8-9157-7c63-bee8-8937a6314982".to_string(),
            parser::codex::TitleMeta {
                title: Some("26-07 세션 타이틀 개선".to_string()),
            },
        );

        apply_codex_title_meta(&mut sessions, &meta);

        assert_eq!(sessions[0].title(), "26-07 세션 타이틀 개선");
        assert!(sessions[0].title_fixed);
        // Enforce search matching by including renamed titles in the search blob, even if they are not in the body.
        assert!(sessions[0].search_blob.contains("타이틀"));
        assert!(sessions[0].search_blob.contains("첫 질문"));
    }

    fn claude_session(id: &str, title_hint: Option<&str>) -> Session {
        Session {
            agent: Agent::Claude,
            profile_id: String::new(),
            id: id.to_string(),
            source_path: None,
            cwd: "/tmp/demo".into(),
            folder: "demo".to_string(),
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["첫 질문".to_string()],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: title_hint.map(str::to_string),
            title_fixed: false,
        }
    }

    #[test]
    fn apply_claude_title_meta_fills_gap_but_never_overrides_body_title() {
        let mut meta = HashMap::new();
        meta.insert(
            "abc-123".to_string(),
            parser::claude::TitleMeta {
                title: Some("메타 제목".to_string()),
                fixed: true,
            },
        );
        let path = std::path::PathBuf::from("/tmp/x/abc-123.jsonl");

        // Cached body-derived title wins over the meta name.
        let mut sessions = vec![claude_session("abc-123", Some("본문 제목"))];
        apply_claude_title_meta(&path, &mut sessions, &meta);
        assert_eq!(sessions[0].title_hint.as_deref(), Some("본문 제목"));
        assert!(sessions[0].title_fixed);

        // Without a body title the meta name fills the gap (and enters the search blob).
        let mut sessions = vec![claude_session("abc-123", None)];
        apply_claude_title_meta(&path, &mut sessions, &meta);
        assert_eq!(sessions[0].title_hint.as_deref(), Some("메타 제목"));
        assert!(sessions[0].search_blob.contains("메타 제목"));
    }

    #[test]
    fn scan_loads_claude_title_meta_from_profile_root() {
        // Meta lives at `<root>/sessions/*.json` while bodies live under
        // `<root>/projects/...` — regression for passing the projects dir to
        // load_title_meta (which made the meta fallback unreachable).
        let root = std::env::temp_dir().join(format!("s7s-scan-title-meta-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project_dir = root.join("projects/-tmp-app");
        std::fs::create_dir_all(&project_dir).expect("create projects");
        std::fs::create_dir_all(root.join("sessions")).expect("create sessions");
        std::fs::write(
            project_dir.join("abc-123.jsonl"),
            "{\"uuid\":\"u1\",\"parentUuid\":null,\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"첫 질문\"},\"cwd\":\"/tmp/app\"}\n",
        )
        .expect("write body");
        std::fs::write(
            root.join("sessions/abc-123.json"),
            r#"{"sessionId":"abc-123","name":"메타 제목","nameSource":"custom"}"#,
        )
        .expect("write meta");

        let profiles = vec![crate::profile::Profile {
            id: "t".to_string(),
            agent: Agent::Claude,
            name: "t".to_string(),
            path: root.clone(),
            oauth_token: None,
            active: true,
            shortcut: None,
            builtin: false,
        }];
        let result = scan_at(&profiles, true, &root.join("index.bin"));
        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions[0].title(), "메타 제목");
        assert!(result.sessions[0].title_fixed);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_sorts_by_session_activity_instead_of_storage_mtime() {
        let root = std::env::temp_dir().join(format!("s7s-scan-activity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project_dir = root.join("projects/-tmp-app");
        std::fs::create_dir_all(&project_dir).expect("create projects");
        let older_activity = project_dir.join("older-activity.jsonl");
        let newer_activity = project_dir.join("newer-activity.jsonl");
        std::fs::write(
            &older_activity,
            "{\"timestamp\":\"2026-07-22T01:00:00Z\",\"uuid\":\"a\",\"parentUuid\":null,\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"older\"},\"cwd\":\"/tmp/app\"}\n",
        )
        .expect("write older activity");
        std::fs::write(
            &newer_activity,
            "{\"timestamp\":\"2026-07-23T01:00:00Z\",\"uuid\":\"b\",\"parentUuid\":null,\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"newer\"},\"cwd\":\"/tmp/app\"}\n",
        )
        .expect("write newer activity");

        let base = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_784_800_000);
        std::fs::File::open(&older_activity)
            .expect("open older activity")
            .set_modified(base + std::time::Duration::from_secs(100))
            .expect("make older activity file physically newer");
        std::fs::File::open(&newer_activity)
            .expect("open newer activity")
            .set_modified(base)
            .expect("make newer activity file physically older");

        let profiles = vec![crate::profile::Profile {
            id: "t".to_string(),
            agent: Agent::Claude,
            name: "t".to_string(),
            path: root.clone(),
            oauth_token: None,
            active: true,
            shortcut: None,
            builtin: false,
        }];
        let result = scan_at(&profiles, true, &root.join("index.bin"));
        assert_eq!(result.sessions.len(), 2);
        assert_eq!(result.sessions[0].id, "newer-activity");
        let first_source_mtime = file_mtime_ms(
            result.sessions[0]
                .source_path
                .as_deref()
                .expect("first source path"),
        )
        .expect("first source mtime");
        let second_source_mtime = file_mtime_ms(
            result.sessions[1]
                .source_path
                .as_deref()
                .expect("second source path"),
        )
        .expect("second source mtime");
        assert!(first_source_mtime < second_source_mtime);
        assert!(result.sessions[0].updated_at_ms > result.sessions[1].updated_at_ms);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_antigravity_title_meta_prefers_fixed_title_over_preview() {
        let mut sessions = vec![Session {
            agent: Agent::Antigravity,
            profile_id: String::new(),
            id: "8c456b4c-e7ba-46da-8c8a-9d37732e8e25".to_string(),
            source_path: None,
            cwd: "/tmp/demo".into(),
            folder: "demo".to_string(),
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["첫 질문".to_string()],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: Some("List GitLab Repository Commands".to_string()),
            title_fixed: false,
        }];
        let mut meta = HashMap::new();
        meta.insert(
            "8c456b4c-e7ba-46da-8c8a-9d37732e8e25".to_string(),
            parser::antigravity::Meta {
                title: Some("26-07 컨테이너 레지스트리 이전".to_string()),
                preview: Some("List GitLab Repository Commands".to_string()),
                workspace: None,
            },
        );

        apply_antigravity_title_meta(&mut sessions, &meta);

        assert_eq!(sessions[0].title(), "26-07 컨테이너 레지스트리 이전");
        assert!(sessions[0].title_fixed);
    }

    /// Manual before/after gate for parser refactoring (§11.4): dumps one line per
    /// session of the machine's real index — order, identity, title, Q count, and
    /// blob hashes — so two runs (baseline vs. refactored) can be diffed exactly.
    /// Uses a throwaway cache path with a forced rebuild, so every session is fully
    /// reparsed by the current code and the user's real cache is never touched.
    /// Run explicitly with:
    /// `cargo test real_data_index_snapshot -- --ignored --nocapture | grep '^S7S-IDX'`
    #[test]
    #[ignore]
    fn real_data_index_snapshot() {
        use std::hash::{Hash, Hasher};
        fn h(s: &str) -> u64 {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            s.hash(&mut hasher);
            hasher.finish()
        }

        let profiles = crate::profile::ProfileStore::load();
        let cache = std::env::temp_dir().join(format!("s7s-idx-snap-{}.bin", std::process::id()));
        let result = scan_at(&profiles.profiles, true, &cache);
        let _ = std::fs::remove_file(&cache);

        // Result order is meaningful (semantic activity desc): dump in order, no sorting.
        for s in &result.sessions {
            println!(
                "S7S-IDX\t{}\t{}\t{}\t{}\t{}\t{}\tQ{}\tsb{:016x}\tab{:016x}\t{}",
                s.agent.key(),
                s.profile_id,
                s.id,
                s.updated_at_ms,
                s.title(),
                s.title_fixed,
                s.user_turns.len(),
                h(&s.search_blob),
                h(&s.assistant_blob),
                s.folder,
            );
        }
        println!("S7S-IDX-TOTAL\t{}", result.sessions.len());
    }
}
