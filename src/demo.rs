//! Demo sandbox data generation module.
//!
//! `s7s demo` builds a disposable mock dataset (English Claude/Codex/Antigravity
//! sessions) under `config::demo_root()` so every TUI screen can be captured
//! without exposing real session data. The parsers and the scanner stay fully
//! demo-agnostic: generated files use the real storage formats and carry real
//! file mtimes (backdated at generation time via `File::set_modified`), so no
//! demo branches exist outside this module and `config.rs` path redirects.

use crate::model::Agent;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Gap between consecutive demo sessions (~2.7 days), spreading the 33 sessions
/// over roughly the past 3 months.
const SESSION_SPACING_MS: i64 = 233_280_000;
/// Loop-generated sessions (idx 1..=LOOP_SESSIONS) in addition to the detailed session 0.
const LOOP_SESSIONS: usize = 32;
/// Fictional development root used as the session cwd inside mock data.
const DEV_ROOT: &str = "/Users/ular-io/dev";
/// Spacing between consecutive turns inside one session.
const TURN_SPACING_MS: i64 = 60_000;

/// Title of the detailed first-row session (also asserted by regression tests).
const SESSION0_TITLE: &str = "Fullstack JWT authentication flow integration";

fn pb_varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

/// Protobuf helper: length-delimited (wire type 2) field.
fn pb_len_field(field: u64, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    pb_varint(field << 3 | 2, &mut out);
    pb_varint(bytes.len() as u64, &mut out);
    out.extend_from_slice(bytes);
    out
}

/// Protobuf helper: varint (wire type 0) field.
fn pb_varint_field(field: u64, val: u64) -> Vec<u8> {
    let mut out = Vec::new();
    pb_varint(field << 3, &mut out);
    pb_varint(val, &mut out);
    out
}

/// Serializes an Antigravity step payload into the real protobuf shape so the
/// parser needs no special handling for demo databases.
fn encode_antigravity_step(msg: &str, folder: &str, ts_ms: i64) -> Vec<u8> {
    // User message: 19.2 (string)
    let msg_encoded = pb_len_field(2, msg.as_bytes());

    // Workspace: 19.12.12 (file:// URI)
    let ws_uri = format!("file://{}/{}", DEV_ROOT, folder);
    let ws_uri_field = pb_len_field(12, ws_uri.as_bytes());
    let f19_12_field = pb_len_field(12, &ws_uri_field);

    let mut f19_inner = msg_encoded;
    f19_inner.extend(f19_12_field);
    let f19_field = pb_len_field(19, &f19_inner);

    // Timestamp: 5.1.1 (seconds) + 5.1.2 (nanos)
    let secs = ts_ms / 1000;
    let nanos = (ts_ms % 1000) * 1_000_000;
    let mut f5_1_inner = pb_varint_field(1, secs as u64);
    f5_1_inner.extend(pb_varint_field(2, nanos as u64));
    let f5_1_field = pb_len_field(1, &f5_1_inner);
    let f5_field = pb_len_field(5, &f5_1_field);

    let mut step_payload = f19_field;
    step_payload.extend(f5_field);
    step_payload
}

/// Current epoch in milliseconds; demo timestamps are derived from this so the
/// sessions always span "today back to ~3 months ago" regardless of when demo runs.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Last-modified timestamp of the demo session at `idx` (idx 0 = newest).
fn session_mtime_ms(now_ms: i64, idx: usize) -> i64 {
    now_ms - idx as i64 * SESSION_SPACING_MS
}

/// Backdates the real file mtime so the scanner reads plausible session dates
/// without any demo-specific time handling.
fn set_file_mtime(path: &Path, ts_ms: i64) -> anyhow::Result<()> {
    let time = UNIX_EPOCH + Duration::from_millis(ts_ms.max(0) as u64);
    let file = std::fs::OpenOptions::new().write(true).open(path)?;
    file.set_modified(time)?;
    Ok(())
}

/// Claude stores sessions under `projects/<munged-cwd>/`, the cwd with `/` mapped to `-`.
fn claude_project_dir(cwd: &str) -> String {
    cwd.replace('/', "-")
}

fn session_cwd(folder: &str) -> String {
    format!("{}/{}", DEV_ROOT, folder)
}

