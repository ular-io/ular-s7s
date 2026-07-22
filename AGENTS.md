# Repo Agent Notes

s7s is a Rust terminal UI (ratatui/crossterm) for browsing, searching, renaming,
and resuming coding-agent sessions (Claude Code, Codex, Antigravity) across
multiple profiles, plus a `s7s session` CLI that projects stored session context.
It reads each agent's on-disk session storage directly, so much of its behavior is
coupled to external CLI formats that can change on upgrade.

Start from [docs/architecture.md](./docs/architecture.md) for the module map and a
change-request-to-source routing table.

## Routing: read before you change

Read the linked document before modifying code in that area.

| If you change… | Read first |
| --- | --- |
| Rename / session-title | [session-title-compat.md](./docs/session-title-compat.md) + [testing.md](./docs/testing.md) |
| Usage display / parsing | [usage-display.md](./docs/usage-display.md) |
| Profiles / env injection (`CLAUDE_CONFIG_DIR` / `CODEX_HOME`) | [profiles.md](./docs/profiles.md) |
| Model list / New Session model dropdown | [models.md](./docs/models.md) |
| Session context (`src/session_context/`, `s7s session` CLI, New Session with Context) | [session-context.md](./docs/session-context.md) |
| TUI panel focus / visual style | [panel-focus-style.md](./docs/panel-focus-style.md), [preview-omission-style.md](./docs/preview-omission-style.md), [ui-style-guide.md](./docs/ui-style-guide.md) |
| Release process | [releasing.md](./docs/releasing.md) |

For the reasoning behind a past change, consult
[docs/development-history.md](./docs/development-history.md) — an archived,
non-authoritative log. Do not read it by default.

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

## Current hazards

- The standing risk is external-CLI drift: storage layouts, option behavior, and
  screen output can change between CLI versions even under identical flags. Treat
  every parser and probe as version-specific and re-verify on upgrade.
- Some paths have only been validated indirectly (e.g. agy contextual launch and
  real-kitty `ctrl+shift+n`). Such gaps are recorded with their iteration number
  in [development-history.md](./docs/development-history.md).
