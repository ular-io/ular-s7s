# Repo Agent Notes

s7s is a Rust terminal UI (ratatui/crossterm) for browsing, searching, renaming,
and resuming coding-agent sessions (Claude Code, Codex, Antigravity) across
multiple profiles, plus a `s7s session` CLI that projects stored session context.
It reads each agent's on-disk session storage directly, so much of its behavior is
coupled to external CLI formats that can change on upgrade.

## Routing: read before you change

Read only the document for the area being changed. Use
[architecture.md](./docs/architecture.md) when a change crosses areas or the
owning module is unclear. Use [backlog.md](./docs/backlog.md) only for feature
planning; backlog entries are not implementation authorization.

| If you change… | Read first |
| --- | --- |
| Rename / session-title | [session-title-compat.md](./docs/session-title-compat.md) + [testing.md](./docs/testing.md) |
| Usage display / parsing | [usage-display.md](./docs/usage-display.md) |
| Profiles / env injection (`CLAUDE_CONFIG_DIR` / `CODEX_HOME`) | [profiles.md](./docs/profiles.md) |
| Model list / New Session model dropdown | [models.md](./docs/models.md) |
| Scratch workspace (`src/scratch.rs`, folder dropdown `[SCRATCH]` row) | [architecture.md](./docs/architecture.md) §Scratch workspace + [testing.md](./docs/testing.md) |
| Session context (`src/session_context/`, `s7s session` CLI, New Session with Context, context-source navigation in `src/ui/context_jump.rs`) | [session-context.md](./docs/session-context.md) |
| Session deletion (`src/session_delete.rs`, shared by the TUI action and `s7s session delete`) | [session-context.md](./docs/session-context.md) §Delete + [testing.md](./docs/testing.md) |
| Work handoff (`src/session_handoff.rs`, `s7s session handoff`) | [session-context.md](./docs/session-context.md) §Handoff + [testing.md](./docs/testing.md) |
| TUI layout / panel focus / visual style | [ui-style-guide.md](./docs/ui-style-guide.md) |
| Terminal lifecycle, paste handling, text input/cursor/truncation (`runtime.rs`, `ui/paste.rs`, `ui/components/{input,text}.rs`) | [terminal-input-hardening.md](./docs/terminal-input-hardening.md) |
| Release process | [releasing.md](./docs/releasing.md) |

For past reasoning, prefer `git log -- <path>`, `git blame`, and `git show`.
Completed plans and chronological change logs are intentionally not kept as
active documentation.

## Critical rules

### Security, privacy, language

- **Public repository.** Never hard-code or commit personal local settings
  (absolute paths, local folder names), secrets (API keys, tokens), or other
  sensitive personal information in code or documentation.
- **English only.** All committed source, comments, documentation, and commit
  messages must be written in English. Comments give an agent context; do not
  restate what the code already makes obvious. Any file containing Korean may be
  added to `.gitignore` only after the user approves.

### Compatibility with external CLIs

- Assume an agent CLI upgrade may have changed its storage structure or rename
  method. When the storage structure changes, update the code and its owning
  document in the same change.
- An external CLI rename is trusted only when the actual storage file
  modification is verified — a successful exit code is not enough.

### Verification

Every code change must pass `scripts/check.sh` (`cargo fmt --all -- --check`,
`cargo test -q`, `cargo clippy --all-targets --all-features -- -D warnings`,
`cargo build --release`). The changes below additionally require a manual check —
automated tests alone are insufficient. [testing.md](./docs/testing.md) is the
authoritative matrix; the essentials:

- **Rename / session-title**: manual CLI verification (do not stop at `cargo test -q`).
- **Usage parsing**: `--usage-probe` cross-check against the real CLI screen (do not misread absolute times vs. countdowns).
- **Model list**: `--model-probe` cross-check against `/model`, `codex debug models`, `agy models` (the CLIs accept invalid model names — agy silently falls back — so s7s owns list accuracy).
- **Rewind/backtrack parsing** (claude `parentUuid`, codex `thread_rolled_back`): rewind in the real CLI and compare the saved-file diff against the s7s preview (agy rewrites storage destructively and has no parser handling — expected).
- **Context / list turn selection**: `cargo test real_data_turn_parity -- --ignored --nocapture` (List Q count == Detail == CLI turn count); re-verify initial-prompt injection on CLI upgrade.
- **New Session dialog layout**: `cargo build --release` is mandatory, plus a PTY/TUI visual check.
- **Terminal lifecycle / paste / text editing**: real-terminal paste, exit, and handover checks, plus the `S7S_PANIC_PROBE=1` panic-restore check — [testing.md](./docs/testing.md#terminal-lifecycle-and-paste-checks).

## Current hazards

- The standing risk is external-CLI drift: storage layouts, option behavior, and
  screen output can change between CLI versions even under identical flags. Treat
  every parser and probe as version-specific and re-verify on upgrade.
- Some paths have only been validated indirectly (e.g. agy contextual launch and
  real-kitty `ctrl+shift+n`). Current gaps are tracked in
  [backlog.md](./docs/backlog.md); the detailed procedures live in
  [testing.md](./docs/testing.md).
- **Codex rewrote its rollout event stream in 0.147** and one consequence is still
  open: whether the `thread_rolled_back` marker survived the move to the
  `item_completed` item stream. If it was renamed, rolled-back turns reappear in
  both the list and the detail view. Verify with a real esc-esc rewind
  ([testing.md](./docs/testing.md) rewind row) — it could not be reproduced from
  the sessions on disk.
- **Codex 0.153 is migrating session history into `thread_history_*.sqlite`.**
  That database is a projection of the rollout JSONL (it stores
  `rollout_byte_offset`/`rollout_ordinal`), and the JSONL is still complete for
  already-paginated sessions — `real_data_turn_parity` passes over 833 sessions.
  The open risk is `codex migrate-rollouts --apply`, which s7s has never been run
  against: 443 sessions are reported eligible, and if migration truncates or
  removes a rollout the parser loses its source. Re-run the parity check after any
  migration.