/// Distinct professional development topics; index 0 doubles as session 0's title.
const TOPICS: [&str; 33] = [
    SESSION0_TITLE,
    "Optimize PostgreSQL index structure for query performance",
    "Refactor Dockerfile to use multi-stage builds for smaller image size",
    "Resolve memory leak in socket connection pool",
    "Implement OAuth2 login flow with GitHub API",
    "Add rate limiting middleware to API endpoints",
    "Setup unit tests for state management react store",
    "Configure SSL certificate automatic renewal via Let's Encrypt",
    "Fix race condition in async background worker queue",
    "Migrate database schema to support dynamic column definitions",
    "Refactor Rust error handling using custom enum types",
    "Implement Redis caching layer for slow search endpoints",
    "Setup Terraform state management with Azure backend",
    "Fix cross-origin resource sharing (CORS) header configuration",
    "Upgrade Express server to Fastify framework",
    "Configure Prometheus and Grafana for system CPU/Memory monitoring",
    "Refactor React components to use functional style with Hooks",
    "Fix CSS layout issue causing text overflow on mobile screens",
    "Implement file upload API with S3 storage backend",
    "Add localized translation support (i18n) to web UI",
    "Optimize Webpack build performance and chunk splits",
    "Fix dead-lock condition in multithreaded file downloader",
    "Configure Nginx reverse proxy load balancer configs",
    "Setup automated database backup cron script",
    "Refactor API responses to follow JSON:API standard specification",
    "Optimize search queries using Elasticsearch fuzzy matching",
    "Fix cookie domain configuration for subdomain sharing",
    "Configure Sentry integration for frontend crash monitoring",
    "Refactor code to extract shared helper utilities package",
    "Resolve memory bloat in large JSON data exporter",
    "Implement WebHook notifications endpoint for payment success",
    "Fix memory leak on WebSockets client disconnection cleanup",
    "Configure automated semantic versioning and release notes action",
];

/// Synthesizes a natural English question for the given topic and step.
fn synth_question(topic: &str, step: usize) -> String {
    match step {
        0 => format!("Implement the features for {}.", topic),
        1 => format!("Let's debug the current issue in {} configuration.", topic),
        2 => format!(
            "Can you write a comprehensive unit test to verify {}?",
            topic
        ),
        3 => format!(
            "Is there any security risk or memory leak within {} implementation?",
            topic
        ),
        4 => format!("Refactor this module to optimize performance of {}.", topic),
        5 => format!(
            "How should we handle potential boundary exceptions for {}?",
            topic
        ),
        6 => format!(
            "Write a shell script to automate deployment validation of {}.",
            topic
        ),
        7 => format!(
            "Analyze the heap dump to track down memory usage for {}.",
            topic
        ),
        8 => format!(
            "Is there a better software pattern for this {} layout?",
            topic
        ),
        9 => format!("Configure production environment variables for {}.", topic),
        _ => format!("Verify step {} of the {} task list.", step + 1, topic),
    }
}

/// Synthesizes a matching assistant answer for the given topic and step.
fn synth_answer(topic: &str, step: usize) -> String {
    match step {
        0 => format!(
            "I will start by installing dependencies and initializing the boilerplate code for {}.",
            topic
        ),
        1 => format!(
            "The compiler errors are now resolved. The {} setup is clean and active.",
            topic
        ),
        2 => format!(
            "Certainly. I've created the mock specs under the test suite verifying {} behavior.",
            topic
        ),
        3 => format!(
            "Ensure to sanitize parameters and check buffers to prevent leaks in {}.",
            topic
        ),
        4 => format!(
            "We can refactor by lazy loading components to speed up {} rendering.",
            topic
        ),
        5 => format!(
            "I've added retry logic and default fallbacks to stabilize {} errors.",
            topic
        ),
        6 => format!(
            "Here is the bash script testing all endpoint responses for {} configurations.",
            topic
        ),
        7 => format!(
            "GC stats look stable and no persistent object leaks were found for {}.",
            topic
        ),
        8 => format!(
            "Using a factory pattern or pub-sub broker would streamline the {} calls.",
            topic
        ),
        9 => format!(
            "Setting standard memory limits and connection sizes suitable for {} scaling.",
            topic
        ),
        _ => format!(
            "Successfully finished step {} of {} development.",
            step + 1,
            topic
        ),
    }
}

