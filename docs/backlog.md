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
| Recognize nested `s7s session` calls as `ContextEntryKind::SessionReference` | `session_context/*` | The enum variant is reserved but not produced. Needed before recursive context embedding or deduplication. |

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
| Codex rollback marker after the `item_completed` stream change | Fresh real rollback storage diff proving whether `thread_rolled_back` still exists | [testing.md](./testing.md), rewind row |
| Codex rename target after the local thread catalog appeared | Fresh real rename storage diff identifying the authoritative file/table | [session-title-compat.md](./session-title-compat.md) |
| Antigravity contextual launch | Interactive launch showing prompt injection, successful bootstrap, no historical task execution, and no Q-count pollution | [testing.md](./testing.md#session-context--contextual-launch-checks) |
| Real kitty `ctrl+shift+n` handling | Actual terminal check showing chord distinction and mode cleanup across handover | [testing.md](./testing.md#keyboard-protocol-checks) |
| New Session dropdown over a CJK background title | PTY/TUI visual check confirming no glyph bleed at popup borders | [testing.md](./testing.md), New Session layout row |
