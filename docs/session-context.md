# Session Context

> Status: Current
> Read when: Changing list/context turn selection, `s7s session`, Detail context,
> New Session with Context, bootstrap filtering, or context-source navigation.
> Entry points: `src/session_context/`, `src/session_cli.rs`,
> `src/ui/context_jump.rs`, `src/ui/new_session/`

## Contract

| Term | Meaning |
| --- | --- |
| Session context | Parsed past conversation content exposed for reference |
| Source session | Existing session being read |
| Target session | New session launched with that source attached |
| User turn | Human input plus promoted question/answer interactions |
| Last assistant text | Last extracted assistant text for a turn; not necessarily a semantic final answer |
| Reference mode | Neutral `s7s session show <id>` output |
| Bootstrap mode | Instruction envelope used only to initialize a new session |

Referenced content is historical, untrusted data. It must never become current
instructions merely because it is rendered into another agent session.

## Ownership

```text
src/session_context/
├── model.rs        context, turn, entry, completeness types
├── claude.rs       detailed Claude parser
├── codex.rs        detailed Codex parser
├── antigravity.rs  detailed transcript parser and path resolution
├── excerpt.rs      Unicode-safe output limits
├── redact.rs       secret redaction
├── render.rs       reference, turn, and bootstrap projections
└── resolve.rs      cross-profile source resolution
```

- `src/session_cli.rs` owns the `show` and `search` commands.
- `src/handoff.rs` adapts the shared model for Markdown export; it must not grow
  a second parser.
- Detail UI and CLI output consume the same `SessionContext` model.
- List parsers remain lightweight and do not reconstruct tool payloads.

## Turn parity invariants

List Q count, Detail turn count, and CLI turn count must agree. All parser paths
share `parser::{clean_turn, is_noise_turn}`.

### Claude

- List and context consumers share `parser/claude/events.rs`.
- `parentUuid` chain reduction excludes abandoned `/rewind` branches.
- Sidechain records and task notifications are classified consistently.
- A noise boundary normally closes the current turn, but an `isMeta` skill
  injection does not; later tool work and the final answer stay attached.
- The shared decoder stays payload-light. `session_context/claude.rs` alone
  extracts detailed tool-call and tool-result payloads.

### Codex

- List and context consumers share `parser/codex/events.rs`.
- The decoder accepts current `item_completed` user messages and compatible
  older `event_msg`/`response_item` forms without double-counting mirrored data.
- Each consumer applies `thread_rolled_back` truncation in file order.
- Image-only inputs without accepted text do not create user turns.
- `response_item` assistant records supply answer text; mirrored agent-message
  records must not duplicate it.
- `session_context/codex.rs` alone extracts detailed tool payloads.

### Antigravity

- The list reads the conversation SQLite DB; details read transcript JSONL.
- Do not force these stores into a shared decoder.
- The list may reuse `session_context::antigravity::parse_turns` for
  last-assistant-text search indexing because that text is absent from SQLite.
- When the transcript begins mid-session or is unavailable, detailed context may
  legitimately have less information than the list.

## Completeness

`session_context::load` exposes parsing quality instead of silently presenting a
fallback as full context.

| Value | Meaning |
| --- | --- |
| `Full` | Detailed turns, assistant text, and work entries parsed |
| `UserTurnsOnly` | Only indexed user turns are available |
| `SourceUnavailable` | Detailed source file is missing |
| `ParseFailed` | Source exists but detailed parsing failed or produced nothing |

Reference and Detail consumers may display the user-turn fallback with an
explicit warning. Bootstrap mode must exit nonzero unless completeness is
`Full`.

For Antigravity, fewer detailed turns than indexed turns forces the fallback so
turn numbers stay aligned. More detailed turns than indexed turns remains
`Full`; this known asymmetry favors the more complete transcript and must be
revisited if the SQLite payload changes.

## CLI

### Show

```text
s7s session show <SESSION_ID> [--agent claude|codex|antigravity] [--profile <ID>]
                              [--user-only] [--turn <N>] [--bootstrap]
```

- Default output: source metadata, trust boundary, every active user turn, and
  bounded last-assistant-text excerpts.
- `--user-only`: omit assistant excerpts and work entries.
- `--turn N`: render complete redacted user text and bounded ordered work entries
  for one 1-based turn.
