# Session Title Compatibility

> Status: Current and version-sensitive
> Read when: Changing title parsing, rename behavior, or agent storage paths.
> Entry points: `src/rename.rs`, `src/title.rs`, agent list parsers,
> `s7s session rename` (`src/session_cli.rs`)

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
- live-session registry: `~/.claude/sessions/*.json`

> **The registry is keyed by process id on claude 2.1.263 (verified 2026-09-08).**
> Entries are `<pid>.json` (with a sibling `<pid>.<hash>.key`) carrying `pid`,
> `sessionId`, `status`, `messagingSocketPath`, and a derived `name` — a record of
> a *running* session, not a persistent title store. A rename in the CLI writes no
> file here.
>
> s7s therefore no longer creates `<sessionId>.json`: such a file is one only s7s
> would read. An existing entry whose `sessionId` matches is still updated, since
> that renames the live session's display name. Files left from the old scheme are
> still read as a last-resort title source, which is harmless because body events
> take precedence and every s7s rename writes them.

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
4. If failed, append the JSONL event directly. A matching registry entry is
   refreshed when one exists, but none is created.

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
- (2026-09-08, claude 2.1.263) the same command **exits with an error** —
  `No deferred tool marker found in the resumed session` — while still writing both
  body events. This is exactly why success is judged by the file change and never by
  the exit code.

### Failure modes

- The CLI might return a success exit code but not write the title event.
- If the JSONL structure changes, the detection of `custom-title`/`agent-name` could break.
- If the registry scheme changes again, the entry refresh silently stops matching.
  That is not a title failure on its own — the body events remain authoritative.

## Codex

### Read paths

- session body: `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<sessionId>.jsonl`
  (an archived session's rollout moves to `~/.codex/archived_sessions/`, which s7s
  does not scan — archived threads drop out of the list without needing a filter)
- title column: `threads.name` in `~/.codex/state_*.sqlite`
- title log: `~/.codex/session_index.jsonl`

> **Read path settled on codex 0.153.4 (verified 2026-09-08).** `threads.name` is
> the store the codex CLI displays. A rename issued through codex's own app server
> wrote `threads.name` and appended a `session_index.jsonl` record, and `lsof` on a
> live `codex app-server` showed it opening `state_*.sqlite` and never opening
> `session_index.jsonl`.
>
> Two earlier notes were wrong and are corrected here. `local_thread_catalog`
> (`display_title`, in `~/.codex/sqlite/codex-*.db`) takes **no part** in renames —
> codex leaves it stale too, and that database has no `threads` table at all. And
> `session_index.jsonl` was not abandoned — codex still appends to it; it had simply
> seen no rename since the upgrade.
>
> `threads.title` holds codex's auto-derived first-message text, not a user title.
> s7s never reads it (it derives its own fallback from the first user turn) and
> writes it only to keep an older codex build in step.

### Title fields

- body
  - `session_meta.payload.id`
- title log
  - `id`
  - `thread_name` (append-only: the **last** record for an id wins)
- sqlite
  - `threads.id`
  - `threads.name` (what the CLI displays)
  - `threads.title` (legacy auto text)

### Current rename strategy

1. Ask codex's app server to do it: spawn `codex app-server` (with the profile's
   `CODEX_HOME`), `initialize`, then `thread/name/set` with `{threadId, name}`.
   This is the same request the codex CLI issues, so every store codex maintains
   stays in step.
2. Read `threads.name` back. Only a match counts as success — a JSON-RPC result is
   no more trustworthy than an exit code.
3. If that path is unavailable or unverified, write the stores directly:
   `thread_name` in every `session_index.jsonl` record for the id, then
   `threads.name` **and** `threads.title` in `state_*.sqlite`.

The whole app-server exchange runs under one 20-second budget and every failure
falls through to step 3, so a rename can never hang on a stalled handshake.

### Read strategy

`threads.name` overrides `session_index.jsonl`, since it is the column codex
displays. The index is still read first as the broader source: it covers sessions
the state database has no row for.

### Verified behavior

