# Usage Display

> Status: Current and version-sensitive
> Read when: Changing usage probes, parsing, cache state, or rendering.
> Entry points: `src/usage.rs`, `src/usage/{parse,shared}.rs`, `src/probe/`, `src/ui/shared_cache.rs`, `src/ui/render/usage.rs`

The feature to display remaining usage per profile (5h/weekly) in the header. Implemented in `src/usage.rs` (query), `src/usage/parse.rs` (screen parsing), `src/usage/shared.rs` (the cache shared by running instances), and `src/ui/render/usage.rs` (`usage_value_spans` / `usage_tail_span` for the header, `usage_spans` for the New Session dialog), on top of the neutral PTY/process driver in `src/probe/` (shared with the model list query — the driver itself knows nothing about usage).

## Mechanism

- Each CLI is run in an **invisible PTY** (200×60) where usage commands (`/usage` for claude/agy, `/status` for codex) are typed, then parsed by reconstructing the screen with `vt100`.
- Every probe child runs in the **fixed `~/.config/s7s/probe` folder** (`probe::probe_cwd`, created on first use), never in the directory s7s was started from. Agent CLIs gate startup on the working directory, so inheriting it made the result depend on where s7s happened to be launched: claude re-shows its trust dialog for a folder whose `.claude/settings.json` pre-approves permissions, and a project `.mcp.json` asks for approval. Neither screen is the usage screen, so the query failed with `untrusted folder (trust prompt)` for every session started in such a folder — a bare `--%` in the header. The `hasTrustDialogAccepted` flag alone does not trigger the dialog: a folder with no local settings boots fine.
- Keep that folder **empty and dedicated** — a `CLAUDE.md` or `.claude/settings.json` under it would bring the dialogs back. The model list query shares the driver, so the same rule fixes it ([models.md](./models.md)).
- Trust records live in the agent's own config keyed by absolute path (claude: `<config dir>/.claude.json` → `projects["<abs path>"].hasTrustDialogAccepted`; agy: `trustedWorkspaces` in `~/.gemini/antigravity-cli/settings.json`), so nothing placed *inside* the probe folder can pre-approve it — and **agy asks for every workspace missing from its array**. `drive_screen` therefore **auto-confirms the dialog** once per agent config and lets the CLI persist the trust itself. Two gates keep that narrow: the cwd must be the dedicated folder s7s created (`ProbeCwd::dedicated`, never the `current_dir` fallback), and the caret must sit on the `Yes, I trust this folder` row (`trust_approval_selected`) — a reordered menu fails as untrusted rather than taking a blind Enter. A dialog still up 8s after Enter fails with `trust prompt not dismissed by confirm`. While a dialog shows, the driver never types (`/usage` would land in the menu) and never lets agy's boot-time "not signed in" line become `NotLoggedIn`. `--dangerously-skip-permissions` does **not** dismiss it (measured on agy 1.1.8).
- **Only the official client screens** are read without extracting tokens or making unofficial API calls.
- The PTY reader keeps **draining output until the child closes the terminal**, even after the result is parsed. On macOS a CLI whose exit-time output nobody reads never finishes exiting (process state `E`); the cleanup thread's `child.wait()` then blocked forever while holding the PTY master, so every long-running s7s instance leaked one stuck `claude` process, thread, and PTY per Claude probe (58 observed on 2026-10-28, listed by `claude agents` with cwd `~/.config/s7s/probe`). The stuck processes go away when their s7s exits.
- On app startup and upon `ctrl+u`, **all profiles are queried in parallel**. `ctrl+u` is two-phase: the usage/model probes start and one preparing frame (`Loading` pulse + "updating sessions and usage…" status) renders **before** the synchronous session rescan runs, so the loading state is visible while the scan blocks the event loop; repeat `ctrl+u` presses during a cycle merge into it (see `docs/architecture.md`, `ui/effect.rs`). Scenarios like missing config folders (`MissingDir`) or missing means to query (`Unavailable` — Antigravity cannot be queried for additional path profiles as env injection is impossible) are considered part of the query and left as phases. **There is no automatic detection at render time** — folder existence is rechecked only upon explicit refreshes, and the screen maintains its last determination even if folders appear or disappear between refreshes.
- When saving profile additions/edits, an **incremental query of only the saved profile** is run instead of all profiles (`App::start_usage_fetch_for`). Since PTY queries are high-cost operations taking several seconds per profile, the latest values of existing profiles are not reread. Even if a full query is in progress, incremental queries start immediately on a separate channel, and are skipped if the same profile is already Loading.
- For profiles not at the default path, `CLAUDE_CONFIG_DIR`/`CODEX_HOME` are injected into the PTY env to read the usage for that subscription. **They are not injected for the default path** — explicitly doing so triggers a re-login screen, failing the query ([Details](profiles.md)).
- State is managed as `UsageEntry { phase, last, last_at }`; `last_at` (Unix ms, wall clock) is when `last` was read from the CLI and is set together with it. On failure (`Failed`) and on logout (`NotLoggedIn`), `last` is kept and displayed in gray so the value isn't lost — Claude logs out often (Claude Code's own token refresh fails; `~/.claude/daemon.log` `proactive refresh failed`), and a shared logout would otherwise blank every window. Missing folder / not installed / unavailable never show a value.
- While refreshing (`Loading`), a profile with a last value keeps showing it and only its **age pulses** in the header tail slot; a profile without one shows `--%` values and a pulsing `Loading...` tail. The profile table STATUS column shows `Loading...` and the USAGE column keeps the previous value in gray. The query thread ensures a **minimum of 500ms** (`usage.rs::MIN_LOADING`) even for instantaneous determinations like missing folders, so the pulse is perceptible — providing feedback that "a check occurred" even if the result is the same.
- New profile config folders that have not completed login/trust will fail with `untrusted folder (trust prompt)`. Running manually once with that config to approve resolves this.
- Reset countdowns are stored as read and **counted down from `last_at`** at render time (`ResetCountdown::remaining`, rounded up to the minute). Once a reset time passes without a new query, that window shows `(reset)` and its percentage turns gray: the stored percentage no longer applies. The event loop redraws once a minute (`ui/shared_cache.rs`) so ages and countdowns advance while idle.