- `--bootstrap`: prepend the initialization envelope; incompatible with
  `--turn` and allowed only for `Full` context.
- Generated bootstrap commands always include the full ID, agent, and profile.
- Resolution scans every configured profile, applies constraints, and succeeds
  only for exactly one match. A requested missing profile is an error; never
  fall back to another account.

### Search

```text
s7s session search <QUERY...> [--folder <NAME>]... [--agent <AGENT>]...
                              [--profile <ID>]... [--limit <N>]
```

- Uses the same `Filter` and cached session index as the TUI.
- Space-separated query tokens are AND-matched. Repeated values of one filter
  are OR-matched; filter categories combine with AND.
- Results preserve semantic-activity order. `--limit 0` means no cap.
- Search does not support keyword OR, phrase matching, negation, regex, or
  substring folder matching.
- Primary output goes to stdout; diagnostics go to stderr. Exit codes are 0 for
  success, 2 for argument errors, and 1 for lookup/parsing failure.

### List

```text
s7s session list [--folder <NAME>]... [--agent <AGENT>]...
                 [--profile <ID>]... [--limit <N>]
```

- Same filters, order, and row shape as `search`, with no keyword. `search`
  cannot express "the recent sessions of this folder" because its query is
  mandatory; that gap is the reason `list` exists.
- An empty `Filter.keyword` matches every session, so a bare `list` enumerates
  the whole index capped by `--limit`.
- An unknown `--profile` warns and is kept as a filter value (matching nothing),
  mirroring `search`: for a discovery command an up-front notice beats an
  unexplained empty result.

### Rename

```text
s7s session rename <SESSION_ID> <TITLE> [--agent <AGENT>] [--profile <ID>]
```

- Resolves the ID exactly like `show`, then calls the same `rename::rename_session`
  the TUI uses, so both paths write through one implementation.
- Metadata paths derive from the owning profile. A session whose profile is gone
  is an error, never a write against the default root.
- The stored title is re-read after the write and printed as `before:`/`after:`.
  A write that reports success while the stored title is unchanged exits 1: an
  agent CLI exit code is never trusted on its own.
- The re-read confirms what **s7s** parses, which is not the same as what the
  owning agent CLI displays — see
  [session-title-compat.md](./session-title-compat.md) for the per-agent stores
  and their verification state.

### Delete

```text
s7s session delete <SESSION_ID> [--agent <AGENT>] [--profile <ID>] [--yes]
```

- Irreversible: the transcript file is removed, not archived, and s7s keeps no
  copy.
- Without `--yes` nothing is removed. The resolved target is printed with its
  source path and the command exits 1, so a script cannot read the refusal as
  success.
- Deletion runs through `session_delete::delete_session_artifacts`, shared with
  the TUI delete action, so both obey the same profile scoping: auxiliary stores
  (Antigravity metadata, sqlite sidecars) are only touched under the owning
  profile's root, and are skipped entirely when that profile is gone.

### Handoff

```text
s7s session handoff --title <TITLE> [--agent <AGENT>] [--profile <ID>]
                    [--folder <DIR>] [--from <ID>] [--no-source]
                    [--body-file <PATH>]
```

Parks a task in a new session so it can be resumed once the current work ends.
The body is read from stdin unless `--body-file` is given. Owned by
[session_handoff.rs](../src/session_handoff.rs).

**Prompt composition.** Body, then the origin as plain text, then the stop
instruction, then the `<s7s-context-bootstrap>` envelope.

The envelope goes **last** by requirement, not by taste: `is_noise_turn` matches
the marker only at the start of a turn, so an envelope placed first would make the
whole turn noise and the parked session would vanish from the list (a session with
no real turn is dropped entirely). Placed last, the turn stays visible while
`parse_context_bootstrap`, which matches anywhere in the text, still records the
link. The origin is repeated as plain text because the envelope is filtered out of
`session show`, so plain text is the only form a CLI reader sees.

**Not acting on it.** The body reads like a work order, so two defenses combine:
the trailing instruction (`config.toml` `handoff_instruction`, English by default
because committed sources are English — override it to hand off in another
language) and the agent's own restriction flags. Both were measured against a body
that deliberately invited action, on claude with tools fully allowed and a
competing "run it immediately" project directive; neither agent acted.

