# Usage Display

The feature to display remaining usage per profile (5h/weekly) in the header. Implemented in `src/usage.rs` (query/parsing) and `src/ui/render.rs::usage_spans` (formatting), on top of the neutral PTY/process driver in `src/probe/` (shared with the model list query — the driver itself knows nothing about usage).

## Mechanism

- Each CLI is run in an **invisible PTY** (200×60) where usage commands (`/usage` for claude/agy, `/status` for codex) are typed, then parsed by reconstructing the screen with `vt100`.
- **Only the official client screens** are read without extracting tokens or making unofficial API calls.
- On app startup and upon `ctrl+u`, **all profiles are queried in parallel**. `ctrl+u` is two-phase: the usage/model probes start and one preparing frame (`Loading` pulse + "updating sessions and usage…" status) renders **before** the synchronous session rescan runs, so the loading state is visible while the scan blocks the event loop; repeat `ctrl+u` presses during a cycle merge into it (see `docs/architecture.md`, `ui/effect.rs`). Scenarios like missing config folders (`MissingDir`) or missing means to query (`Unavailable` — Antigravity cannot be queried for additional path profiles as env injection is impossible) are considered part of the query and left as phases. **There is no automatic detection at render time** — folder existence is rechecked only upon explicit refreshes, and the screen maintains its last determination even if folders appear or disappear between refreshes.
- When saving profile additions/edits, an **incremental query of only the saved profile** is run instead of all profiles (`App::start_usage_fetch_for`). Since PTY queries are high-cost operations taking several seconds per profile, the latest values of existing profiles are not reread. Even if a full query is in progress, incremental queries start immediately on a separate channel, and are skipped if the same profile is already Loading.
- For profiles not at the default path, `CLAUDE_CONFIG_DIR`/`CODEX_HOME` are injected into the PTY env to read the usage for that subscription. **They are not injected for the default path** — explicitly doing so triggers a re-login screen, failing the query ([Details](profiles.md)).
- State is managed as `UsageEntry { phase, last }`. On failure (`Failed`), `last` (the last successful snapshot) is maintained and displayed in gray so the value isn't lost.
- While refreshing (`Loading`), regardless of the previous determination (including logged out/uninstalled/missing folder), `Loading...` is displayed **consistently across all profiles** in both the header usage slot and the profile table STATUS column (regardless of whether a previous value exists). The profile table USAGE column retains the previous value in gray. The query thread ensures a **minimum of 500ms** (`usage.rs::MIN_LOADING`) even for instantaneous determinations like missing folders, so the blink is perceptible — providing feedback that "a check occurred" even if the result is the same. (The past sticky blocked display — keeping the blocked message during requeries — has been removed and replaced by this consistent display.)
- New profile config folders that have not completed login/trust will fail with `untrusted folder (trust prompt)`. Running manually once with that config to approve resolves this.
- The result is a snapshot, and the countdown does not decrease in real-time while the TUI is open.

## Screen Format per Agent (based on actual measurements)

Formats may change with external CLI upgrades. The below are based on the versions at the time of verification (claude 2.1.202, agy 1.0.16, codex 0.142.5/0.143.0).

| Item | claude `/usage` | agy `/usage` | codex `/status` |
| :-- | :-- | :-- | :-- |
| % Meaning | `N% used` → remaining = 100−N | Gauge `N%` = remaining | `N% left` = remaining |
| Reset Notation | Absolute time | Relative time | Absolute time |
| Reset Example | `Resets 5am (Asia/Seoul)`, `Resets Jul 10 at 5pm (Asia/Seoul)` | `Refreshes in 16h 51m` | `(resets 04:45)`, `(resets Mon 14:30)`, `(resets Mon Jul 10)`, `(resets 17:33 on 15 Jul)` |
| 5h Label | `Current session` | `Five Hour Limit` | `5h limit:` |
| Weekly Label | `Current week (all models)` | `Weekly Limit` | `Weekly limit:` |

Notes:

