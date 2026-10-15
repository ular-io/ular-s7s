//! Scanner: incrementally collects sessions per profile, utilizing the cache.

use crate::cache::Cache;
use crate::config;
use crate::model::{Agent, Session};
use crate::parser;
use crate::profile::Profile;
use crate::session_workspace::WorkspaceStore;
use std::path::{Path, PathBuf};
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
    scan_at(
        profiles,
        rebuild_cache,
        &config::cache_path(),
        &config::session_workspaces_path(),
    )
}

/// Same as `scan` but with an explicit cache file path. Split out so tests can
/// scan generated fixtures without touching the user's real cache.
pub(crate) fn scan_at(
    profiles: &[Profile],
    rebuild_cache: bool,
    cache_path: &Path,
    workspace_path: &Path,
) -> ScanResult {
    let (result, cache) = collect_at(profiles, rebuild_cache, cache_path, workspace_path);
    let _ = cache.save(cache_path);
    result
}

/// Collects an index without publishing its cache. Background refreshes stage
/// the cache separately so a superseded scan cannot overwrite newer changes.
pub(crate) fn collect_at(
    profiles: &[Profile],
    rebuild_cache: bool,
    cache_path: &Path,
    workspace_path: &Path,
) -> (ScanResult, Cache) {
    let old = if rebuild_cache {
        Cache::default()
    } else {
        Cache::load(cache_path)
    };
    let mut new = Cache::default();

    let mut sessions: Vec<Session> = Vec::new();
    let mut scanned = 0usize;
    let mut reparsed = 0usize;
    let workspaces = WorkspaceStore::load(workspace_path);

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
                    None,
                    |path, sessions| {
                        apply_claude_title_meta(
                            path,
                            sessions,
                            &claude_meta,
                            &workspaces,
                            &profile.id,
                        );
                    },
                );
            }
            Agent::Codex => {
                let dir = profile.sessions_dir();
                let codex_meta = parser::codex::load_title_meta(&dir);
                let segments = parser::codex::segments::Segments::scan(&dir);
                scan_jsonl_tree(
                    &dir,
                    &old,
                    &mut new,
                    &mut sessions,
                    &mut scanned,
                    &mut reparsed,
                    |path, mtime| parser::codex::parse_file(path, mtime, Some(&codex_meta)),
                    |name| name.starts_with("rollout-"),
                    Some(&segments),
                    |_, sessions| {
                        apply_codex_title_meta(sessions, &codex_meta, &workspaces, &profile.id)
                    },
                );
            }
            Agent::Antigravity => scan_antigravity(
                profile,
                &workspaces,
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

    (
        ScanResult {
            sessions,
            scanned_files: scanned,
            reparsed_files: reparsed,
        },
        new,
    )
}

/// Scans JSONL (also JSONL.zst for Codex), applying cache and segment folding.
///
/// `segments` (codex only) folds a rewound thread's rollout files into one session: superseded
/// files are skipped, and the newest segment's cache freshness, creation time and size cover the
/// earlier files it reads its history from.
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
    segments: Option<&parser::codex::segments::Segments>,
    refresh_cached: impl Fn(&Path, &mut Vec<Session>),
) where
    P: Fn(&Path, i64) -> Option<Session>,
    F: Fn(&str) -> bool,
{
    if !root.exists() {
        return;
    }
    let mut seen = std::collections::HashSet::new();
    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        let supported = if segments.is_some() {
            parser::codex::rollout::logical_name(name).is_some()
        } else {
            name.ends_with(".jsonl")
        };
        if !supported || !name_filter(name) {
            continue;
        }
        let resolved;
        let path = if segments.is_some() {
            resolved = parser::codex::rollout::resolve_path(path);
            if !seen.insert(parser::codex::rollout::plain_path(&resolved)) {
                continue;
            }
            resolved.as_path()
        } else {
            path
        };
        if segments.is_some_and(|s| s.is_superseded(path)) {
            continue;
        }
        *scanned += 1;
        let mtime = match file_mtime_ms(path) {
            Some(m) => m,
            None => continue,
        };
        let earlier = segments.map(|s| s.earlier(path)).unwrap_or(&[]);
        let mtime = earlier
            .iter()
            .filter_map(|p| file_mtime_ms(p))
            .fold(mtime, i64::max);
        let ctime = file_ctime_ms(earlier.first().map_or(path, PathBuf::as_path), mtime);
        let size = file_size_bytes(path) + earlier.iter().map(|p| file_size_bytes(p)).sum::<u64>();
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

/// Applies the s7s-owned folder override (`session_workspaces.json`) to one session.
///
/// Claude and Codex resume in whatever directory s7s launches them from — the
/// folder stored in the transcript is only where the session started — so a
/// recorded override is the session's real folder and wins.
///
/// Antigravity is the exception. A resumed conversation keeps working in the
/// folder captured when it was created; launching it elsewhere changes the
/// header but not the working directory (verified against agy 1.2.8). Its own
/// value therefore always wins, and the override only fills the gap left by
/// handoffs, which record no folder at all. `Change Folder` is refused for
/// Antigravity sessions for the same reason (see docs/session-folder.md).
fn apply_workspace_cwd(session: &mut Session, workspaces: &WorkspaceStore, profile_id: &str) {
    if session.agent == Agent::Antigravity && !session.cwd.as_os_str().is_empty() {
        return;
    }
    let Some(cwd) = workspaces.cwd(profile_id, &session.id) else {
        return;
    };
    session.cwd = cwd.to_path_buf();
    session.folder = cwd
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.to_string_lossy().to_string());
}

