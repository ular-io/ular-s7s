# Backlog and Verification Debt

> Status: Current
> Read when: Planning the next feature or auditing unresolved compatibility risk.
> This file records candidates and gaps; it does not authorize implementation.

Keep this file short. Remove completed entries in the same change that completes
them, and move enduring behavior into the owning domain document.

## Confirmed unimplemented behavior

| Item | Primary owner | Notes |
| --- | --- | --- |
| Select/override a model when resuming a session | `resume.rs`, resume UI flow, `models.rs` | New Session supports model selection; resume does not. Verify the resume flag interactively for every agent before designing the UI. |
| Count a Codex message sent while a turn is running as part of that turn | `parser/codex/events.rs`, both consumers | Codex `thread/turns/list` groups such a message into the running turn (same `turn_id`); s7s opens a new question for it, so its Q count exceeds the CLI's (seen: 4 vs 3). Decide whether s7s should follow the CLI before changing turn selection. |
| Recognize nested `s7s session` calls as `ContextEntryKind::SessionReference` | `session_context/*` | The enum variant is reserved but not produced. Needed before recursive context embedding or deduplication. |
| Open dropdowns upward when the rows below are too few | `ui/new_session/render.rs`, `ui/workspace/render.rs` (`draw_folder_list`) | The shared rust-tui rule (controls §4) flips a popup above its combo when it does not fit below; s7s always opens downward and clamps at the terminal bottom. Decide how the upward popup joins the combo's top border before sharing it through `components::modal::dropdown_frame`. |
| New Session errors on a notice line | `ui/new_session/render.rs` | The rust-tui standard puts a form's error on a reserved notice line. New Session truncates its error left of the buttons (about 16 cells on an 80-column terminal); moving it to a notice line adds one reserved row to the dialog. |
| Bold names in the `Folders` summary | `ui/workspace/render.rs` (`folder_combo`) | rust-tui controls §7.4 draws the selected names in the combo value style (bold) with only the count dim; s7s draws them in the default weight. |

## Deferred ideas — not committed

| Candidate | Likely owner | Main decision required |
| --- | --- | --- |
| JSON/NDJSON output for `session show/search` | `session_cli.rs`, `session_context/render.rs` | Stable schema/versioning and interaction with `--bootstrap` |
| Partial session-ID resolution | `session_context/resolve.rs` | Minimum prefix and ambiguity behavior |
| Turn ranges for very large sessions | `session_cli.rs`, `session_context/render.rs` | CLI shape and output limits |
| Per-result keyword snippets in session search | `session_cli.rs`, list index | Index size and snippet source |
| Configurable bootstrap response language | bootstrap rendering/config | Trust boundary and default behavior |
| Cross-session reference graph/deduplication | parser/context model | Requires nested-reference recognition first |

## Verification debt

| Gap | Required evidence | Procedure |
| --- | --- | --- |
| Codex rewind segments from the terminal TUI | A real esc-esc rewind in the `codex` TUI showing whether it writes a `_<segment id>` rollout with `history_base` like the VS Code extension (0.159.2/0.160.0), or still a `thread_rolled_back` marker | [testing.md](./testing.md), rewind row |
| Codex rewind segment chains | A thread rewound twice, showing which file and ordinal the second segment's `history_base` refers to; `segments::read_rollout` assumes the newest earlier file and logical-stream ordinals | [testing.md](./testing.md), rewind row |
| Flaky bookmark tests | `TempBookmarkStore` names its directory by process id and wall-clock nanos, which macOS rounds to microseconds, so parallel tests can share one store; `bookmarks::tests::*` fail intermittently and block `scripts/check.sh` (also on the pre-fix base). Needs a collision-free name and repeated green runs | `cargo test -q --lib bookmarks`, repeated |
| Rollout integrity after `codex migrate-rollouts --apply` | One migrated thread showing the rollout JSONL unchanged in size and line count, then `real_data_turn_parity` over the whole index | [testing.md](./testing.md), session context checks |
| Antigravity contextual launch | Interactive launch showing prompt injection, successful bootstrap, no historical task execution, and no Q-count pollution | [testing.md](./testing.md#session-context--contextual-launch-checks) |
| Real kitty `ctrl+shift+n` handling | Actual terminal check showing chord distinction and mode cleanup across handover | [testing.md](./testing.md#keyboard-protocol-checks) |
| New Session dropdown over a CJK background title | PTY/TUI visual check confirming no glyph bleed at popup borders | [testing.md](./testing.md), New Session layout row |
