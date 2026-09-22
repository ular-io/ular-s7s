# Session folder

A session's folder is where s7s opens it. The list shows it, the folder filter
groups by it, and resume runs `cd <folder> && <agent CLI>` in it.

The folder normally comes from the agent's own storage: the first `cwd` recorded
in a Claude transcript, `session_meta.cwd` in a Codex rollout, the workspace
recorded in the Antigravity conversation DB. `Change Folder` overrides that value
per session, and `src/scan.rs::apply_workspace_cwd` decides which one wins.

## Change Folder

Palette-only command (`:` → `Change Folder`), implemented in
`src/ui/overlays/change_folder.rs`. It is deliberately narrow:

- It changes **only where the session opens next time**.
- It moves no file and never rewrites the agent's transcript, so every absolute
  path in the stored conversation keeps working.
- The chosen folder must already exist. Creating a project folder belongs to New
  Session.
- The value is written to `~/.config/s7s/session_workspaces.json` by
  `AppEffect::ChangeSessionFolder`, and dropping that record restores the folder
  the agent recorded. `session_delete` clears the record for every agent.

Changed sessions are not marked in the list — that was an explicit decision, so
the only way back is to set the original folder again. The original is never
lost: it stays in the agent's own storage.

## Why Antigravity is refused

`Change Folder` returns a message instead of opening for Antigravity sessions
(`quick::input::change_folder_candidate` dims the palette row;
`open_change_folder_at` refuses as well, so every entry point gives the reason).

The three CLIs do not agree on what a resumed session's folder is. Verified
against claude 2.1.278, codex-cli 0.155.1 and agy 1.2.8 by starting a disposable
session in one folder and resuming it from another:

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
