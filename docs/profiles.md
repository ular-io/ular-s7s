# Profiles (Multiple Subscriptions)

Profile = a bundle of **Agent type + Name + config folder (path) + OAuth token (storage only)**.
Even for the same agent, if the config folder is different, it is a separate profile (e.g., Claude personal subscription + team subscription). Implementation is in `src/profile.rs` (model/storage) and `src/ui/profile/` (profile screen/form: `state`, `input`, `render`). Usage/model fetch coordination and cross-feature helpers (`set_single_profile`, `profile_name`, `session_profile_root`) remain in `src/ui/mod.rs`.

## Storage Location

- `~/.config/s7s/profiles.json` (`config_base_dir()`, `src/config.rs` — hardcoded, all platforms)
- This is a file owned and saved by the app (the configuration `config.toml` is manually edited by the user and is separate from profiles).
- Since the OAuth token can be included in plain text, it is given **0600 permissions** when saved.
- It seeds the default 3 (builtin) on first run. Builtin profiles cannot be deleted or have their agents changed, and even if they are manually edited out, they are re-seeded upon load.

## Meaning of the Path

Profile path = Agent **config root**. The session directory is obtained by derivation rules.

| Agent | Default Root | Session Directory |
| :-- | :-- | :-- |
| Claude | `~/.claude` | `<path>/projects` |
| Codex | `~/.codex` | `<path>/sessions` |
| Antigravity | `~/.gemini/antigravity-cli` | `<path>` itself |

The path of the default profile absorbs the directory overrides from `config.toml` and is seeded (if the basename of the session directory matches the derivation rule, the parent is used as the root).

## Env Injection Rules (Core Precautions)

During usage query, resume/new session execution, and Claude rename CLI attempts (`claude --resume <id> --name ...`), the profile path is injected as an environment variable:

| Agent | Environment Variable | Notes |
| :-- | :-- | :-- |
| Claude | `CLAUDE_CONFIG_DIR` | |
| Codex | `CODEX_HOME` | |
| Antigravity | None | Additional profiles skip usage, and resume executes with the default account |

It has been empirically confirmed that Antigravity does not have a dedicated variable (2026-07-14, agy 1.1.2 — exhaustive strings check on the binary + boot experiment specifying `ANTIGRAVITY_CONFIG_DIR`, "agy env injection verification" section in [Model Selection](models.md)). Accordingly, **in the Add/Edit Profile form, Antigravity is dim + unselectable** (only editing existing Antigravity profiles is allowed, `ProfileFormState::agy_allowed`) — you can create one, but the creation of an additional profile with no functionality is fundamentally blocked.

**It is not injected for the default path profile** (`Profile::env_var()` returns None).
Reasons confirmed empirically:

- The core state file `.claude.json` of Claude Code (onboarding/account/folder trust) is located at **`~/.claude.json` (home root)** when the env is not set, and at **`<dir>/.claude.json`** when `CLAUDE_CONFIG_DIR` is set.
- Therefore, `CLAUDE_CONFIG_DIR=~/.claude claude` is **not the same** as `claude` — if `~/.claude/.claude.json` does not exist, it is recognized as a new installation and starts over from theme selection/login.
- Keychain authentication items are also separated based on the config path.

### Contaminated Env Cleanup (Guaranteeing Transcript Save)

