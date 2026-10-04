# Session folder

A session's folder is where s7s opens it. The list shows it, the folder filter
groups by it, and resume runs `cd <folder> && <agent CLI>` in it.

The folder normally comes from the agent's own storage: the first `cwd` recorded
in a Claude transcript, `session_meta.cwd` in a Codex rollout, the workspace
recorded in the Antigravity conversation DB. `Change Folder` overrides that value
per session, and `src/scan.rs::apply_workspace_cwd` decides which one wins.

## Change Folder

The TUI palette command (`:` → `Change Folder`) and
`s7s session change-folder` share validation and persistence in
`src/session_folder.rs`. The dialog is implemented in
`src/ui/overlays/change_folder.rs`; the CLI adapter is
`src/session_cli/change_folder.rs`. Both are deliberately narrow:

- It changes **only where the session opens next time**.
- It moves no file and never rewrites the agent's transcript, so every absolute
  path in the stored conversation keeps working.
- The chosen folder must already exist. Creating a project folder belongs to New
  Session.
- The pick list shows every session folder by latest session activity
  (`ui::cwds_by_latest`), then the scratch workspace if no session has run
  there; typed matches move to the top without changing that order. Each row
  ends with the folder's total session count as a dim ` (N)`.
- The value is written to `~/.config/s7s/session_workspaces.json` by the shared
  folder-change service. `session_delete` clears the record for every agent.
- A running session keeps its current directory. The saved override is consumed
  on the next s7s resume, not by direct resumes outside s7s.

Changed sessions are not marked in the list — that was an explicit decision, so
the only way back is to set the original folder again. The original is never
lost: it stays in the agent's own storage.

## CLI

```bash
s7s session change-folder <ID>... --to <DIR> [--agent AGENT] [--profile ID] [--dry-run]
```

- Full, explicit IDs only; no folder-name selector. Existing `session list`
  folder filters compare basenames, which cannot distinguish two projects with
  the same name. Duplicate IDs are ignored, keeping the first occurrence order.
- One quiet incremental scan resolves the entire batch. Every ID must match
  exactly one session under the optional agent/profile constraints. A missing
  profile, missing ID, ambiguous ID, or Antigravity target fails the complete
  batch before any folder mapping is written.
- `--to` must name an existing directory. Relative paths resolve against the
  command's current directory; `~/` is expanded. Unlike the TUI's project-name
  shorthand, a bare CLI name is a relative path, not an s7s-managed project.
  Symlinks resolve to the canonical absolute destination.
- `--dry-run` validates and prints the plan without writing the folder store or
  taking its writer lock. The ordinary incremental session scan may still update
  the disposable index cache, as it does for `session list`.
- Output includes each session's agent/profile, full ID, and before/after full
  paths. Successful changes print only after the stored mappings are re-read
  and verified. Exit codes: 0 on success (including dry-run), 1 for resolution,
  validation or storage failures, 2 for argument errors.

## Shared store writes

`session_workspace::record_many` holds `store_lock::with_store_lock` across
re-reading the file, merging every batch mapping, atomic replacement, and
read-back verification. Unrelated mappings and handoff cwd records are retained.
Single-session TUI writes and handoffs use the same operation; deletion's
mapping cleanup holds the same lock. Concurrent writers cannot drop each
other's changes by saving an old snapshot. A malformed or unsupported-version
store fails a write instead of being replaced by an empty store.

An already-open TUI picks up CLI changes on `Ctrl+U`; no full cache rebuild is
needed. To restore a session, explicitly set its original directory again.

## Why Antigravity is refused

`Change Folder` returns a message instead of opening for Antigravity sessions
(`quick::input::change_folder_candidate` dims the palette row;
`open_change_folder_at` refuses as well, so every entry point gives the reason).
The CLI and shared service enforce the same refusal, including mixed batches.

The three CLIs do not agree on what a resumed session's folder is. Claude
2.1.289 and codex-cli 0.160.0 were rechecked using disposable sessions, a CLI
folder-change batch, and the s7s resume launcher with non-interactive agent
templates: `pwd` followed the saved destination. The Antigravity behavior was
verified separately against agy 1.2.8:

| | resume by id from another folder | working directory after resume | does the session learn the folder changed |
| --- | --- | --- | --- |
| claude | works; records append to the original project directory | **the folder it was launched in** | yes — an `Environment update` system reminder names the new and the previous folder |
| codex | works | **the folder it was launched in** | yes — each turn carries a `<cwd>` block, so earlier turns keep the old value visible |
| agy | works | **the folder recorded when the conversation was created** | not applicable |

For Claude and Codex the launch directory wins, so an override is the session's
real folder. For Antigravity the header and the trust prompt follow the launch
directory but `pwd` still returns the original folder, so an override would only
make the list disagree with where the work happens.

The same table is why no "the folder changed" prompt is injected into the
session: Claude and Codex already announce it with the previous value, and
Antigravity carries no folder to compare against. Nothing is appended to a
transcript for this feature.

## Re-verify after a CLI upgrade

The table above is version-specific. Re-run the check in
[testing.md](./testing.md#session-folder-checks) when any agent CLI is upgraded —
especially before relaxing the Antigravity refusal.
