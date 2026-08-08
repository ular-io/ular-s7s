# Testing Guide

This is the authoritative verification matrix for the repository. `AGENTS.md`
carries only the concise routing rules and defers here for the full procedure.

Much of s7s depends on the internal storage structures and screen output of
external agent CLIs, so testing must not end with "passing unit tests." Match
the change area below and run every check listed for it.

## Verification matrix by change area

| Change area | Required checks beyond the baseline | Details |
| --- | --- | --- |
| Any code change (baseline) | `scripts/check.sh` — `cargo fmt --all -- --check`, `cargo test -q`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo build --release` | this doc |
| Rename / session-title | Manual CLI verification — external rename is trusted only when the actual storage file change is observed | [session-title-compat.md](./session-title-compat.md), §Manual verification below |
| Usage parsing / usage display | `--usage-probe` cross-check against the real CLI screen (do not misread absolute times vs. countdowns) | [usage-display.md](./usage-display.md) |
| Probe working directory / trust handling | `--usage-probe` **and** `--model-probe` from a folder whose `.claude/settings.json` pre-approves permissions (the case that used to fail): every profile must come back `Ready`. To cover auto-confirm, first drop the probe folder from agy's `trustedWorkspaces` so the dialog is actually raised — agy must re-add it on its own | [usage-display.md](./usage-display.md) |
| Model list / New Session model dropdown | `--model-probe` cross-check against `/model`, `codex debug models`, `agy models` (the CLIs do not reject invalid model names — agy silently falls back — so s7s owns list accuracy) | [models.md](./models.md) |
| Rewind / backtrack parsing (claude `parentUuid` branch, codex `thread_rolled_back`) | Perform a real rewind in the CLI and compare the saved-file diff against the s7s preview (agy rewrites storage destructively, so it has no parser handling — this is expected) | [session-context.md](./session-context.md) |
| Session context parser (`src/session_context/`) or list parser turn selection | `cargo test real_data_turn_parity -- --ignored --nocapture` (List Q count == Detail == CLI turn count); re-verify initial-prompt injection on CLI upgrade | §Session context checks below |
| Session activity time / Updated ordering | `cargo test real_data_index_snapshot -- --ignored --nocapture`; compare Updated against the real CLI record, then resume and exit without input and verify it is unchanged | §Session activity checks below |
| New Session dialog layout / UI | `cargo build --release` is **mandatory**, plus a PTY/TUI visual check | [panel-focus-style.md](./panel-focus-style.md) |
| Panel focus / TUI style | Manual TUI or PTY visual check | [panel-focus-style.md](./panel-focus-style.md) |
| Keyboard protocol / input | kitty-protocol PTY checks and tmux/legacy fallback | §Keyboard protocol checks below |
| Terminal lifecycle / bracketed paste / grapheme editing | Fault-injection lifecycle tests + paste-routing tests, plus the real-terminal checks below | §Terminal lifecycle and paste checks · [terminal-input-hardening.md](./terminal-input-hardening.md) |
| Storage structure change | Update code and the owning document together; consider whether `CACHE_VERSION` must bump | [session-title-compat.md](./session-title-compat.md) |
| CLI flags / subcommands / `s7s <dir>` startup | `runtime::tests` parse cases, plus a run of the release binary: `s7s <dir>` opens the dialog on that folder, and a wrong path / a subcommand combination exits 2 before the scan | [architecture.md](./architecture.md) |

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
- Check `display_title` in `local_thread_catalog` of `~/.codex/sqlite/codex-*.db` — a store 0.147 added; confirm whether a rename now lands here instead of `session_index.jsonl`
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
6. **On a codex upgrade, diff the record shape of a freshly written rollout** — codex 0.147 dropped the `event_msg` `user_message`/`agent_message` events in favor of one `item_completed` item stream, which silently removed every 0.147 session from the list. A record-name change is invisible to `cargo test -q` (fixtures still use the old names) and to the parity test (a session with zero turns is simply absent from both views), so compare the distribution directly:

   ```bash
   jq -r '"\(.type)/\(.payload.type // "-")\(if .payload.item.type then "["+.payload.item.type+"]" else "" end)"' \
     "$(ls -t ~/.codex/sessions/*/*/*/rollout-*.jsonl | head -1)" | sort | uniq -c | sort -rn
   ```

   Then confirm the session is listed with the right Q count: `s7s session search --agent codex --limit 3 ""`.

## Session activity checks

When semantic activity parsing or Updated ordering changes:

1. Compare the latest active user timestamp and response-completion timestamp
   against the displayed Updated value for all three agents.
2. Resume each session and exit without input. Physical mtime/cache freshness may
   change, but Updated and list order must remain unchanged.
3. Submit a query and interrupt before completion. Updated must use the query
   timestamp.
4. Complete a response. Updated must advance to response completion.
5. For Claude rewind and Codex rollback, abandoned-branch timestamps must not
   affect Updated.

## Keyboard protocol checks

- Verify separate behaviors in kitty protocol-supported terminals: `ctrl+shift+n` → contextual, `ctrl+n` → ordinary.
- Verify the fallback to the `:` palette in legacy terminals · tmux.
- Verify that keyboard enhancement does not remain active after exit · agent handover (check if key inputs in the handed-over CLI are normal).

## Terminal lifecycle and paste checks

Contract and rationale: [terminal-input-hardening.md](./terminal-input-hardening.md).
Automated coverage (`scripts/check.sh`) is the grapheme, paste-routing, and
fault-injection lifecycle tests. The following need a real terminal — run them in
the primary terminal plus one legacy/tmux terminal and one kitty-protocol
terminal:

1. Paste multi-line text into every editable field (keyword `/`, rename `ctrl+r`,
   profile form, New Session folder, `:` palette, `!` terminal, `f` folder filter).
   Each must insert once and submit nothing.
2. In `!` terminal mode, a multi-line paste must keep only the first line, report
   it in the status line, and run nothing until a separate Enter keypress.
3. Verify grapheme editing: `←/→`, Backspace, and Delete must treat `é`
   (`e` + U+0301) and a ZWJ family emoji as one character.
4. Exit normally and confirm the parent shell has normal echo, cursor visibility,
   keyboard mode, and paste behavior. Repeat after each handover type (resume, new
   session, login, `!` terminal, Edit Config).
5. Force an unwind panic with the debug-only probe and confirm the terminal is
   restored **and** the panic message is readable on the main screen:
   `S7S_PANIC_PROBE=1 cargo run -- demo` (compiled out of release builds).
6. Confirm neither keyboard enhancement nor bracketed paste leaks into the child
   agent or the parent shell (`printf` a bracketed-paste sequence in the shell
   afterwards, or paste into the handed-over CLI).

A PTY harness can automate 1, 2, 4 and 5 by driving the release binary with
`\e[200~…\e[201~` sequences and comparing the pre/post `termios` of the slave fd.

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
- [Terminal and Text Input Hardening](./terminal-input-hardening.md)
