# Testing Guide

The rename/session-title related logic in this project depends on the internal storage structures of external agent CLIs.
Therefore, testing must not end with just "passing unit tests."

## Required checks

Perform at least the following when changing code:

1. `cargo fmt --all`
2. `cargo test -q`
3. If rename/session-title related code was changed, manual verification in the actual local CLI
4. If panel focus/style was changed, manual verification in the actual TUI

## Why unit tests are not enough

- External CLIs may change their storage file structures upon upgrades.
- The behavior might change even with the same option name.
- Even if the exit code is a success, the actual title event might not be written.
- Certain agents might have different rename behaviors between non-interactive and interactive environments.

## Unit test policy

Tests for the rename/session-title code must cover the following:

- Whether explicit renames take precedence over automated titles
- Whether the title is overwritten based on ID when reapplying the cache
- Whether the meta file paths for each agent are correct
- Whether fallback writing is avoided (not duplicated) when CLI rename succeeds
- Whether it falls back to a direct storage update when CLI rename fails

## Current automated coverage

- Claude meta JSON update
- Claude JSONL title event append
- Prevention of duplicate appends upon successful Claude CLI rename
- Codex `session_index.jsonl` update
- Codex sqlite `threads.title` update
- Antigravity `annotations/*.pbtxt` update
- Derivation of meta paths from profile roots (common to all three agents — rename tests use arbitrary roots)
- Aborting rename when the affiliated profile is not found (prohibiting fallback to the default path)

## Manual verification checklist

If rename/session-title logic has been changed or an external CLI has upgraded, manually verify the following:

1. Create a temporary session
2. Execute a title change
3. Verify the title change in the TUI list
4. Reopen the same session in the agent's original CLI
5. Verify the session title is retained
6. Rescan the list after restarting the app
7. Verify the title is retained even after `--rebuild-cache`

## Agent-specific manual checks

### Claude

- Run `claude --resume <id> --name <title> -p --output-format json`
- Check if `custom-title` / `agent-name` events appear in the JSONL
- Check `name`, `nameSource` in `~/.claude/sessions/*.json`
- Verify that the `/rename ...` prompt is still blocked in non-interactive environments

### Codex

- Check `thread_name` in `~/.codex/session_index.jsonl`
- Check `threads.title` in `~/.codex/state_*.sqlite`
- Verify any changes in the behavior of non-interactive `codex exec resume <id> "/rename ..."`

### Antigravity

- `title:"..."` in `annotations/<id>.pbtxt`
- `summary.Title` in `conversation_metadata.json`
- Reverify if `agy --print "/rename ..."` leaves actual rename traces
- Reverify if `--conversation <id>` actually uses the target session

## Session context / contextual launch checks

If session context (`src/session_context/` · `s7s session`) or New Session with Context paths have been changed, or if agent CLIs have upgraded, verify the following ([Details](./session-context.md)).

1. `cargo test real_data_turn_parity -- --ignored --nocapture` — Full real-data turn parity (list Q count == context turn count, claude/codex strict).
2. `s7s session show <actual ID>` — Verify that the reference output has no stop/wait/language instructions, and check each projection (`--turn`/`--user-only`/`--bootstrap`). Also spot-check `s7s session search <keyword>` (with/without `--folder`/`--agent`/`--profile`/`--limit`) against known sessions.
3. Actual contextual launch (each agent): Check if the bootstrap prompt is recorded as a user turn in the transcript, if `s7s session show ... --bootstrap` succeeds, if there are no past tasks/file changes executed, and if the ready message is in the source user turn's primary language.
4. Check that the launched session does not contaminate the s7s list's Q count/preview/title/search (sessions with only a bootstrap are hidden from the list), and remains the same even after `--rebuild-cache`.
5. Reverify the initial prompt injection method upon CLI upgrade: claude/codex positional (`[prompt]`/`[PROMPT]`), agy `--prompt-interactive` (positional unsupported).

## Keyboard protocol checks

- Verify separate behaviors in kitty protocol-supported terminals: `ctrl+shift+n` → contextual, `ctrl+n` → ordinary.
- Verify the fallback to the `:` palette in legacy terminals · tmux.
- Verify that keyboard enhancement does not remain active after exit · agent handover (check if key inputs in the handed-over CLI are normal).

## When to update tests and docs

Update tests and documentation together if any of the following changes:

- CLI options
- Session storage paths
- Title field names
- Session ID extraction methods
- Cache structure
- Rename success determination logic

## Related docs

- [Panel Focus Style](./panel-focus-style.md)
- [Session Title Compatibility](./session-title-compat.md)
- [Session Context](./session-context.md)
