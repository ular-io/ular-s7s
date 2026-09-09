# Architecture Map

> Status: Current
> Read when: A change crosses feature boundaries or its owning module is unclear.
> Entry points: `src/lib.rs`, `src/runtime.rs`, `src/ui/mod.rs`

This document maps current responsibilities and data flow. Feature contracts and
verification details belong to the domain documents routed from `AGENTS.md`.

## Execution modes

| Invocation | Owner |
| --- | --- |
| `s7s` | `runtime::run_loop` driving `ui::App` |
| `s7s <dir>` | `runtime::resolve_startup_dir` then the New Session dialog |
| `s7s session show/search/list/rename/delete/handoff` | `session_cli::run` |
| `s7s --print`, `--rebuild-cache` | `runtime`, `scan`, `cache` |
| `s7s --usage-probe`, `--model-probe` | `usage`, `models`, `probe` |
| `s7s demo` | `demo` |
| `s7s version`, `-v`, `--version` | `runtime` |

`src/main.rs` is a thin binary shim. `src/lib.rs` exposes `run`; command parsing,
the event loop, handovers, and terminal ownership live in `src/runtime.rs`.

## Session index flow

```text
ProfileStore
  -> scan profile-owned storage
  -> lightweight agent parser
  -> Session index
  -> cache/index.bin
  -> filter
  -> TUI list or `s7s session search`
```

- `profile.rs` owns agent/profile configuration roots. Storage paths must derive
  from `Profile.path`, never from a global default when a profile is known.
- `scan.rs` uses physical mtime only for cache freshness. Display ordering uses
  semantic activity time: the later of the last active user submission and last
  active response completion.
- `cache.rs` owns the versioned bincode index. Do not repeat the current cache
  version in documentation; change its constant when serialized meaning changes.
- `filter.rs` applies keyword, agent, folder, and profile filters. The keyword
  index includes user text, title, folder, last assistant text, and sufficiently
  long session-ID tokens.

## Parser boundaries

List parsing must remain lightweight. Detailed context parsing may reconstruct
assistant and tool records.

| Agent | List parser | Detailed parser | Active-path rule |
| --- | --- | --- | --- |
| Claude | `parser/claude/` | `session_context/claude.rs` | shared `events` decoder and `parentUuid` chain reduction |
| Codex | `parser/codex/` | `session_context/codex.rs` | shared `events` decoder; consumers apply `thread_rolled_back` truncation |
| Antigravity | `parser/antigravity.rs` over SQLite | `session_context/antigravity.rs` over transcript JSONL | different stores; no shared decoder |

All parsers use `parser::{clean_turn, is_noise_turn}`. Claude and Codex event
decoders classify records without materializing tool payloads; detailed parsers
extract payloads from the raw records. Antigravity's list parser reuses
`session_context::antigravity::parse_turns` only for last-assistant-text search
indexing because that text is absent from its SQLite store.

Invariant: List Q count, Detail turn count, and `s7s session show` turn count
must agree under the rules in [session-context.md](./session-context.md).

## Detailed context flow

```text
Session
  -> session_context::load
  -> agent detailed parser
  -> redaction and excerpt limits
  -> Detail UI / session CLI / handoff exporter
```

- `session_context/model.rs` owns `SessionContext`, turns, entries, and
  completeness.
- `resolve.rs` resolves a source across agent/profile boundaries without unsafe
  fallback.
- `render.rs` owns reference, one-turn, and bootstrap text projections.
- `handoff.rs` is a compatibility adapter and Markdown exporter, not a second
  parsing implementation.

See [session-context.md](./session-context.md) before changing this flow.

## TUI ownership

`ui::App` owns cross-feature state and coordinates feature modules. Key handlers
enqueue in-place work through `AppEffect`; terminal-unmounting handovers remain
discrete request fields drained by `runtime`.

| Area | Owner |
| --- | --- |
| Shared frame, header, status, shared render helpers | `ui/render.rs` |
| Session list/search/preview | `ui/session/` |
| Detail screen | `ui/detail/` |
| New Session dialog | `ui/new_session/` |
| Profile screen/forms | `ui/profile/` |
| Filters, confirmation, help, theme, messages | `ui/overlays/` |
| Quick Command and terminal-command input | `ui/quick/` |
| Context-source navigation stack | `ui/context_jump.rs` |
| Clipboard projections | `ui/copy.rs` |
| Paste routing | `ui/paste.rs` |
| Reusable input/modal/scroll/text primitives | `ui/components/` |
| Synchronous external effects | `ui/effect.rs` |
| Usage/model receiver coordination | `ui/background.rs` |