- `thread/name/set` over `codex app-server` stdio succeeds and emits a
  `thread/name/updated` notification (0.153.4, 2026-09-08).
- After it, `threads.name` carries the new title, `session_index.jsonl` gains an
  appended record, and `threads.title` is untouched.
- A direct s7s write to `threads.name` is also picked up by the codex CLI, so the
  fallback path is not second-class.
- Non-interactive `codex exec resume <id> "/rename ..."` did not change the title,
  and there is no `codex rename` subcommand. The app server is the only CLI path.
- 0.153 added `codex archive` / `unarchive` / `delete`, all of which accept a
  **session name** as the identifier. A wrong stored name is now a wrong lookup
  key, not only a wrong label.

### Failure modes

- If `threads.name` is renamed or dropped, the CLI-visible title stops updating
  while the index still shows the new one — the read-back check is what turns that
  into a visible failure instead of a silent divergence.
- If the app server's method name or parameter shape changes, the rename falls back
  to direct writes. Regenerate the contract with
  `codex app-server generate-ts --out <dir>` and grep `ClientRequest` for the
  current method string.
- If the `session_index.jsonl` format changes, explicit title loading will break.
- Because the session body filename and the actual `session_meta.payload.id` might mismatch, matching by filename is prohibited.

## Antigravity (agy)

### Read paths

- session DB: `~/.gemini/antigravity-cli/conversations/<conversationId>.db`
- title annotation: `~/.gemini/antigravity-cli/annotations/<conversationId>.pbtxt`
- metadata cache: `~/.gemini/antigravity-cli/cache/conversation_metadata.json`
- last conversation map: `~/.gemini/antigravity-cli/cache/last_conversations.json`
- s7s handoff workspace store: `~/.config/s7s/session_workspaces.json`

> **The metadata cache has gone stale on agy 1.1.27 (verified 2026-09-08).** It
> stopped gaining entries for new conversations, so a recent session has no entry
> at all and the annotation pbtxt is the only live title store. s7s refreshes an
> entry that already exists and never inserts one, because an inserted entry would
> grow a cache nothing reads back.
>
> A new `~/.gemini/antigravity-cli/conversation_summaries.db` exists
> (`conversation_summaries(conversation_id, title, agent_name, preview, …)`), but it
> is **not** the live store: of 124 rows only 3 carry a title and the newest row
> predates the current sessions. Do not write it without re-checking. The new
> `agent_name` column and the `AgentName` summary field are agent nicknames, not
> titles.

`agy --print` also omits the launch cwd from its conversation DB. For sessions
created by `s7s session handoff`, s7s captures the exact target cwd after reading
the new id from `last_conversations.json` and stores it in its own workspace
file. This is a scan fallback only: DB workspace data and `WorkspaceURIs` retain
precedence, and ordinary `agy --print` sessions are not inferred from the
cwd-keyed last-conversation cache.

### Title fields

- annotation
  - `title:"..."`
- metadata
  - `conversations.<id>.summary.Title`
  - `conversations.<id>.summary.Preview`

### Current rename strategy

- `title:"..."` in `annotations/<id>.pbtxt` — the live store
- `summary.Title` in `conversation_metadata.json` — refreshed only for an entry
  that already exists

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
- Prefer the agent's own rename path, then verify it against storage: claude's
  `--name`, codex's `thread/name/set`. Both report failure while succeeding, so the
  storage check is what decides.
- External CLI renames are only considered successful when an "actual file change" is verified.
- Do not trust the exit code before confirming success.
- Write only stores the agent still maintains. A write to an abandoned store costs
  nothing to make and everything to trust later.
- `s7s session rename` re-reads the stored title after writing and fails when it
  does not match the requested one.
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
- [src/parser/codex/mod.rs](../src/parser/codex/mod.rs) (record decoding shared via [src/parser/codex/events.rs](../src/parser/codex/events.rs))
- [src/parser/antigravity.rs](../src/parser/antigravity.rs)
- [src/scan.rs](../src/scan.rs)
- [src/title.rs](../src/title.rs)