- **Absolute times are not countdowns.** `resets 04:45` in codex means "resets at 4:45". If a date is attached like `resets 17:33 on 15 Jul`, it is viewed as an absolute time on that date and interpreted as the next occurring instance based on local time, converting it into a countdown.
- **agy has 2 model groups** (`GEMINI MODELS` / `CLAUDE AND GPT MODELS`). The group is chosen to read based on the active model name in the bottom status bar of the screen.
- **agy 5h `Disabled`**: When the weekly limit is exhausted, the 5h limit is disabled, and the message `Disabled: … will fully refresh in 16 hours, 37 minutes.` appears. In this case, 0% remaining + the refresh time in the message is used as the countdown. The PTY width is set to 200 so this message (~150 characters) is not truncated.

## Display Format

Fixed-width columns are maintained across all states to align vertically between rows:

```
<1> Claude        72%(4h 30m)  52%(2d 16h) left
<2> Antigravity    0%(17h  6m)   0%(   17h) left
<3> Codex         95%(4h 15m)  51%(    2h) left
```

- % is right-aligned with width 3. Blue for >=50%, Red for <50%. Loading is a spinner (`✽✻✶%`), failure is `--%`.
- In the header, if either current(5h) or weekly is `0%`, both usage segments are displayed in a dim gray (`Color::Gray` + `Modifier::DIM`), the same as `left`.
- current(5h) countdown: `(4h 30m)` — minutes right-aligned with width 2.
- weekly countdown: minutes omitted, `(2d 16h)` / `(   17h)`.
- The profile screen table splits the usage into four columns: `5H` / `RESET` / `1W` / `RESET`, omitting the parentheses in the reset columns.
- On the profile screen, if either current(5h) or weekly is `0%` based on the latest snapshot, the entire row is displayed in the same light gray as the `left` text.
- **Profiles determined to have missing config folders** (`UsagePhase::MissingDir` — deleted, renamed, etc.) display `Config folder not found` (Red, `MISSING_DIR_LABEL`) instead of usage — displayed in the usage slot in the header, and in the USAGE cell (width 30) of the profile table, while maintaining `Error` in the STATUS cell so they read side-by-side (ratatui Table cells cannot overflow into adjacent columns, so the adjacent USAGE cell is used instead of STATUS). If inactive, it is submerged with a soft dim. The determination is based on query time; `is_dir()` checks are not made at render time.
- While usage is refreshing (`Loading`), only the `Loading...` text (header usage slot, profile table STATUS cell) blinks with a fade pulse. The rest of the cells in the row do not blink: normal → light (fg 60% attenuated) → lighter (soft dim like `left` label) → invisible (replaced with space of the same width) → lighter → light cycle. Each step is 200ms (cycle 1.2s) — double the redraw polling cycle (100ms, `main.rs`) to ensure steps aren't skipped by aliasing. The invisible step is processed by space replacement rather than the HIDDEN (conceal) attribute due to varying terminal support. Implementation is `PULSE_SEQ`/`pulse_span` in `src/ui/render.rs`.

## Verification Methods

When parsing code or external CLIs change, do not rely solely on unit tests; verify with actual measurements.

```bash
# Output only the query results for the three agents without TUI
cargo build && ./target/debug/s7s --usage-probe

# Dump the final screen text of each CLI to a file (for parser debugging).
# Filename is `<cli>-<slash_command>.screen.txt` (e.g., claude-usage.screen.txt —
# distinguish from model list query `claude-model.screen.txt`, refer to docs/models.md)
mkdir -p /tmp/dump && ULAR_USAGE_DUMP=/tmp/dump ./target/debug/s7s --usage-probe
```

- Visually check if the countdowns in the probe results match the actual CLI screens.
- If the absolute/relative notation is ambiguous, **capture twice a few minutes apart**: if the numbers stay the same, it's absolute time; if they decrease, it's a countdown.
- Fixtures in unit tests (`src/usage.rs` `tests`) use actual screen captures, and time-dependent logic is verified by injecting a fixed `now`.
