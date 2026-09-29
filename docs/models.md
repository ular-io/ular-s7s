# Model Selection (New Session Model Dropdown)

> Status: Current and version-sensitive
> Read when: Changing model discovery, caching, selection, or command injection.
> Entry points: `src/models.rs`, `src/ui/new_session/`, `src/resume.rs`

Contract for querying, caching, and injecting selectable models in the New
Session dialog. `src/models.rs` owns query/cache data,
`src/ui/new_session/` owns dialog state and interaction, `src/ui/background.rs`
coordinates probes, and `src/resume.rs::with_model_flag` injects the command.

## Model List Enumeration Methods (Observed September 2026)

| Agent | Method | Value Format | Default Model Source |
| :-- | :-- | :-- | :-- |
| claude | Scraping `/model` screen via PTY (`probe::pty::drive_screen`, shared with usage) | pinned model ID for versioned rows (`claude-fable-5-1`); alias for older unversioned rows (`opus[1m]`) | `✔` mark in the screen list |
| codex | `codex debug models` JSON (only `visibility=="list"`) | slug (`gpt-5.6-sol`) | Top-level `model` key in `<CODEX_HOME>/config.toml` |
| agy | `agy models`, one `slug<TAB>display name` row per model | slug (`gemini-3.6-flash-high`); legacy display-name-only rows remain supported | Top-level `model` key in `settings.json`, normalized to its matching slug |

- Only claude lacks an enumeration command, so PTY is required (takes a few seconds to boot per profile). The list may differ depending on the plan/account, so it is queried **per profile** (injecting `CLAUDE_CONFIG_DIR`).
- The PTY child runs in the fixed `~/.config/s7s/probe` folder, not the directory s7s was started from, so a folder-scoped startup dialog cannot fail the query ([usage-display.md](./usage-display.md)). The startup version gate used to make such a failure sticky: with the CLI version unchanged, the stale catalog stayed cached until the next upgrade.
- The `Default (recommended)` row in the claude `/model` screen duplicates s7s's own Default (no injection) item in the dropdown, so it is excluded from the list. If `✔` is on this row, the default model is set to None (CLI Default).
- Claude 2.1.283 and 2.1.284 show ten rows at a time, followed by `… +N model`. The PTY probe moves down through the picker without confirming a selection and combines the screens, so rows below the fold are included.
- codex is also queried per profile (injecting `CODEX_HOME`), but it's a fast subprocess. The catalog is confirmed to be output even in an empty CODEX_HOME (bundled catalog).
- agy cannot inject config env (see "agy env injection verification" below), so it is queried **globally once for the default path profile**, and additional agy profiles share that result (`ModelCatalog::for_profile` fallback).

### agy row parsing (`models.rs::parse_agy_models`)

- agy 1.1.11 emits `slug<TAB>display name`, for example
  `gemini-3.6-flash-high<TAB>Gemini 3.6 Flash (High)`.
- The slug is the `--model` value and primary dropdown label; the display name is the dimmed note.
- Older display-name-only rows remain valid for compatibility with earlier CLI versions.
- A legacy display-name value in `settings.json` is mapped to the slug whose display name matches.
  Truly unknown configured values remain unchanged and therefore use the existing missing-model guard.
- A tab must never survive in `ModelEntry.value`, `label`, or `note`. `unicode-width` assigns a tab
  zero width while terminals advance it to the next tab stop; measuring the row as zero-width and
  then emitting the raw tab corrupts every later cell in that row, including the popup border.

### claude row name → `--model` value (`models.rs::claude_model_value`)