/// Cleans old sandbox data and populates fresh mock sessions under `base`.
///
/// Regeneration policy: `sessions/` and `cache/` are wiped and rebuilt every run
/// (deterministic dataset, no stale leftovers), and the config files owned by the
/// generator (`config.toml`, `profiles.json`) are always overwritten. Other
/// app-owned state in `config/` (theme.json, histories, models.json) is preserved
/// so demo-mode UI choices survive relaunches.
pub fn ensure_demo_sandbox(base: &Path) -> anyhow::Result<()> {
    let now = now_ms();

    // 0. Wipe regenerated trees to avoid stale sessions and cache artifacts.
    let _ = std::fs::remove_dir_all(base.join("sessions"));
    let _ = std::fs::remove_dir_all(base.join("cache"));

    // 1. Create directory structures.
    std::fs::create_dir_all(base.join("config"))?;
    std::fs::create_dir_all(base.join("cache"))?;
    std::fs::create_dir_all(base.join("sessions/claude/projects"))?;
    std::fs::create_dir_all(base.join("sessions/claude/sessions"))?;
    std::fs::create_dir_all(base.join("sessions/codex/sessions"))?;
    std::fs::create_dir_all(base.join("sessions/antigravity/conversations"))?;
    std::fs::create_dir_all(base.join("sessions/antigravity/cache"))?;

    // 2. Write example config files (always overwritten: generator-owned).
    std::fs::write(
        base.join("config/config.toml"),
        crate::config::CONFIG_TEMPLATE,
    )?;
    write_profiles_json(base)?;

    // 3. Detailed Claude session 0 (10 turns, first table row).
    write_detailed_claude_session(base, now)?;

    // 4. Loop-generated mock sessions (idx 1..=LOOP_SESSIONS).
    let mut codex_index_lines = Vec::new();
    let mut antigravity_convs = serde_json::Map::new();

    for idx in 1..=LOOP_SESSIONS {
        let topic = TOPICS[idx % TOPICS.len()];
        let agent = match idx % 3 {
            0 => Agent::Claude,
            1 => Agent::Codex,
            _ => Agent::Antigravity,
        };
        let folder = match idx % 4 {
            0 => "nextjs-auth-app",
            1 => "cargo-backend",
            2 => "kube-deploy",
            _ => "frontend-core",
        };
        let q_count = 3 + (idx % 28); // 3 to 30
        let mtime = session_mtime_ms(now, idx);

        match agent {
            Agent::Claude => write_claude_session(base, idx, topic, folder, q_count, mtime)?,
            Agent::Codex => {
                let index_line = write_codex_session(base, idx, topic, folder, q_count, mtime)?;
                codex_index_lines.push(index_line);
            }
            Agent::Antigravity => {
                let (id, meta) =
                    write_antigravity_session(base, idx, topic, folder, q_count, mtime)?;
                antigravity_convs.insert(id, meta);
            }
        }
    }

    // 5. Global Codex index file.
    std::fs::write(
        base.join("sessions/codex/session_index.jsonl"),
        codex_index_lines.join("\n") + "\n",
    )?;

    // 6. Global Antigravity metadata file.
    let antigravity_metadata_json = serde_json::json!({
        "conversations": antigravity_convs
    });
    std::fs::write(
        base.join("sessions/antigravity/cache/conversation_metadata.json"),
        serde_json::to_string_pretty(&antigravity_metadata_json)?,
    )?;

    Ok(())
}

fn write_profiles_json(base: &Path) -> anyhow::Result<()> {
    let profile = |id: &str, agent: &str, name: &str, dir: &str, shortcut: u8| {
        serde_json::json!({
            "id": id,
            "agent": agent,
            "name": name,
            "path": base.join("sessions").join(dir),
            "active": true,
            "shortcut": shortcut,
            "builtin": true
        })
    };
    let profiles_json = serde_json::json!({
        "version": 1,
        "profiles": [
            profile("builtin-claude", "Claude", "Claude", "claude", 1),
            profile("builtin-codex", "Codex", "Codex", "codex", 2),
            profile("builtin-antigravity", "Antigravity", "Antigravity", "antigravity", 3),
        ]
    });
    std::fs::write(
        base.join("config/profiles.json"),
        serde_json::to_string_pretty(&profiles_json)?,
    )?;
    Ok(())
}