## Sharing results across running instances

Several s7s windows usually run at once. A query result one instance obtains is shown by every other instance within about a second, and instances avoid running the same CLI query twice (`src/usage/shared.rs`, `src/ui/shared_cache.rs`).

- **Shared file**: `~/.config/s7s/usage.json` (`version` 1), per profile id: `path` (config root the values belong to), `success` (`at_ms` + snapshot), and `attempt` (`at_ms` + outcome: `ready`, `failed`, `not_logged_in`, `not_installed`, `missing_dir`, `unavailable`). A failed attempt never removes `success`. An entry whose `path` differs from the profile's current folder is ignored. Writes merge one profile under `store_lock::with_store_lock` and replace the file atomically. A corrupt file is replaced; a file with a newer `version` is never overwritten.
- **Per-profile probe lock**: `~/.config/s7s/.usage-<profile id>.lock`, taken with non-blocking `flock` (`store_lock::try_lock`). The kernel drops it when the holder exits, so a crash leaves no stale lock.
- **Query flow** (every query: startup, `ctrl+u`, profile save, return from login — `usage::spawn_fetch_shared`):
  1. Lock held by another instance: wait (poll every 200ms, up to 3 minutes). If the holder recorded an attempt after this request, use it — a failure included, so a failing CLI is not retried once per waiting window. Otherwise (the holder only reused a value, or exited mid-query) continue holding the lock. Past 3 minutes the profile shows as failed.
  2. A stored success no older than 60s (`REUSE_MS`), whose latest attempt also succeeded, is used without running the CLI (`Origin::Reused`). Failures and logouts are never reused, so after `claude auth login` a `ctrl+u` queries at once.
  3. Otherwise run the CLI under the lock, store the result, then release the lock.
- **Other instances** check `usage.json` (and `models.json`) modification times once a second and re-read a file only when it changed (`App::poll_shared_caches`). An entry that is `Loading` in this instance keeps its phase and only takes a newer value; otherwise phase and value follow the file. The first check right after startup shows the stored values while the startup query is still running.
- **Status message**: when every profile of a query reused a value, the status reads `usage is up to date` (or `session update complete · … · usage is up to date` after `ctrl+u`) instead of `usage update complete`.
- `--usage-probe` always runs the CLIs and never reads or writes the shared file.

## Screen Format per Agent (based on actual measurements)

Formats may change with external CLI upgrades. The below are based on the versions at the time of verification (claude 2.1.202, agy 1.0.16, codex 0.142.5/0.143.0), re-checked with `--usage-probe` on 2026-10-28 against claude 2.1.312, codex 0.165.0, and agy 1.4.9.

| Item | claude `/usage` | agy `/usage` | codex `/status` |
| :-- | :-- | :-- | :-- |
| % Meaning | `N% used` → remaining = 100−N | Gauge `N%` = remaining | `N% left` = remaining |
| Reset Notation | Absolute time | Relative time | Absolute time |
| Reset Example | `Resets 5am (Asia/Seoul)`, `Resets Jul 10 at 5pm (Asia/Seoul)` | `Refreshes in 16h 51m` | `(resets 04:45)`, `(resets Mon 14:30)`, `(resets Mon Jul 10)`, `(resets 17:33 on 15 Jul)` |
| 5h Label | `Current session` | `Five Hour Limit` | `5h limit:` |
| Weekly Label | `Current week (all models)` | `Weekly Limit` | `Weekly limit:` |

Notes:

- **Absolute times are not countdowns.** `resets 04:45` in codex means "resets at 4:45". If a date is attached like `resets 17:33 on 15 Jul`, it is viewed as an absolute time on that date and interpreted as the next occurring instance based on local time, converting it into a countdown.
- **agy has model groups** (`GEMINI MODELS` / `CLAUDE AND GPT MODELS`, and since 1.4.9 `GEMINI 4 ARGON`). The group is chosen from the active model name in the bottom status bar: `Argon` → `GEMINI 4 ARGON`, other `Gemini` → `GEMINI MODELS`, anything else → `CLAUDE AND GPT MODELS`. A group section ends at the next all-uppercase heading. The Argon bucket has only a weekly limit (`Gemini 4 Argon Weekly Limit`), so its 5h window is empty. The Argon footer text is inferred from the model's display name (`Gemini 4 Argon (High)`) and not yet observed with Argon active.
- **agy 5h `Quota available`** (1.4.9): a full 5h window prints no `Refreshes in`, so the window has no countdown.
- **codex accounts without a 5h limit** (0.165.0, Pro plan): `/status` prints only `Weekly limit:`; the 5h window is empty (`--%`). Another account on the same version still prints both.
- **agy 5h `Disabled`**: When the weekly limit is exhausted, the 5h limit is disabled, and the message `Disabled: … will fully refresh in 16 hours, 37 minutes.` appears. In this case, 0% remaining + the refresh time in the message is used as the countdown. The PTY width is set to 200 so this message (~150 characters) is not truncated.

## Display Format

Fixed-width columns are maintained across all states to align vertically between rows. Each row ends with a tail slot (`TAIL_W` = 13, the width of `Not logged in`):

```
<1> Claude        72%(4h 30m)  52%(2d 16h) left  just now
<2> Antigravity    0%(17h  6m)   0%(   17h) left  2h ago
<3> Codex         95%(4h 15m)  51%(reset)  left  Error
<4> CDX-Share     --%          --%         left  Not logged in
```

- Tail slot by state: success → age (`just now` under a minute, then `5m ago`, `1h ago` — floored, 1h 30m reads `1h ago` — and `3d ago`; always shown, dim); `Loading` with a value → the age, pulsing; `Loading` without one → `Loading...`, pulsing; `Failed` and missing folder → `Error` (red); logged out → `Not logged in` (warning color); not installed → `Not installed` (dim); unavailable (extra Antigravity profile) → `Unavailable` (dim). The header no longer prints `Config folder not found`; the profile table USAGE cell still does.
- The New Session dialog keeps its compact form (`usage_spans`): values without the tail slot, or a single label where no value exists.

- % is right-aligned with width 3. Blue for >=50%, Red for <50%. No value is `--%`.
- In the header, if either current (5h) or weekly is `0%`, both usage segments
  use the theme's faint-text treatment, matching `left`.
- current(5h) countdown: `(4h 30m)` — minutes right-aligned with width 2.
- weekly countdown: minutes omitted, `(2d 16h)` / `(   17h)`; under one hour it falls back to minutes, `(   45m)`, instead of `(    0h)`.
- The profile screen table shows two remaining-usage columns, `5H(LEFT)` and
  `1W(LEFT)`. Each combines the percentage and reset countdown in the same
  format as the header profile list: `72%(4h 30m)` / `52%(2d 16h)`. The
  following `UPDATED` column (width 8) shows the value's age, or `-` before
  any success, including for inactive profiles that the header omits.
- On the profile screen, if either current(5h) or weekly is `0%` based on the latest snapshot, the entire row is displayed in the same light gray as the `left` text.
- **Profiles determined to have missing config folders** (`UsagePhase::MissingDir` — deleted, renamed, etc.) display `Error` in the header tail slot and `Config folder not found` (Red, `MISSING_DIR_LABEL`) in the USAGE cell (width 26) of the profile table, while maintaining `Error` in the STATUS cell so they read side-by-side (ratatui Table cells cannot overflow into adjacent columns, so the adjacent USAGE cell is used instead of STATUS). If inactive, it is submerged with a soft dim. The determination is based on query time; `is_dir()` checks are not made at render time.
- While usage is refreshing (`Loading`), only the header tail slot (age or
  `Loading...`) and the profile table STATUS cell blink with a fade pulse. The
  rest of the row stays stable. Each step is 200ms against the 100ms runtime polling
  cycle. The invisible step replaces content with equal-width spaces instead of
  relying on terminal conceal support. Implementation is
  `PULSE_SEQ`/`pulse_span` in `src/ui/render/usage.rs`.

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
- Sharing (manual, release build): open two s7s windows. `ctrl+u` in one; the other's ages change to `just now` within about a second without input, and only one `claude`/`codex`/`agy` per profile runs (`ps`). `ctrl+u` again within a minute reports `usage is up to date` without starting a CLI. With `claude agents` (or `ps -axo stat,ppid,command | awk '$1 ~ /E/'`), confirm no probe process outlives its query.
- If the absolute/relative notation is ambiguous, **capture twice a few minutes apart**: if the numbers stay the same, it's absolute time; if they decrease, it's a countdown.
- Fixtures in unit tests (`src/usage/parse_tests.rs`) use actual screen captures, and time-dependent logic is verified by injecting a fixed `now`.