The `/model` row name is **not** always the alias. Through 2.1.207 it was (`Opus` → `opus`).
2.1.220 renamed the long-context row to `Opus (1M context)`, whose alias is `opus[1m]`.
2.1.283 shows versioned names such as `Opus 5.5`, `Fable 5.1`, and `Sonnet 5`. These must map to
pinned IDs (`claude-opus-5-5`, `claude-fable-5-1`, `claude-sonnet-5`); a family alias could select
a different version after a CLI upgrade. 2.1.284 added `Sonnet 5.5` (`claude-sonnet-5-5`).
The `Haiku 4.5` row maps to the dated ID `claude-haiku-4-5-20251001`. Check the ID map against
[Claude Code model configuration](https://code.claude.com/docs/en/model-config) and
[Anthropic model IDs](https://platform.claude.com/docs/en/models/overview).

- Known versioned rows map to documented IDs. Bare aliases and the legacy `Opus (1M context)`
  row remain supported for older Claude versions.
- An unknown version or qualifier (for example `Opus 5.6` or `Opus (Preview)`) makes the whole
  query `Unavailable`, so a partial model list is never cached. With no cached Claude entry the
  dialog then shows only the four built-in aliases, so users lose version selection until the
  new row is mapped and released. A stale row labeled `Custom model`
  is the exception: it is not an offered model and is omitted from the list.
- The picker shows ten rows and `… +N model` for hidden rows. The probe collects screens until
  it has seen the reported number of distinct rows. If capture stops early, parsing fails.
- Rows collapsing to an already-seen value are folded into the first. A stale `Custom model` row
  can duplicate a canonical row; the checked (✔) state is retained.
- **Re-verify on every claude upgrade**: a new versioned row requires a documented ID mapping.
  `--model-probe` reports `Unavailable` for unknown or missing rows; update the map and fixture
  before release. Existing cache remains available after a failed query, so the probe result is
  the required drift check.

## CLI Does Not Validate Model Names (Observed)

- agy: If an invalid model name is provided, it **quietly falls back to the default model without errors**.
- codex: Displays the invalid slug as is without validation upon booting (fails on the first message).
- Therefore, **list accuracy is the responsibility of s7s**: Only dynamically enumerated results are placed in the dropdown, existing caches are not overwritten upon query failure (prohibiting saving empty lists), and if the configured default model is not in the list, OK is disabled (UI rules below).

## Cache and Update Timing

- Cache: `~/.config/s7s/models.json` (profile id key, `ModelCatalog`). The CLI version at the time of query (first line of `--version`) is saved along with the items.
- **Schema version (`MODELS_FILE_VERSION`)**: a file whose `version` differs is discarded wholesale on load. Bump it whenever cached values can be *wrong* rather than merely stale — the version gate below keys off the CLI version, so a parser fix alone would never evict a bad entry (nor the `default_model` / `last_selected` derived from it). v2 = claude aliases normalized; v3 = agy tab-separated rows split into slug/display fields; v4 = Claude versioned rows and full-list capture, evicting v3 lists that contained only one accepted row.
- **App Startup**: Initiates background querying but with a **version gate** — if the cached CLI version and current version match, re-querying is skipped (`ModelsResult::Skipped`) to eliminate the cost of booting the claude PTY. The model list only changes upon CLI upgrade/plan change.
- **ctrl+u**: Force re-query (ignores version gate) — covers plan changes.
- **Profile Save**: Only saved profiles are incrementally force-queried (path might have changed).
- **Profile Delete**: Removes the cached item.
- Unlike usage querying, this proceeds **quietly**: No Loading indicator or completion message. Even if the usage `Loading...` disappears, the model query may still continue.
- Not logged in / folder missing / CLI not installed are skipped as `Unavailable` (cache maintained).

## New Session Dialog UI Rules

- Control order (tab/↑↓): Profile → **Model** → Folder → OK/Cancel. Modal height 14 lines.
- Dropdown 0 is always **Default** (no `--model` injection — uses the CLI's own default model).
- **Initial selection priority: last launched pick → CLI-configured default → missing placeholder.**
  - **Last pick (`last_selected`)**: the model the user last launched a new session with for this profile (see "Last-selected memory" below). Wins when it is still present in the fetched list. A remembered **Default** pick selects Default (index 0). A remembered model that is no longer in the list (e.g. removed after a CLI upgrade) is skipped and falls through to the CLI default — it never produces a placeholder.
  - **CLI default (`default_model`)**: used when there is no usable last pick. If it is not in the list, a **missing placeholder item** (red, "not in the fetched model list") is selected and **OK is disabled** — the user must choose another item (including Default) to execute (to prevent quietly executing typos/stale configurations).
- Upon confirming a profile change, the model items are reconfigured based on that agent.
- Background query completion **does not immediately replace the list in an open dialog** (to prevent cursor jumping) — it is reflected the next time it opens.
- If there is no cache at all, built-in fallbacks are used: claude has 4 aliases (fable/opus/sonnet/haiku). codex/agy enumerate quickly, so only Default is shown without a fallback (filled after the first query).
- Model selection for resume (continue) is not implemented (decided 2026-07-14: separately later).

### Last-selected memory

- On launching a new session, the chosen model is stored per profile as `ProfileModels.last_selected` in `models.json` (`LastSelection::Default` for the Default entry, `LastSelection::Model(value)` for a specific model). This is what drives the "last pick" tier of the initial-selection priority above.
- `None` = never recorded (fall back to the CLI default). An explicit **Default** pick is remembered distinctly from "never picked", so choosing Default sticks.
- Stored on the same `models.json` entry as the fetched list. Since background re-fetches build a fresh `ProfileModels` (with `last_selected == None`), `ModelCatalog::insert` **carries over** the previously stored pick so refreshes never wipe it.
- Written via `ModelCatalog::set_last_selected` + `save()` at launch time; a no-op if the profile has no cached entry yet (rare first-run window before any fetch completes). `save()` is test-guarded so unit tests never touch the real cache.
- Motivation: when a CLI removes or renames models across versions, the CLI's own `default_model` in its config can go stale and no longer match the fetched list. Matching agy display-name defaults are normalized to current slugs; for genuinely stale defaults, once the user picks a valid model, `last_selected` becomes the dialog default and the stale config no longer resurfaces.

## Command Injection (Append Method)

- `NewSessionRequest.model` (Option) → `resume::run_new`/`preview_new_command` appends ` --model '<value>'` to the tail of the template. If Default (None), it leaves it as is.
- Templates (`new_*` in `config.toml`) are not touched, ensuring compatibility with existing user settings.
- The value is always wrapped in single quotes (legacy agy display-name values contain spaces/parentheses).
- The `--model` long flag behavior for all three CLIs was empirically verified via the boot banner/status bar: claude alias/full name (`claude-haiku-4-5-20251001`), codex slug, and agy slug (legacy versions used display names).

## Blocking Antigravity in Add Profile

- For agy, additional profiles are meaningless (cannot inject env → skips usage, resumes with default account), so **during new addition or switching from another agent, the Antigravity radio button is dimmed and unselectable** (`ProfileFormState::agy_allowed`, including defensive validation during the save phase). Editing/deleting existing Antigravity profiles is maintained. The builtin agy profile is always present as a seed, so there is no loss of accessibility.

### agy Env Injection Verification (2026-07-14, agy 1.1.2)

- The exhaustive list of `ANTIGRAVITY_*` environment variables from `strings $(which agy)` **does not include** `ANTIGRAVITY_CONFIG_DIR` (the code path for that variable from third-party docs does not exist in this CLI).
- Booting with an empty folder specified as `ANTIGRAVITY_CONFIG_DIR` still boots with the existing account and leaves the folder empty — **confirmed completely ignored**.
- `HOME` override works (creates a new `.gemini` tree + login flow), but it is not adopted because agent workspace distortion and keychain account conflicts have not been verified.
- Upon agy upgrade, re-verify the creation of dedicated variables with `strings $(which agy) | grep -o 'ANTIGRAVITY_[A-Z_]*'`, and unblock if one appears.

## Verification Method

If model parsing code is changed or the agent CLI is upgraded:

```bash
# Force query the model list of all profiles without TUI (does not update cache)
cargo build --release && ./target/release/s7s --model-probe

# Text dump of the claude /model screen (for parser debugging, claude-model.screen.txt)
mkdir -p /tmp/dump && ULAR_USAGE_DUMP=/tmp/dump ./target/release/s7s --model-probe
```

- For claude, open `/model` in actual `claude` and compare with the list and ✔ position.
- Treat `claude: failed to parse /model screen` as a failed verification. Do not accept a probe
  that omits a row from the real picker, even if the TUI still shows cached models.
- For codex, compare with `codex debug models` output (visibility=list), and for agy, with `agy models` output.
- Since CLIs do not filter out invalid model names, the final verification is to actually launch a session once with the value from the probe result and check the active model notation in the banner/status bar.
- The claude screen fixtures in unit tests (`src/models.rs::tests`) cover 2.1.220, 2.1.283, and 2.1.284 captures. If the screen format changes, update the fixtures as well.

## New Session with Context and Model Selection

The new session with context attached ([Details](session-context.md)) reuses the existing New Session dialog exactly, so the behavior of the Model dropdown (list source, default selection, missing handling) is also identically maintained without changes. The `--model` flag of the selected model is injected before the bootstrap prompt (`<template> --model '<value>' '<prompt>'`).
