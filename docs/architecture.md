# Architecture Map

A compact current-state map of s7s: where things live and what to run when you
change them. This document links to the domain documents for detailed contracts
rather than repeating them.

## Entry points and execution modes

The application is a library crate (`src/lib.rs`) with a thin binary shim
(`src/main.rs`) that only calls `s7s::run`. CLI dispatch, the TUI event loop,
agent handovers, and the terminal lifecycle live in `src/runtime.rs`. Command
parsing is `clap` derive. Modes:

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

- `ui/mod.rs` — `App` state and the cross-feature state machine (`UiMode`,
  `Screen`, `App` fields, session rescan/refresh, filter clearing, screen
  switching, resume/terminal requests, and the session-deletion filesystem work
  invoked by the delete dialog). The per-feature key handlers and the overlay
  handlers now live in their feature/overlay modules.
- `ui/render.rs` — the shared frame chrome: the full-frame `draw` dispatcher, the
  `draw_header`/`draw_body`/`draw_status_bar` sub-dispatchers, the New Session
  project-directory confirmation (`draw_project_dir_confirm`), and the shared
  helpers (`session_meta_lines`, `preview_turn_lines`, `agent_tag`, `input_view`,
  `centered_fixed_rect`, usage/pulse formatting) reused across features. The
  full-frame Session, Detail, and representative-modal (backdrop dimming) render
  tests stay here with the `draw` dispatcher (§9.6).
- `ui/components/` — feature-agnostic UI primitives reused across dialogs:
  `input` (Unicode-safe `TextInput`), `modal` (frame/buttons/backdrop),
  `scrollbar`, and `text` (width-aware truncation/wrapping).
- `ui/new_session/` — extracted feature module (R6): `state` (dialog state,
  focus, model/source options, pure transitions), `input` (the `App` key handling
  and launch logic), and `render` (the dialog and dropdown overlay). The public
  dialog types are re-exported from `ui` so `crate::ui::NewSession*` paths stay
  stable.
- `ui/profile/` — extracted feature module (R7): `state` (`FormFocus` /
  `ProfileFormState` and the form focus/agent-cycle transitions), `input` (the
  `App` key handling for the profile table, add/edit form, deletion, and
  config-directory confirmation, plus profile persistence and the login request),
  and `render` (the profile table with the merged usage cell and the form/delete/
  dir-confirm modals). `FormFocus` / `ProfileFormState` are re-exported from `ui`
  so `crate::ui::FormFocus` paths stay stable. Usage/model fetch coordination,
  `set_single_profile` (header number-key filter), `profile_name`,
  `session_profile_root`, and the Antigravity metadata cleanup remain in
  `ui/mod.rs` as cross-feature `App` coordination.
- `ui/detail/` — extracted feature module (R8a): `state` (`DetailFocus` /
  `SessionDetailState` and the question-selection / right-panel-scroll
  transitions), `input` (the `App` key handling — open/close plus in-screen
  navigation, focus toggle, tool-log visibility, and the resume/rename/delete/
  contextual-New-Session operations shared with the main view), and `render`
  (the two-column Prompt list and Work & Answer panel). `DetailFocus` /
  `SessionDetailState` are re-exported from `ui` so `crate::ui::DetailFocus`
  paths stay stable. The full-frame Detail render tests stay with the `draw`
  dispatcher in `ui/render.rs` (§9.6).
- `ui/session/` — extracted feature module (R8b): `state` (only the `Focus`
  enum — the rest of the Session screen state lives in `App` fields, whose §8.1
  split is deferred), `input` (the `App` key handling — `on_key_table`,
  `on_key_keyword`, and the private table-selection / preview-scroll helpers), and
  `render` (the session table, the per-turn preview panel, and the `/` keyword
  search prompt). `Focus` is re-exported from `ui` so `crate::ui::Focus` stays
  stable. Cross-feature filter coordination (`clear_all_filters`,
  `set_single_profile`, `recompute`, `switch_screen`) and the filter/rename/delete
  overlays remain in `ui/mod.rs`; the `draw`/`draw_body` dispatchers and the shared
  preview helpers remain in `ui/render.rs`.
- `ui/overlays/` — extracted overlay modules (R9), each a single file combining
  state, `App` key handling, and rendering (ownership is clear per overlay, so no
  four-file split — §7): `filters` (agent/folder multi-select modals +
  `ModalState`), `confirm` (session rename/delete dialogs + `RenameFocus` /
  `RenameModalState`), `message` (the reusable `show_message` alert +
  `MessageKind` / `MessageDialog`), `help` (the `?` shortcuts screen), and
  `theme` (the theme selection dialog + `ThemeSelectState`). The overlay state
  types are re-exported from `ui` so `crate::ui::{ModalState, RenameFocus,
  RenameModalState, MessageKind, MessageDialog, ThemeSelectState}` stay stable.
  The session-deletion filesystem work stays in `ui/mod.rs`.
- `ui/quick.rs` — the `:` command palette / `!` terminal command window; owns its
  state, key handling, and rendering (`draw_quick_command` moved here in R9 to
  complete its state/input/render boundary — §9.5).
- `theme.rs` — palettes, custom theme files, selection persistence.
- Agent handover (`resume.rs`) unmounts the TUI, runs the agent/shell command
  synchronously in the session's folder, then returns to a rescan. `main.rs`
  coordinates the handover screens and input draining.

> Note: `App` still concentrates the cross-feature state, transitions, and
> effects in `ui/mod.rs`; New Session (R6), Profile (R7), the Detail screen
> (R8a), the Session screen (R8b), and the overlays (R9) are the features carved
> into their own state/input/render modules. The remaining staged split into
> effect-based boundaries (App effects and background coordination — R10) is
> described in [refactoring-plan.md](./refactoring-plan.md).

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
| Session list / filter / search | `scan.rs`, `filter.rs`, `parser/*`, `cache.rs`, `ui/session/*` | Real-data parity if turn selection changes |
| Detailed context / `s7s session` CLI | `session_context/*`, `session_cli.rs` | `real_data_turn_parity` — [session-context.md](./session-context.md) |
| Detail screen (turn list / work panel) | `ui/detail/*` | `cargo build --release` + PTY/TUI check — [panel-focus-style.md](./panel-focus-style.md) |
| Usage display | `usage.rs`, `ui/render.rs` | `--usage-probe` — [usage-display.md](./usage-display.md) |
| Model list / New Session model dropdown | `models.rs`, `ui/new_session/*` | `--model-probe` — [models.md](./models.md) |
| Profiles / env injection | `profile.rs`, `ui/profile/*`, `resume.rs` | [profiles.md](./profiles.md) |
| Rewind / backtrack parsing | `parser/claude.rs`, `parser/codex.rs`, `session_context/*` | Real CLI rewind + saved-file diff |
| TUI layout / dialogs / focus | `ui/mod.rs`, `ui/render.rs`, `ui/session/*`, `ui/new_session/*`, `ui/profile/*`, `ui/detail/*`, `ui/overlays/*`, `ui/quick.rs` | `cargo build --release` + PTY/TUI check — [panel-focus-style.md](./panel-focus-style.md) |
| Themes | `theme.rs`, `ui/render.rs` | Render-buffer tests |
| Resume / new-session / terminal handover | `resume.rs`, `main.rs` | Manual handover check |

See [testing.md](./testing.md) for the authoritative verification matrix.