Claude Code sessions inject `CLAUDECODE=1`, `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_CHILD_SESSION=1`, etc. into child processes. If s7s is executed inside a claude session (`!`/Bash), it inherits these variables, and the claude launched by s7s (confirmed empirically in 2.1.204+) sees `CLAUDE_CODE_CHILD_SESSION=1`, considers itself a child session for automation, and **completely skips saving the transcript** — the session appears to have disappeared from both the s7s list and `/resume` (it actually just didn't save, unrecoverable).

To prevent this, `src/resume.rs::sanitize_agent_env()` removes these variables from the process env and injects `CLAUDE_CODE_FORCE_SESSION_PERSISTENCE=1` when spawning a resume/new session. Unlike profile envs (like `CLAUDE_CONFIG_DIR`), it is processed as process env rather than as a command string prefix — to ensure shell initialization does not reset these variables and to avoid cluttering the preview string.

## Additional Profile Initial Setup

Since the new config folder is empty of login/trust states, you must manually run it once to complete it:

```bash
CLAUDE_CONFIG_DIR=~/.claude-team claude
# → Theme selection → Login → Exit after approving folder trust
```

### When Saving a Non-Existent Config Folder (Automating Creation + Login)

If you save a non-existent path in the add/edit profile form, a confirmation modal (`Create Config Folder`, `UiMode::ProfileDirConfirm`) that automates the above initial setup appears:

- **Create** (Default focus): Creates the folder with `create_dir_all` → saves the profile → the main loop releases the TUI and executes the agent for login (`resume::run_login`). After the agent exits, it returns to the TUI, rescans sessions, and performs an incremental usage query for that profile.
- **Cancel/esc**: Returns to the form maintaining input (path can be modified).
- Login execution, unlike resume/new session, runs **in the current folder of s7s without cd**, appending only the env prefix to the **base command without flags** (`claude`/`codex`/`agy`) — since the usage query runs in the s7s execution folder, if you trust this folder during login, subsequent usage queries will also pass.
- For **Antigravity additional paths**, env injection is impossible, making login execution meaningless, so it only creates the folder + saves and displays an informational message (`profile::login_runnable()` check).

- **Trust is on a per-folder basis** and is saved in `projects.<path>.hasTrustDialogAccepted` of `<dir>/.claude.json`. It is not shared between profiles, so in a new profile, it asks once for each folder for the first time.
- The usage query spawns claude in the s7s execution folder, so **you must trust that folder** for the query to pass (fails with `untrusted folder (trust prompt)` if unapproved).

## Session / Filter / Usage Integration

- The New Session dialog is shared across the query/detail/profile screens and is opened with `ctrl+n` (the existing `enter` to open from the profile screen is replaced by `ctrl+n`). The dialog consists of 3 dropdown controls and an OK/Cancel button row at the bottom (OK left, Cancel right):
  **Profile Combobox** (text input disabled, usage displayed next to each name),
  **Model Combobox** (text input disabled — for items, initial selection, and OK disable rules, see [Model Selection](models.md)),
  **Folder Combobox** (text input allowed).
  Focus cycles through Profile → Model → Folder → OK → Cancel using `tab`/`shift+tab` (buttons are independent tab stops; movement between buttons is also possible with `←`/`→`).
  Focused buttons are displayed as bright blue boxes, unfocused buttons as bright gray boxes + gray dim text like the "left" label.
  - Default profile: The query/detail screen uses the selected session's profile; the profile screen uses the selected profile.
  - Initial folder value & focus: The query/detail screen starts with the selected session's folder filled and focus on Profile; the profile screen starts with the folder empty and focus on Folder, with the Folder dropdown open (first item highlighted).
  - `enter` is the default action of the focused control: Buttons perform their function (OK = start session, Cancel = cancel); if a dropdown is closed, it opens the list; if open, it commits the cursor item and closes (if the cursor is not on the list, it commits the input text as is). The global `enter` = OK (start session) shortcut has been removed — to start a session, focus on the OK button and press `enter`.
  - `→` (closed dropdown) also opens the list. Profile/Model (uneditable) opens with the cursor on the current selection, and Folder puts the cursor on the first item.
  - `↑`/`↓` rotates vertical focus between controls when the dropdown is closed (Profile ↔ Model ↔ Folder ↔ Buttons, wraps at both ends like `tab` — the button row is a single vertical stop and entering it focuses OK first); when open, it moves the list cursor. Open lists wrap around top-to-bottom — `↑` at the top moves to the bottom (without closing), and `↓` at the bottom moves to the top.
  - Dropdown Common (when open): `space` only selects and keeps the list open (folder immediately reflects in the input box); `esc` closes without selecting. `tab`/`shift+tab` commits the cursor item, then closes and moves focus (commit-then-move — closing without selecting is exclusive to `esc`).
  - Folder Combo: Typing automatically opens the dropdown (can also be opened with `enter`/`→`), displaying matching folders at the top (normal color) and non-matching folders at the bottom (soft dim color like the "left" label) (no hiding). `→` while open only moves the text cursor right (the select+close complete function is removed — press `space` to reflect the cursor folder in the input box, or `enter` to reflect + close).
  - Starting a session is done via OK button `enter` (if the folder is empty, a "Select a folder first" error is shown on the left of the button row), cancel via Cancel button/`esc`. After execution, it returns to the session screen and rescans to reflect the newly saved session.
- Scanning is performed per profile loop, assigning a `profile_id` to each session (cached values are not trusted and always reassigned on scan — prevents stale data when deleting/recreating profiles).
- The meta paths used by rename (`rename.rs`) and Antigravity meta cleanup on session deletion are derived from `session.profile_id → Profile.path` (Commit 46). If the affiliated profile is not found, it does not fallback to the default path — rename aborts after showing an error, and deletion skips agy meta cleanup (body file deletion proceeds).
- Profiles without numbers are also scanned (the spec is to display all in the session list). The header displays up to 5 profiles as `<1>`~`<5>`, filtering them using the same number keys in the session screen.
- Rows 1~3 in the profile table are fixed to the default `Claude` → `Codex` → `Antigravity` order. User-added profiles are displayed from row 4 in registration order.
- In the profile screen, pressing `1`~`5` inserts the selected profile at that number position. Subsequent numbers shift down by one, and if 5 are already assigned, the existing number 5 is unassigned. `space` toggles the number. If assigned, the profile loses its number, shifts following numbers up, and unassigned profiles are appended after the current last number. Attempting to add when all 5 are assigned displays an error dialog and leaves the state unchanged. Number order is managed by a separate `shortcut` field, while profile table rows are fixed to registration order regardless of number changes.
- Deleting a profile removes it from the list, filters, usage state, and session list, but keeps the actual folder.
- In the agent filter (`a`) and folder filter (`f`) dialogs, checking an item (`space`) immediately reflects the selection in `filter` and calls `recompute()`, updating the background session list in real-time without closing the dialog (`sync_agent_selection_to_filter` / `sync_folder_selection_to_filter`). Enter = confirm and close, Esc = close (selection already reflected).
- Usage query target: Profiles with an existing path (only the default path for Antigravity). Even during refresh, the previous successful value (`UsageEntry.last`) is maintained in gray.
- Upon saving profile add/edit, all sessions are rescanned (cheap with mtime cache), but usage is **incrementally queried only for the saved profile**. Details: "Operation Method" section in [Usage Display](usage-display.md).

## Separating Source/Target Profiles (New Session with Context)

In a new session with context attached ([Details](session-context.md)), the profile's role is split in two.

- **Target Profile**: The profile selected in the New Session dialog. Determines the account, model, and env injection (`CLAUDE_CONFIG_DIR`/`CODEX_HOME`) of the executing agent — exactly identical to a regular new session.
- **Source Profile**: The affiliated profile of the referenced past session. It is **not injected into the target agent as env**, but only moves within the generated `s7s session ... --profile <source ID>` command. A child s7s process executing that command inside the new session independently scans all profiles to interpret the correct source.
- If the source profile has been deleted, execution aborts at the time of OK (fallback to other profiles prohibited — same account safety principle as rename). Interpretation by `s7s session` also ends in error if the requested profile is missing.
