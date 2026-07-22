# Model Selection (New Session Model Dropdown)

Design for querying, caching, and injecting the "selectable models list" to be displayed in the Model dropdown of the New Session dialog. Implementation: `src/models.rs` (query/cache), `src/ui/mod.rs` (dropdown state/background integration), `src/resume.rs::with_model_flag` (command injection).

## Model List Enumeration Methods (Observed July 2026)

| Agent | Method | Value Format | Default Model Source |
| :-- | :-- | :-- | :-- |
| claude | Scraping `/model` screen via PTY (`probe::pty::drive_screen`, shared with usage) | alias lowercase (`fable`) | `✔` mark in the screen list |
| codex | `codex debug models` JSON (only `visibility=="list"`) | slug (`gpt-5.6-sol`) | Top-level `model` key in `<CODEX_HOME>/config.toml` |
| agy | Line-by-line output of `agy models` | Display name exactly as is (`Gemini 3.1 Pro (Low)`) | Top-level `model` key in `settings.json` |

- Only claude lacks an enumeration command, so PTY is required (takes a few seconds to boot per profile). The list may differ depending on the plan/account, so it is queried **per profile** (injecting `CLAUDE_CONFIG_DIR`).
- The `Default (recommended)` row in the claude `/model` screen duplicates s7s's own Default (no injection) item in the dropdown, so it is excluded from the list. If `✔` is on this row, the default model is set to None (CLI Default).
- codex is also queried per profile (injecting `CODEX_HOME`), but it's a fast subprocess. The catalog is confirmed to be output even in an empty CODEX_HOME (bundled catalog).
- agy cannot inject config env (see "agy env injection verification" below), so it is queried **globally once for the default path profile**, and additional agy profiles share that result (`ModelCatalog::for_profile` fallback).

## CLI Does Not Validate Model Names (Observed)

- agy: If an invalid model name is provided, it **quietly falls back to the default model without errors**.
- codex: Displays the invalid slug as is without validation upon booting (fails on the first message).
- Therefore, **list accuracy is the responsibility of s7s**: Only dynamically enumerated results are placed in the dropdown, existing caches are not overwritten upon query failure (prohibiting saving empty lists), and if the configured default model is not in the list, OK is disabled (UI rules below).

## Cache and Update Timing

- Cache: `~/.config/s7s/models.json` (profile id key, `ModelCatalog`). The CLI version at the time of query (first line of `--version`) is saved along with the items.
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
- Motivation: when a CLI renames models across versions (e.g. agy display-name → slug), the CLI's own `default_model` in its config can go stale and no longer match the fetched list. Once the user picks a valid model once, `last_selected` becomes the dialog default and the stale config no longer resurfaces.

## Command Injection (Append Method)

- `NewSessionRequest.model` (Option) → `resume::run_new`/`preview_new_command` appends ` --model '<value>'` to the tail of the template. If Default (None), it leaves it as is.
- Templates (`new_*` in `config.toml`) are not touched, ensuring compatibility with existing user settings.
- The value is always wrapped in single quotes (in preparation for spaces/parentheses in agy display names).
- The `--model` long flag behavior for all three CLIs was empirically verified via the boot banner/status bar: claude alias/full name (`claude-haiku-4-5-20251001`), codex slug, agy display name.

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
- For codex, compare with `codex debug models` output (visibility=list), and for agy, with `agy models` output.
- Since CLIs do not filter out invalid model names, the final verification is to actually launch a session once with the value from the probe result and check the active model notation in the banner/status bar.
- The claude screen fixture in unit tests (`src/models.rs::tests`) uses actual captures (2026-07-14, 2.1.207). If the screen format changes, update the fixture as well.

## New Session with Context and Model Selection

The new session with context attached ([Details](session-context.md)) reuses the existing New Session dialog exactly, so the behavior of the Model dropdown (list source, default selection, missing handling) is also identically maintained without changes. The `--model` flag of the selected model is injected before the bootstrap prompt (`<template> --model '<value>' '<prompt>'`).
