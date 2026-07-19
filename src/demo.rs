//! Demo sandbox data generation module.
//!
//! Generates mock Claude, Codex, and Antigravity sessions in a local `example/`
//! directory to demonstrate and test s7s TUI capabilities.

use crate::model::Agent;

/// Protobuf helper: length-delimited (wire type 2) field.
fn pb_len_field(field: u64, bytes: &[u8]) -> Vec<u8> {
    fn varint(mut v: u64, out: &mut Vec<u8>) {
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
    let mut out = Vec::new();
    varint(field << 3 | 2, &mut out);
    varint(bytes.len() as u64, &mut out);
    out.extend_from_slice(bytes);
    out
}

/// Protobuf helper: varint (wire type 0) field.
fn pb_varint_field(field: u64, val: u64) -> Vec<u8> {
    fn varint(mut v: u64, out: &mut Vec<u8>) {
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
    let mut out = Vec::new();
    varint(field << 3 | 0, &mut out);
    varint(val, &mut out);
    out
}

/// Serializes Antigravity step payload into the custom protobuf shape.
/// Used to avoid parser hacks on conversation DB files.
fn encode_antigravity_step(msg: &str, folder: &str, ts_ms: i64) -> Vec<u8> {
    // User message: 19.2 (string)
    let msg_encoded = pb_len_field(2, msg.as_bytes());

    // Workspace: 19.12.12 (file:// URI)
    let cwd = format!("/Users/ular-io/dev/{}", folder);
    let ws_uri = format!("file://{}", cwd);
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

/// Cleans old sandbox data and populates fresh mock sessions under `example/`.
pub fn ensure_demo_sandbox() -> anyhow::Result<()> {
    let base = std::env::current_dir()?
        .join("example");

    // 0. Clean old structures and stale cache to avoid compile/count artifacts
    let _ = std::fs::remove_dir_all(base.join("sessions"));
    let _ = std::fs::remove_dir_all(base.join("cache"));

    // 1. Create directory structures
    std::fs::create_dir_all(base.join("config"))?;
    std::fs::create_dir_all(base.join("cache"))?;
    std::fs::create_dir_all(base.join("sessions/claude/projects"))?;
    std::fs::create_dir_all(base.join("sessions/claude/sessions"))?;
    std::fs::create_dir_all(base.join("sessions/codex/sessions"))?;
    std::fs::create_dir_all(base.join("sessions/antigravity/conversations"))?;
    std::fs::create_dir_all(base.join("sessions/antigravity/cache"))?;

    // 2. Write example/config/config.toml
    let config_toml_path = base.join("config/config.toml");
    if !config_toml_path.exists() {
        std::fs::write(&config_toml_path, crate::config::CONFIG_TEMPLATE)?;
    }

    // 3. Write example/config/profiles.json (dynamic absolute paths)
    let claude_path = base.join("sessions/claude");
    let codex_path = base.join("sessions/codex");
    let antigravity_path = base.join("sessions/antigravity");
    let profiles_json = serde_json::json!({
        "version": 1,
        "profiles": [
            {
                "id": "builtin-claude",
                "agent": "Claude",
                "name": "Claude",
                "path": claude_path,
                "active": true,
                "shortcut": 1,
                "builtin": true
            },
            {
                "id": "builtin-codex",
                "agent": "Codex",
                "name": "Codex",
                "path": codex_path,
                "active": true,
                "shortcut": 2,
                "builtin": true
            },
            {
                "id": "builtin-antigravity",
                "agent": "Antigravity",
                "name": "Antigravity",
                "path": antigravity_path,
                "active": true,
                "shortcut": 3,
                "builtin": true
            }
        ]
    });
    std::fs::write(
        base.join("config/profiles.json"),
        serde_json::to_string_pretty(&profiles_json)?,
    )?;

    // 4. Write Claude mock session 0 (10 turns, highly detailed, nextjs-auth-app)
    let claude_session_id = "demo-session-0";
    let claude_jsonl_content = r#"{"uuid":"c1","parentUuid":null,"type":"user","message":{"role":"user","content":[{"type":"text","text":"Setup Next.js 14 project and create custom Edge middleware for JWT authorization."}]},"cwd":"/Users/ular-io/dev/nextjs-auth-app"}
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
    std::fs::write(
        base.join(format!("sessions/claude/projects/{}.jsonl", claude_session_id)),
        claude_jsonl_content,
    )?;

    let claude_meta_json = serde_json::json!({
        "sessionId": claude_session_id,
        "name": "Fullstack JWT authentication flow integration",
        "nameSource": "user"
    });
    std::fs::write(
        base.join(format!("sessions/claude/sessions/{}.json", claude_session_id)),
        serde_json::to_string_pretty(&claude_meta_json)?,
    )?;

    // 5. Generate loop-based mock sessions (idx 1..=32)
    let topics = [
        "Fullstack JWT authentication flow integration",
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
        "Configure automated semantic versioning and release notes action"
    ];

    let mut codex_index_lines = Vec::new();
    let mut antigravity_convs = serde_json::Map::new();

    for idx in 1..=32 {
        let topic = topics[idx % topics.len()];
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
        let cwd = format!("/Users/ular-io/dev/{}", folder);
        let q_count = 3 + (idx % 28); // 3 to 30

        // Helper closures to synthesize realistic Q&As based on the topic
        let make_q = |step: usize| {
            match step {
                0 => format!("Implement the features for {}.", topic),
                1 => format!("Let's debug the current issue in {} configuration.", topic),
                2 => format!("Can you write a comprehensive unit test to verify {}?", topic),
                3 => format!("Is there any security risk or memory leak within {} implementation?", topic),
                4 => format!("Refactor this module to optimize performance of {}.", topic),
                5 => format!("How should we handle potential boundary exceptions for {}?", topic),
                6 => format!("Write a shell script to automate deployment validation of {}.", topic),
                7 => format!("Analyze the heap dump to track down memory usage for {}.", topic),
                8 => format!("Is there a better software pattern for this {} layout?", topic),
                9 => format!("Configure production environment variables for {}.", topic),
                _ => format!("Verify step {} of the {} task list.", step + 1, topic),
            }
        };

        let make_a = |step: usize| {
            match step {
                0 => format!("I will start by installing dependencies and initializing the boilerplate code for {}.", topic),
                1 => format!("The compiler errors are now resolved. The {} setup is clean and active.", topic),
                2 => format!("Certainly. I've created the mock specs under the test suite verifying {} behavior.", topic),
                3 => format!("Ensure to sanitize parameters and check buffers to prevent leaks in {}.", topic),
                4 => format!("We can refactor by lazy loading components to speed up {} rendering.", topic),
                5 => format!("I've added retry logic and default fallbacks to stabilize {} errors.", topic),
                6 => format!("Here is the bash script testing all endpoint responses for {} configurations.", topic),
                7 => format!("GC stats look stable and no persistent object leaks were found for {}.", topic),
                8 => format!("Using a factory pattern or pub-sub broker would streamline the {} calls.", topic),
                9 => format!("Setting standard memory limits and connection sizes suitable for {} scaling.", topic),
                _ => format!("Successfully finished step {} of {} development.", step + 1, topic),
            }
        };

        match agent {
            Agent::Claude => {
                let session_id = format!("demo-session-{}", idx);
                let mut jsonl = String::new();
                
                for q in 0..q_count {
                    let uuid = format!("u-{}-{}", idx, q);
                    let parent = if q == 0 { "null".to_string() } else { format!("\"u-{}-{}\"", idx, q - 1) };
                    
                    jsonl.push_str(&format!(
                        "{{\"uuid\":\"{}\",\"parentUuid\":{},\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}},\"cwd\":\"{}\"}}\n",
                        uuid, parent, make_q(q), cwd
                    ));
                    
                    let a_uuid = format!("a-{}-{}", idx, q);
                    jsonl.push_str(&format!(
                        "{{\"uuid\":\"{}\",\"parentUuid\":\"{}\",\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}}}}\n",
                        a_uuid, uuid, make_a(q)
                    ));
                }
                
                std::fs::write(
                    base.join(format!("sessions/claude/projects/{}.jsonl", session_id)),
                    jsonl,
                )?;

                let meta = serde_json::json!({
                    "sessionId": session_id,
                    "name": topic,
                    "nameSource": "user"
                });
                std::fs::write(
                    base.join(format!("sessions/claude/sessions/{}.json", session_id)),
                    serde_json::to_string_pretty(&meta)?,
                )?;
            }
            Agent::Codex => {
                let session_id = format!("demo-session-{}", idx);
                let mut jsonl = String::new();
                
                jsonl.push_str(&format!(
                    "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{}\",\"cwd\":\"{}\"}}}}\n",
                    session_id, cwd
                ));
                
                for q in 0..q_count {
                    jsonl.push_str(&format!(
                        "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"{}\"}}}}\n",
                        make_q(q)
                    ));
                    jsonl.push_str(&format!(
                        "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"agent_message\",\"message\":\"{}\"}}}}\n",
                        make_a(q)
                    ));
                }
                
                std::fs::write(
                    base.join(format!("sessions/codex/sessions/rollout-{}.jsonl", session_id)),
                    jsonl,
                )?;

                codex_index_lines.push(format!(
                    "{{\"id\":\"{}\",\"thread_name\":\"{}\"}}",
                    session_id, topic
                ));
            }
            Agent::Antigravity => {
                let session_id = format!("demo-session-{}", idx);
                
                // Create sqlite dummy database
                let db_path = base.join(format!("sessions/antigravity/conversations/{}.db", session_id));
                if db_path.exists() {
                    let _ = std::fs::remove_file(&db_path);
                }
                let conn = rusqlite::Connection::open(&db_path)?;
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS steps (
                        idx INTEGER PRIMARY KEY,
                        step_type INTEGER,
                        step_payload BLOB
                    )",
                    [],
                )?;

                // Insert user steps with actual protobuf payloads
                for q in 0..q_count {
                    let ts_ms = 1784430000000i64 - (idx as i64) * 233280000i64 + (q as i64) * 60000i64;
                    let payload = encode_antigravity_step(&make_q(q), folder, ts_ms);
                    conn.execute(
                        "INSERT INTO steps (idx, step_type, step_payload) VALUES (?, ?, ?)",
                        rusqlite::params![q, 14, payload],
                    )?;
                }

                let ts_ms = 1784430000000i64 - (idx as i64) * 233280000i64;
                let secs = ts_ms / 1000;
                use chrono::{TimeZone, Utc};
                let utc_time = Utc.timestamp_opt(secs, 0).unwrap();
                let last_modified_str = utc_time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

                // Add to metadata
                let conv_meta = serde_json::json!({
                    "last_modified_time": last_modified_str,
                    "summary": {
                        "Title": topic,
                        "Preview": format!("Checking configurations for {}.", topic),
                        "WorkspaceURIs": [format!("file://{}", cwd)]
                    }
                });
                antigravity_convs.insert(session_id.clone(), conv_meta);

                // Transcript logs (using valid JSON with single trailing curly brace)
                let mut transcript = String::new();
                let logs_dir = base.join(format!("sessions/antigravity/brain/{}/.system_generated/logs", session_id));
                std::fs::create_dir_all(&logs_dir)?;

                for q in 0..q_count {
                    transcript.push_str(&format!(
                        "{{\"source\":\"USER_EXPLICIT\",\"type\":\"USER_INPUT\",\"content\":\"<USER_REQUEST>\\n{}\\n</USER_REQUEST>\"}}\n",
                        make_q(q)
                    ));
                    transcript.push_str(&format!(
                        "{{\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"content\":\"{}\"}}\n",
                        make_a(q)
                    ));
                }
                
                std::fs::write(
                    logs_dir.join("transcript.jsonl"),
                    transcript,
                )?;
            }
        }
    }

    // 6. Write global Codex index file
    std::fs::write(
        base.join("sessions/codex/session_index.jsonl"),
        codex_index_lines.join("\n") + "\n",
    )?;

    // 7. Write global Antigravity metadata file
    let antigravity_metadata_json = serde_json::json!({
        "conversations": antigravity_convs
    });
    std::fs::write(
        base.join("sessions/antigravity/cache/conversation_metadata.json"),
        serde_json::to_string_pretty(&antigravity_metadata_json)?,
    )?;

    Ok(())
}