fn apply_claude_title_meta(
    path: &Path,
    sessions: &mut [Session],
    meta: &std::collections::HashMap<String, parser::claude::TitleMeta>,
    workspaces: &WorkspaceStore,
    profile_id: &str,
) {
    // A missing registry entry is the normal case, not a reason to skip: claude
    // 2.1.263 keys `sessions/` by process id, so a stored session usually has no
    // entry at all. Only the title fill-in below is conditional; the search blob
    // is always recomputed, or a body-derived title would never reach the index
    // and `session search` could not find a renamed session by its title.
    let meta = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|id| meta.get(id));
    for session in sessions {
        // Body events (custom-title/agent-name/ai-title) are the authoritative
        // claude title source; the meta json only fills the gap, mirroring
        // parse_file's precedence. Every supported rename path also appends a
        // body event (see docs/session-title-compat.md), so cache-hit refreshes
        // must not let stale meta names clobber body-derived titles.
        if let Some(meta) = meta {
            if session.title_hint.is_none() {
                session.title_hint = meta.title.clone();
            }
            session.title_fixed = session.title_fixed || meta.fixed;
        }
        apply_workspace_cwd(session, workspaces, profile_id);
        parser::reindex_search_blob(session);
    }
}

fn apply_codex_title_meta(
    sessions: &mut [Session],
    meta: &std::collections::HashMap<String, parser::codex::TitleMeta>,
    workspaces: &WorkspaceStore,
    profile_id: &str,
) {
    for session in sessions {
        if let Some(meta) = meta.get(&session.id) {
            session.title_hint = meta.title.clone().or_else(|| session.title_hint.clone());
            session.title_fixed = meta.title.is_some();
        }
        apply_workspace_cwd(session, workspaces, profile_id);
        parser::reindex_search_blob(session);
    }
}

fn apply_antigravity_scan_meta(
    sessions: &mut [Session],
    meta: &std::collections::HashMap<String, parser::antigravity::Meta>,
    workspaces: &WorkspaceStore,
    profile_id: &str,
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
        apply_workspace_cwd(session, workspaces, profile_id);
        parser::reindex_search_blob(session);
    }
}

fn scan_antigravity(
    profile: &Profile,
    workspaces: &WorkspaceStore,
    old: &Cache,
    new: &mut Cache,
    sessions: &mut Vec<Session>,
    scanned: &mut usize,
    reparsed: &mut usize,
) {
    let cli_dir = &profile.path;
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
            apply_antigravity_scan_meta(&mut cached, &meta, workspaces, &profile.id);
            set_ctime(&mut cached, ctime);
            set_size(&mut cached, size);
            sessions.extend(cached.iter().cloned());
            new.put(key, freshness, cached);
        } else {
            *reparsed += 1;
            let mut parsed: Vec<Session> = parser::antigravity::parse_db(path, db_mtime, &meta)
                .into_iter()
                .collect();
            apply_antigravity_scan_meta(&mut parsed, &meta, workspaces, &profile.id);
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
mod tests;