Pure state recomputation is not an effect. Usage/model results stay on `App`;
`BackgroundState` owns only receivers and in-flight coordination. Resume, new
session, login, and terminal commands suspend the TUI, run synchronously through
`resume.rs`, then restore and rescan.

See [ui-style-guide.md](./ui-style-guide.md) for visual changes and
[terminal-input-hardening.md](./terminal-input-hardening.md) for input or
terminal ownership changes.

## Probe flow

- `probe/pty.rs` owns PTY lifecycle, screen capture, stabilization, and cleanup.
- `probe/process.rs` owns process discovery and termination helpers.
- `usage.rs` and `models.rs` are independent clients and own their parsing.
- Probe children run in the fixed s7s probe directory, not the launch directory.

Validate both clients when changing shared probe behavior. Agent-specific rules
live in [usage-display.md](./usage-display.md) and [models.md](./models.md).

## Scratch workspace

`scratch.rs` owns `~/.config/s7s/scratch`, the working directory behind the fixed
`[SCRATCH]` row of the New Session folder dropdown. Sessions started there run
without a project.

- `runtime::handover_new_session` calls `scratch::prepare` immediately before the
  handover: it deletes every direct entry except `AGENTS.md`/`CLAUDE.md`, then
  rewrites both from the source constants. An earlier session's leftovers must
  never become the next session's context — a stale `CLAUDE.md` in the working
  directory would be loaded by the agent CLI on startup.
- `prepare` deletes without a confirmation prompt, so it verifies the target is
  the scratch workspace instead of trusting the caller, touches direct entries
  only, and unlinks symlinks rather than following them.
- The policy text is the redirect mechanism, not the folder permissions: the
  agent runs as the same user and could restore any write bit. `AGENTS.md` holds
  the policy and `CLAUDE.md` imports it, so both agent families read one source.
  It must keep telling the agent to ask the user for a target directory —
  blocking writes without naming an alternative only moves the file somewhere
  unpredictable.
- `scratch::folder_label` renames the workspace to `[SCRATCH]` in the session
  list, the metadata grids, and clipboard projections; those keep the full path
  where they already showed one. `Session.folder` stays the raw basename, so the
  cached index, the folder filter identity, and the search blob are untouched.
- The dialog offers the workspace as a row outside `folders`/`ordered` and fills
  the input with its real path on selection, so confirming runs the ordinary
  validation and launch path. `folders` excludes the workspace so it cannot also
  appear as an ordinary folder once sessions exist there.

## Persistence ownership

| Path | Owner | Format |
| --- | --- | --- |
| `~/.config/s7s/config.toml` | user | TOML |
| `~/.config/s7s/themes/*.toml` | user | TOML |
| `~/.config/s7s/profiles.json` | app | JSON |
| `~/.config/s7s/models.json` | app | JSON |
| `~/.config/s7s/theme.json` | app | JSON |
| `~/.config/s7s/{quick,terminal}_history.json` | app | JSON |
| `~/.config/s7s/session_workspaces.json` | app | JSON; cwd captured for s7s-created sessions whose agent store omits it |
| `~/.config/s7s/projects/` | app/user | directories |
| `~/.config/s7s/scratch/` | app | shared working directory, emptied on every launch |
| `<OS cache>/s7s/index.bin` | app | versioned bincode, mode `0600` |
| `<OS cache>/s7s/demo/` | app | disposable demo data |

Rule: user-edited configuration is TOML; app-owned state is JSON; the session
index is a disposable cache.

## Cross-cutting change map

| Change | Primary source | Required contract |
| --- | --- | --- |
| Scan, list, filter | `scan.rs`, `filter.rs`, `parser/*`, `cache.rs` | `session-context.md` when turn selection changes |
| Rename/title | `rename.rs`, `title.rs`, parser title paths | `session-title-compat.md` |
| Session deletion | `session_delete.rs` (shared by `ui/effect.rs` and `session_cli.rs`) | `session-context.md` §Delete |
| Work handoff | `session_handoff.rs`, `config.rs` (`handoff_instruction`) | `session-context.md` §Handoff |
| Detailed context/CLI | `session_context/*`, `session_cli.rs` | `session-context.md` |
| Usage/model probes | `usage.rs`, `models.rs`, `probe/*` | `usage-display.md`, `models.md` |
| Profiles/env injection | `profile.rs`, `resume.rs`, `ui/profile/*` | `profiles.md` |
| TUI/input/terminal | `ui/*`, `runtime.rs` | `ui-style-guide.md`, `terminal-input-hardening.md` |
| Scratch workspace / its policy text | `scratch.rs`, `ui/new_session/*`, `runtime.rs` | §Scratch workspace above, `testing.md` |
| Release | `scripts/release.sh` | `releasing.md` |

[testing.md](./testing.md) is the authoritative verification matrix.