/// Path of a claude session jsonl under the real `projects/<munged-cwd>/` layout.
fn claude_session_path(base: &Path, folder: &str, session_id: &str) -> anyhow::Result<PathBuf> {
    let dir = base
        .join("sessions/claude/projects")
        .join(claude_project_dir(&session_cwd(folder)));
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("{}.jsonl", session_id)))
}

/// Title sources mirror a real `/rename`: the CLI appends a `custom-title` body
/// event to the jsonl (the authoritative source the parser reads) and also keeps
/// a `sessions/<id>.json` meta file, so both are generated here.
fn claude_title_event(session_id: &str, title: &str) -> String {
    format!(
        "{{\"type\":\"custom-title\",\"customTitle\":\"{}\",\"sessionId\":\"{}\"}}\n",
        title, session_id
    )
}

fn write_claude_title_meta(base: &Path, session_id: &str, title: &str) -> anyhow::Result<()> {
    let meta = serde_json::json!({
        "sessionId": session_id,
        "name": title,
        "nameSource": "user"
    });
    std::fs::write(
        base.join(format!("sessions/claude/sessions/{}.json", session_id)),
        serde_json::to_string_pretty(&meta)?,
    )?;
    Ok(())
}

/// Detailed 10-turn Claude session shown as the first table row.
fn write_detailed_claude_session(base: &Path, now_ms: i64) -> anyhow::Result<()> {
    let session_id = "demo-session-0";
    let jsonl = r#"{"uuid":"c1","parentUuid":null,"type":"user","message":{"role":"user","content":[{"type":"text","text":"Setup Next.js 14 project and create custom Edge middleware for JWT authorization."}]},"cwd":"/Users/ular-io/dev/nextjs-auth-app"}
{"uuid":"c2","parentUuid":"c1","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"First, let's install the Edge-compatible `jose` library since standard `jsonwebtoken` uses Node APIs."},{"type":"tool_use","id":"call_1","name":"run_command","input":{"command":"npm install jose"}}]}}
{"uuid":"c3","parentUuid":"c2","type":"user","toolUseResult":{"status":"success","content":"+ jose@5.2.0\nadded 1 package, and audited 120 packages in 1s"}}
{"uuid":"c4","parentUuid":"c3","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Great. Now let's implement the core middleware code verifying JWT tokens from request headers."}]}}
{"uuid":"c5","parentUuid":"c4","type":"user","message":{"role":"user","content":[{"type":"text","text":"Create a registration endpoint `/api/auth/register` validating inputs with Zod."}]},"cwd":"/Users/ular-io/dev/nextjs-auth-app"}
{"uuid":"c6","parentUuid":"c5","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I will write the user registration route and perform Zod parsing. Let's create `src/app/api/auth/register/route.ts`."}]}}
{"uuid":"c7","parentUuid":"c6","type":"user","message":{"role":"user","content":[{"type":"text","text":"Add password hashing via bcryptjs library."}]},"cwd":"/Users/ular-io/dev/nextjs-auth-app"}
{"uuid":"c8","parentUuid":"c7","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Installing `bcryptjs` and writing salt-generation helper modules."},{"type":"tool_use","id":"call_2","name":"run_command","input":{"command":"npm install bcryptjs @types/bcryptjs"}}]}}
{"uuid":"c9","parentUuid":"c8","type":"user","toolUseResult":{"status":"success","content":"+ bcryptjs@2.4.3\nadded 2 packages."}}
{"uuid":"c10","parentUuid":"c9","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"helpers created. Password hashing is now integrated with the database storage logic."}]}}
{"uuid":"c11","parentUuid":"c10","type":"user","message":{"role":"user","content":[{"type":"text","text":"Now implement `/api/auth/login` endpoint that generates JWT and sets HTTP-only cookie."}]},"cwd":"/Users/ular-io/dev/nextjs-auth-app"}
{"uuid":"c12","parentUuid":"c11","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Here is the login controller setting the HTTP-only cookie using the Next.js `cookies()` helper."}]}}
{"uuid":"c13","parentUuid":"c12","type":"user","message":{"role":"user","content":[{"type":"text","text":"Write unit tests using Jest to verify the JWT generation and middleware routing guards."}]},"cwd":"/Users/ular-io/dev/nextjs-auth-app"}
{"uuid":"c14","parentUuid":"c13","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Let's configure Jest environment settings."},{"type":"tool_use","id":"call_3","name":"view_file","input":{"path":"jest.config.js"}}]}}
{"uuid":"c15","parentUuid":"c14","type":"user","toolUseResult":{"status":"success","content":"module.exports = { testEnvironment: 'node' };"}}
{"uuid":"c16","parentUuid":"c15","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Jest tests written in `src/__tests__/auth.test.ts`. Verified login and token expiration scenarios."}]}}
{"uuid":"c17","parentUuid":"c16","type":"user","message":{"role":"user","content":[{"type":"text","text":"Implement robust error-catching wrapper for API routes to avoid exposing server stacktraces."}]},"cwd":"/Users/ular-io/dev/nextjs-auth-app"}
{"uuid":"c18","parentUuid":"c17","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Creating a global `withErrorHandler` Higher-Order Function to format clean HTTP error messages."}]}}
{"uuid":"c19","parentUuid":"c18","type":"user","message":{"role":"user","content":[{"type":"text","text":"Add validation logic to parse env variables at startup and fail fast if missing."}]},"cwd":"/Users/ular-io/dev/nextjs-auth-app"}
{"uuid":"c20","parentUuid":"c19","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Writing `src/lib/env.ts` checking `DATABASE_URL` and `JWT_SECRET` variables."}]}}
{"uuid":"c21","parentUuid":"c20","type":"user","message":{"role":"user","content":[{"type":"text","text":"Configure Redis integration to black-list logged-out JWT tokens."}]},"cwd":"/Users/ular-io/dev/nextjs-auth-app"}
{"uuid":"c22","parentUuid":"c21","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Setting up key-value expiration logic with `ioredis` to manage the blacklist."}]}}
{"uuid":"c23","parentUuid":"c22","type":"user","message":{"role":"user","content":[{"type":"text","text":"Create GitHub Actions workflow YAML running Jest tests on every PR."}]},"cwd":"/Users/ular-io/dev/nextjs-auth-app"}
{"uuid":"c24","parentUuid":"c23","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Creating `.github/workflows/ci.yml` pipeline with node-cache steps."}]}}
{"uuid":"c25","parentUuid":"c24","type":"user","message":{"role":"user","content":[{"type":"text","text":"Optimize Pg connection pool options for optimal throughput."}]},"cwd":"/Users/ular-io/dev/nextjs-auth-app"}
{"uuid":"c26","parentUuid":"c25","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Configured dynamic connection limits based on host CPU core availability."}]}}
"#;
    let path = claude_session_path(base, "nextjs-auth-app", session_id)?;
    let content = claude_title_event(session_id, SESSION0_TITLE) + jsonl;
    std::fs::write(&path, content)?;
    set_file_mtime(&path, session_mtime_ms(now_ms, 0))?;
    write_claude_title_meta(base, session_id, SESSION0_TITLE)
}

