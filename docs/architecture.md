# Architecture Map

A compact current-state map of s7s: where things live and what to run when you
change them. This document links to the domain documents for detailed contracts
rather than repeating them.

## Entry points and execution modes

`src/main.rs` owns CLI startup and the terminal lifecycle. Command parsing is
`clap` derive. Modes:

| Invocation | Mode | Handler |
| --- | --- | --- |
| `s7s` (no command) | Interactive TUI | `ui::App` render/event loop in `main.rs` |
| `s7s session show <id>` / `s7s session search <q>` | CLI context projection | `session_cli::run` |
| `s7s --print` | Dump the session list, no TUI (debug) | `main.rs` |
| `s7s --rebuild-cache` | Force full cache rebuild before the TUI | `scan` |
| `s7s --usage-probe` | Print usage for all profiles and exit (debug) | `usage` |
| `s7s --model-probe` | Print model lists for all profiles and exit (debug) | `models` |
| `s7s demo` | TUI over a disposable mock sandbox under the OS cache dir | `demo` |

## Session scan and cache flow

1. `profile` enumerates profiles (Agent + name + config-folder path). Session
   storage location belongs to the profile (`Profile.path` → `sessions_dir()`),
   not to `config`.
2. `scan` walks each profile's storage, using an mtime-based incremental cache to
   avoid re-parsing unchanged files.
3. `cache` (`<OS cache>/s7s/index.bin`, `0600`) serializes the `Session` index and
   is gated by `CACHE_VERSION` (currently 12); a mismatch discards the cache and
   forces a full rebuild. Bump it only when serialized meaning changes.
4. `filter` applies the composite query (keyword over body+title+folder+last
   assistant answer, AND agent AND folder AND profile) — the same index backs the
   TUI `/` search and `s7s session search`.

## List parsing vs. detailed context parsing

Two parser layers deliberately stay separate (list parsing must remain
lightweight — see [session-context.md](./session-context.md)):

- **List parsers** — `src/parser/{claude,codex,antigravity}.rs` (+ `mod.rs`,
  `turn.rs`) build the lightweight `Session` index: id, title, folder, mtime,
  size, Q (active user-turn count), and redacted search blobs. No tool-call/result
  reconstruction.
- **Context parsers** — `src/session_context/{claude,codex,antigravity}.rs` build
  the detailed `ContextTurn` model consumed by the Detail screen, the
  `s7s session` CLI (`render.rs`), and the handoff exporter (`handoff.rs`).
  `redact.rs` scrubs secrets from every text piece; `excerpt.rs` bounds sizes;
  `resolve.rs` resolves one session across agent/profile boundaries.

Claude and Codex share active-path semantics between the two layers (Claude
`parentUuid` active branch; Codex `thread_rolled_back` rollback). List Q count,
Detail turn count, and CLI turn count must agree — enforced by
`cargo test real_data_turn_parity -- --ignored --nocapture`.

## TUI state / event / render flow

- `ui/mod.rs` — `App` state and the state machine (`UiMode`, key handlers,
  transitions, validation, persistence, and requested external work such as
  rename, rescan, and terminal handover).
- `ui/render.rs` — all screens, dialogs, and components (header, session table,
  preview, detail panels, modals).
- `ui/quick.rs` — the `:` command palette / `!` terminal command window.
- `theme.rs` — palettes, custom theme files, selection persistence.
- Agent handover (`resume.rs`) unmounts the TUI, runs the agent/shell command
  synchronously in the session's folder, then returns to a rescan. `main.rs`
  coordinates the handover screens and input draining.

> Note: `App` currently concentrates screen state, transitions, and effects in
> one module. The staged split into feature-owned state/input/render/effect
> boundaries is described in [refactoring-plan.md](./refactoring-plan.md).

## Usage and model probe flow

- `usage.rs` — drives each agent CLI in a PTY to read remaining % and reset
  countdown; parses the screen; fetched on a background channel. Cross-verify with
  `--usage-probe`. See [usage-display.md](./usage-display.md).
- `models.rs` — enumerates selectable models per agent (`/model` screen scrape,
  `codex debug models`, `agy models`), caches them in `models.json`, and remembers
  the last launched pick. Cross-verify with `--model-probe`. See
  [models.md](./models.md).

## Persistence files and ownership

| Path | Owner | Format | Notes |
| --- | --- | --- | --- |
| `~/.config/s7s/config.toml` | User-edited | TOML | Command templates, editor; self-documenting seeded template |
| `~/.config/s7s/themes/*.toml` | User-edited | TOML | Custom themes overlaying a built-in base |
| `~/.config/s7s/profiles.json` | App-owned | JSON | Profile definitions |
| `~/.config/s7s/models.json` | App-owned | JSON | Per-profile model cache + last selection |
| `~/.config/s7s/theme.json` | App-owned | JSON | Selected theme key |
| `~/.config/s7s/quick_history.json` | App-owned | JSON | `:` palette history |
| `~/.config/s7s/terminal_history.json` | App-owned | JSON | `!` terminal command history |
| `~/.config/s7s/projects/` | App/user | dirs | Project folders created from New Session |
| `<OS cache>/s7s/index.bin` | App-owned | bincode | Session index cache (`0600`), `CACHE_VERSION`-gated |
| `<OS cache>/s7s/demo/` | App-owned | mixed | Disposable demo sandbox |

Rule of thumb: user-edited files are TOML; app-owned state files are JSON.

## Change-request routing

| I want to change… | Source | Required tests (beyond `scripts/check.sh`) |
| --- | --- | --- |
| Rename / session-title | `rename.rs`, `title.rs`, `parser/*` title paths | Manual CLI storage-diff — [session-title-compat.md](./session-title-compat.md) |
| Session list / filter / search | `scan.rs`, `filter.rs`, `parser/*`, `cache.rs` | Real-data parity if turn selection changes |
| Detailed context / `s7s session` CLI | `session_context/*`, `session_cli.rs` | `real_data_turn_parity` — [session-context.md](./session-context.md) |
| Usage display | `usage.rs`, `ui/render.rs` | `--usage-probe` — [usage-display.md](./usage-display.md) |
| Model list / New Session model dropdown | `models.rs`, `ui/mod.rs` | `--model-probe` — [models.md](./models.md) |
| Profiles / env injection | `profile.rs`, `resume.rs` | [profiles.md](./profiles.md) |
| Rewind / backtrack parsing | `parser/claude.rs`, `parser/codex.rs`, `session_context/*` | Real CLI rewind + saved-file diff |
| TUI layout / dialogs / focus | `ui/mod.rs`, `ui/render.rs`, `ui/quick.rs` | `cargo build --release` + PTY/TUI check — [panel-focus-style.md](./panel-focus-style.md) |
| Themes | `theme.rs`, `ui/render.rs` | Render-buffer tests |
| Resume / new-session / terminal handover | `resume.rs`, `main.rs` | Manual handover check |

See [testing.md](./testing.md) for the authoritative verification matrix.