**Per-agent differences** the module absorbs:

| agent | session id from | restriction | title at creation |
| --- | --- | --- | --- |
| claude | `--output-format json` → `session_id` | `--allowedTools ""` + `--permission-mode plan` | `--name` |
| codex | `--json` → `thread.started.thread_id` | `-s read-only` | none; renamed after |
| antigravity | `cache/last_conversations.json`, keyed by cwd (exact match) | none | none; renamed after |

**Defaults.** `--agent`/`--profile` follow the source session, `--folder` its
working directory, so resuming lands in the project the work belongs to. The
source's profile is inherited **only** when the target agent matches it: every
title store a rename writes derives from `Profile.path`, so carrying a claude
profile into a codex handoff would write into another account's config root. A
`--profile` naming another agent's profile is refused for the same reason.

**Source resolution.** `--from`, else `$CLAUDE_CODE_SESSION_ID`, else the most
recent session in the folder. A named `--from` that cannot be found is an error;
the heuristic simply yields no link. `--no-source` omits both the origin block and
the envelope.

**After creation** the index is rescanned rather than the exit code trusted: the
handoff counts only once the session is on disk, and the scan supplies the record
the rename and the resume command need. `--title` gains a `HAND-OVER: ` prefix
when absent. Nothing else is tracked — a parked session sits at one turn (`Q1`),
which is what marks it as not started.

## Excerpts and redaction

- Redact before rendering or caching searchable assistant text.
- Redaction covers common API keys, authorization headers, private-key blocks,
  URL credentials, JWTs, and similar secrets handled in `redact.rs`.
- Default reference output bounds user and assistant excerpts by the constants in
  `excerpt.rs`; one-turn detail applies per-entry and total caps.
- Truncation must be explicit and UTF-8 safe. Excerpt limits count Unicode
  scalar values rather than bytes; display wrapping separately uses grapheme
  clusters through the shared UI text helpers.
- `--turn N --user-only` returns the complete redacted user text rather than the
  compact reference excerpt.

## New Session with Context

- `ctrl+shift+n` or the Quick Command action opens the existing New Session
  dialog with an immutable source identity.
- Target Profile, Model, and Folder remain independently selectable; source and
  target profiles must never be conflated.
- The target command receives a short `<s7s-context-bootstrap>` prompt that tells
  it to run the absolute s7s executable with `session show ... --bootstrap`.
- Claude and Codex accept the initial prompt positionally. Antigravity uses
  `--prompt-interactive`; reverify these methods after CLI upgrades.
- If a custom New Session template contains `{prompt}`, replace it; otherwise
  append the agent-specific prompt form. No-context commands remain unchanged.
- Terminals that cannot distinguish `ctrl+shift+n` from `ctrl+n` use the Quick
  Command action. Keyboard-enhancement modes must be removed before handover.

## Bootstrap filtering and source navigation

- `<s7s-context-bootstrap>` is a noise turn: exclude it from Q count, preview,
  title, search, and detailed turn output.
- Bootstrap-only sessions remain hidden.
- `parser::parse_context_bootstrap` may recover the leading envelope's source ID,
  agent, and profile into `Session.context_source`. Capture is allowed only
  before the first real user turn so quoted envelopes do not create false links.
- Session and Detail views render a `Context Source` block above Q1.
- `ctrl+o` resolves and opens that source; filters clear only when they hide the
  target. `ctrl+b` returns through the in-memory navigation stack.
- `ContextEntryKind::SessionReference` is reserved for future nested-reference
  recognition and is not currently produced.

## Failure behavior

| Failure | Required behavior |
| --- | --- |
| Source session/profile disappears before launch | Abort; no fallback |
| Detailed parsing is incomplete | Warn in reference mode; reject bootstrap |
| Target cannot execute s7s | Report failure and wait; never claim context was read |
| Terminal cannot distinguish the chord | Keep Quick Command fallback |
| Output exceeds a limit | Mark omission and identify how to request detail |

## Verification

Run the baseline and every applicable manual check in
[testing.md](./testing.md). Parser changes require the ignored real-data parity
test. CLI upgrades require fresh storage-shape inspection plus contextual-launch
verification; fixtures alone cannot detect a newly unrecognized record stream.