fn write_claude_session(
    base: &Path,
    idx: usize,
    topic: &str,
    folder: &str,
    q_count: usize,
    mtime: i64,
) -> anyhow::Result<()> {
    let session_id = format!("demo-session-{}", idx);
    let cwd = session_cwd(folder);
    let mut jsonl = claude_title_event(&session_id, topic);

    for q in 0..q_count {
        let uuid = format!("u-{}-{}", idx, q);
        let parent = if q == 0 {
            "null".to_string()
        } else {
            format!("\"u-{}-{}\"", idx, q - 1)
        };

        jsonl.push_str(&format!(
            "{{\"uuid\":\"{}\",\"parentUuid\":{},\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}},\"cwd\":\"{}\"}}\n",
            uuid, parent, synth_question(topic, q), cwd
        ));

        let a_uuid = format!("a-{}-{}", idx, q);
        jsonl.push_str(&format!(
            "{{\"uuid\":\"{}\",\"parentUuid\":\"{}\",\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}}}}\n",
            a_uuid, uuid, synth_answer(topic, q)
        ));
    }

    let path = claude_session_path(base, folder, &session_id)?;
    std::fs::write(&path, jsonl)?;
    set_file_mtime(&path, mtime)?;
    write_claude_title_meta(base, &session_id, topic)
}

