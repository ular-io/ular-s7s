# Session Title Compatibility

The session title processing logic of `s7s` strongly depends on the internal storage structures of external agent CLIs.
These storage structures and rename behaviors can change at any time during agent upgrades.

This document records the following:

- Actual title storage locations per agent
- Read and write paths used by `s7s`
- Currently verified rename paths
- Elements with a high likelihood of changing
- Check sequence for structure changes

## Volatile Warning

All of the elements below are subject to change.

- The file path where the session title is stored
- The field name where the session title is stored
- The method of extracting the session ID
- Whether renaming is possible in a non-interactive CLI environment
- Whether the body or metadata updates first upon successful rename
- The option names and behavior of the session resume command

Therefore, whenever an agent CLI is upgraded, the implementation must be reverified in an actual local environment before modifying the code.

## Claude

### Read paths

- session body: `~/.claude/projects/<encoded-cwd>/<sessionId>.jsonl`
- session meta: `~/.claude/sessions/*.json`

### Title fields

- body events
  - `custom-title.customTitle`
  - `agent-name.agentName`
  - `ai-title.aiTitle`
- meta file
  - `sessionId`
  - `name`
  - `nameSource`

### Current rename strategy

1. Attempt `claude --resume <id> --name <title> -p --output-format json`
   (If the session belongs to a profile with an additional path, inject `CLAUDE_CONFIG_DIR` + clean contaminated env — same rule as resume, 46th)
2. Check if `custom-title` + `agent-name` events have appeared in the actual JSONL
3. If successful, trust the result
4. If failed, update the meta JSON + manually append the JSONL event

### Read precedence

Body events are the authoritative title source; the meta file is a fallback only:

1. body `custom-title` (explicit `/rename`)
2. body `agent-name`
3. body `ai-title`
4. meta `name` (applied only when no body-derived title exists — both at parse
   time and on cache-hit refreshes, so a stale meta name can never clobber a
   body title)

`nameSource` marks the title as fixed only for explicit sources (`custom`,
`user`). `derived`/`auto`/missing sources are not fixed: the CLI writes auto
names with no `nameSource` at times, including degenerate ones (the session id
used as the name), and non-fixed titles go through the bad-auto-title fallback
in `title::resolve`.

### Verified behavior

- `--name` leaves title events even in non-interactive environments.
- The `/rename ...` prompt does not currently work in the print environment.
- Calling only `--name` without a prompt may fail if there are no deferred markers.
- Thus, the current implementation determines success based on whether the actual file changed after calling `--name`.
- (2026-07-19) `claude --resume <id> --name <t> -p --output-format json` writes
  both `custom-title` and `agent-name` body events but does **not**
  create/update `~/.claude/sessions/<id>.json`; the meta file comes from s7s's
  fallback rename and from CLI auto naming.

### Failure modes

- The CLI might return a success exit code but not write the title event.
- If the JSONL structure changes, the detection of `custom-title`/`agent-name` could break.
- If the format of `~/.claude/sessions/*.json` changes, the fallback meta update could break.

## Codex

### Read paths

- session body: `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<sessionId>.jsonl`
- title index: `~/.codex/session_index.jsonl`
- title DB: `threads.title` in `~/.codex/state_*.sqlite`

### Title fields

- body
  - `session_meta.payload.id`
- title index
  - `id`
  - `thread_name`
- sqlite
  - `threads.id`
  - `threads.title`

### Current rename strategy

- Update `thread_name` in `session_index.jsonl`
- Update `threads.title` in `state_*.sqlite`

### Verified behavior

- Non-interactive `codex exec resume <id> "/rename ..."` did not change the title.
- Codex responded that direct title changes via the `/rename` prompt are impossible.
- At present, the external CLI-based rename path cannot be trusted.

### Failure modes

- If the sqlite schema changes, the `threads.title` update will break.
- If the `session_index.jsonl` format changes, explicit title loading will break.
- Because the session body filename and the actual `session_meta.payload.id` might mismatch, matching by filename is prohibited.

## Antigravity (agy)

### Read paths

- session DB: `~/.gemini/antigravity-cli/conversations/<conversationId>.db`
- title annotation: `~/.gemini/antigravity-cli/annotations/<conversationId>.pbtxt`
- metadata cache: `~/.gemini/antigravity-cli/cache/conversation_metadata.json`
- last conversation map: `~/.gemini/antigravity-cli/cache/last_conversations.json`

### Title fields

- annotation
  - `title:"..."`
- metadata
  - `conversations.<id>.summary.Title`
  - `conversations.<id>.summary.Preview`

### Current rename strategy

- `title:"..."` in `annotations/<id>.pbtxt`
- `summary.Title` in `conversation_metadata.json`

### Verified behavior

- Non-interactive `agy --print "/rename ..."` currently leaves no rename traces in metadata or pbtxt.
- Even with `--conversation <id>`, it was observed reusing the last conversation in the current working directory in reality.
- At present, the external CLI-based rename path cannot be trusted.

### Failure modes

- `Preview` is not the title, so it must not be treated the same as `Title`.
- A version could emerge where `pbtxt` is absent and only metadata is updated.
- Conversely, a version where metadata is empty and only pbtxt is updated is also possible.

## `s7s` Implementation Principles

- All meta paths written to by rename are derived from the **config root of the profile the session belongs to** (`Profile.path`) (`rename_session(&Profile, ...)`). The default path notations like `~/.claude` in the sections above are examples based on the builtin profile; sessions of additional profiles are recorded in their respective profile roots. If the profile is not found, it aborts the rename without falling back to the default path (prevents cross-account recording).
- External CLI renames are only considered successful when an "actual file change" is verified.
- Do not trust the exit code before confirming success.
- For agents where external CLI renaming is unverified, maintain the direct storage update method.
- Metadata must also be reapplied even in cache reuse paths.
- If the storage structure changes, consider bumping the cache version.

## Investigation Sequence upon Structure Changes

1. Recheck the target agent CLI's `--help` and resume-related subcommands.
2. Create a temporary session and actually attempt a title change.
3. Check the diff of the stored files before and after the title change.
4. Investigate the session body, meta files, auxiliary caches, and sqlite entirely.
5. Determine which file is the source of truth.
6. Reflect the read/write paths simultaneously in the documentation and code.
7. Update both unit tests and manual verification procedures.

## Related Code

- [src/rename.rs](../src/rename.rs)
- [src/parser/claude/mod.rs](../src/parser/claude/mod.rs) (title-event decoding shared via [src/parser/claude/events.rs](../src/parser/claude/events.rs))
- [src/parser/codex.rs](../src/parser/codex.rs)
- [src/parser/antigravity.rs](../src/parser/antigravity.rs)
- [src/scan.rs](../src/scan.rs)
- [src/title.rs](../src/title.rs)