/// Writes one Codex rollout file and returns its session_index.jsonl line.
fn write_codex_session(
    base: &Path,
    idx: usize,
    topic: &str,
    folder: &str,
    q_count: usize,
    mtime: i64,
) -> anyhow::Result<String> {
    let session_id = format!("demo-session-{}", idx);
    let cwd = session_cwd(folder);
    let mut jsonl = String::new();

    jsonl.push_str(&format!(
        "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{}\",\"cwd\":\"{}\"}}}}\n",
        session_id, cwd
    ));

    for q in 0..q_count {
        jsonl.push_str(&format!(
            "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"{}\"}}}}\n",
            synth_question(topic, q)
        ));
        jsonl.push_str(&format!(
            "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"agent_message\",\"message\":\"{}\"}}}}\n",
            synth_answer(topic, q)
        ));
    }

    let path = base.join(format!(
        "sessions/codex/sessions/rollout-{}.jsonl",
        session_id
    ));
    std::fs::write(&path, jsonl)?;
    set_file_mtime(&path, mtime)?;

    Ok(format!(
        "{{\"id\":\"{}\",\"thread_name\":\"{}\"}}",
        session_id, topic
    ))
}

/// Writes one Antigravity conversation DB (+ brain transcript) and returns its
/// (session_id, conversation_metadata.json entry) pair.
fn write_antigravity_session(
    base: &Path,
    idx: usize,
    topic: &str,
    folder: &str,
    q_count: usize,
    mtime: i64,
) -> anyhow::Result<(String, serde_json::Value)> {
    let session_id = format!("demo-session-{}", idx);
    let cwd = session_cwd(folder);

    let db_path = base.join(format!(
        "sessions/antigravity/conversations/{}.db",
        session_id
    ));
    let conn = rusqlite::Connection::open(&db_path)?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS steps (
            idx INTEGER PRIMARY KEY,
            step_type INTEGER,
            step_payload BLOB
        )",
        [],
    )?;

    // Step timestamps lead up to the session mtime so no payload time exceeds it.
    for q in 0..q_count {
        let ts_ms = mtime - (q_count - 1 - q) as i64 * TURN_SPACING_MS;
        let payload = encode_antigravity_step(&synth_question(topic, q), folder, ts_ms);
        conn.execute(
            "INSERT INTO steps (idx, step_type, step_payload) VALUES (?, ?, ?)",
            rusqlite::params![q, 14, payload],
        )?;
    }
    drop(conn);
    set_file_mtime(&db_path, mtime)?;

    use chrono::{TimeZone, Utc};
    let utc_time = Utc.timestamp_opt(mtime / 1000, 0).unwrap();
    let last_modified_str = utc_time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let conv_meta = serde_json::json!({
        "last_modified_time": last_modified_str,
        "summary": {
            "Title": topic,
            "Preview": format!("Checking configurations for {}.", topic),
            "WorkspaceURIs": [format!("file://{}", cwd)]
        }
    });

    // Brain transcript logs backing the Detail screen.
    let mut transcript = String::new();
    let logs_dir = base.join(format!(
        "sessions/antigravity/brain/{}/.system_generated/logs",
        session_id
    ));
    std::fs::create_dir_all(&logs_dir)?;

    for q in 0..q_count {
        transcript.push_str(&format!(
            "{{\"source\":\"USER_EXPLICIT\",\"type\":\"USER_INPUT\",\"content\":\"<USER_REQUEST>\\n{}\\n</USER_REQUEST>\"}}\n",
            synth_question(topic, q)
        ));
        transcript.push_str(&format!(
            "{{\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"content\":\"{}\"}}\n",
            synth_answer(topic, q)
        ));
    }
    std::fs::write(logs_dir.join("transcript.jsonl"), transcript)?;

    Ok((session_id, conv_meta))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Agent;
    use crate::profile::Profile;
    use std::collections::{HashMap, HashSet};

    fn temp_base(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("s7s-demo-test-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn demo_profiles(base: &Path) -> Vec<Profile> {
        [
            (Agent::Claude, "claude"),
            (Agent::Codex, "codex"),
            (Agent::Antigravity, "antigravity"),
        ]
        .into_iter()
        .map(|(agent, dir)| Profile {
            id: format!("demo-{}", dir),
            agent,
            name: dir.to_string(),
            path: base.join("sessions").join(dir),
            oauth_token: None,
            active: true,
            shortcut: None,
            builtin: false,
        })
        .collect()
    }

    fn scan_sandbox(base: &Path) -> Vec<crate::model::Session> {
        let profiles = demo_profiles(base);
        crate::scan::scan_at(&profiles, true, &base.join("cache/index.bin")).sessions
    }

    fn session_idx(id: &str) -> Option<usize> {
        id.strip_prefix("demo-session-")
            .and_then(|s| s.parse().ok())
    }

    #[test]
    fn sandbox_scan_parity() {
        let base = temp_base("parity");
        ensure_demo_sandbox(&base).unwrap();
        let sessions = scan_sandbox(&base);

        // 33 sessions total, sorted newest-first == by idx ascending.
        assert_eq!(sessions.len(), 1 + LOOP_SESSIONS);
        for (pos, s) in sessions.iter().enumerate() {
            assert_eq!(
                session_idx(&s.id),
                Some(pos),
                "sort order broken at {}",
                s.id
            );
        }

        // Detailed first row: Claude, 10 questions, fixed title.
        let first = &sessions[0];
        assert_eq!(first.agent, Agent::Claude);
        assert_eq!(first.user_turns.len(), 10);
        assert_eq!(first.title_hint.as_deref(), Some(SESSION0_TITLE));

        // Q-count formula, agent mix, folder variety, unique titles.
        let mut agents: HashMap<Agent, usize> = HashMap::new();
        let mut folders: HashSet<String> = HashSet::new();
        let mut titles: HashSet<String> = HashSet::new();
        for s in &sessions {
            let idx = session_idx(&s.id).unwrap();
            if idx >= 1 {
                assert_eq!(
                    s.user_turns.len(),
                    3 + idx % 28,
                    "q count mismatch for {} ({:?})",
                    s.id,
                    s.agent
                );
            }
            *agents.entry(s.agent).or_insert(0) += 1;
            folders.insert(s.folder.clone());
            assert!(
                titles.insert(s.title_hint.clone().unwrap_or_default()),
                "duplicate title in {}",
                s.id
            );
        }
        assert_eq!(agents[&Agent::Claude], 11);
        assert_eq!(agents[&Agent::Codex], 11);
        assert_eq!(agents[&Agent::Antigravity], 11);
        assert_eq!(folders.len(), 4);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn sandbox_mtimes_span_past_three_months() {
        let base = temp_base("mtimes");
        ensure_demo_sandbox(&base).unwrap();
        let sessions = scan_sandbox(&base);
        let now = now_ms();

        // Newest session is "today"; timestamps step back by SESSION_SPACING_MS
        // and never land in the future, regardless of when demo runs.
        assert!(sessions[0].mtime_ms <= now);
        assert!(now - sessions[0].mtime_ms < 60_000);
        for pair in sessions.windows(2) {
            let gap = pair[0].mtime_ms - pair[1].mtime_ms;
            assert!(
                (gap - SESSION_SPACING_MS).abs() < 60_000,
                "unexpected mtime gap {}",
                gap
            );
        }
        let oldest = sessions.last().unwrap().mtime_ms;
        assert!(now - oldest < 95 * 24 * 3600 * 1000);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn sandbox_regeneration_leaves_no_stale_sessions() {
        let base = temp_base("regen");
        ensure_demo_sandbox(&base).unwrap();
        // Plant a stale file that a second run must clear.
        std::fs::write(
            base.join("sessions/claude/projects/demo-session-999.jsonl"),
            "{}\n",
        )
        .unwrap();
        // Preserved app-owned config state must survive regeneration.
        std::fs::write(base.join("config/theme.json"), "{\"name\":\"nord\"}").unwrap();

        ensure_demo_sandbox(&base).unwrap();
        let sessions = scan_sandbox(&base);
        assert_eq!(sessions.len(), 1 + LOOP_SESSIONS);
        assert_eq!(
            std::fs::read_to_string(base.join("config/theme.json")).unwrap(),
            "{\"name\":\"nord\"}"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn set_file_mtime_backdates_file() {
        let base = temp_base("mtime-helper");
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join("f.txt");
        std::fs::write(&path, "x").unwrap();
        let target = now_ms() - 42 * 24 * 3600 * 1000;
        set_file_mtime(&path, target).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let got = meta
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert!((got - target).abs() < 2_000);
        let _ = std::fs::remove_dir_all(&base);
    }
}
