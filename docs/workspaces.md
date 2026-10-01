# Workspaces

> Status: Current
> Read when: Changing the Workspaces screen, workspace storage or matching, the
> open-workspace scope, or the `ctrl+w` palette rows.
> Entry points: `src/workspaces.rs`, `src/ui/workspace/`, `src/ui/reload.rs`, `ui::App::rebuild_filtered`,
> `ui::quick::registry::build_workspace_items`

A workspace is an s7s-owned, named scope over the session index: include words,
exclude words, and a set of folders. It never touches agent storage. Unrelated to
`session_workspace.rs` (captured cwds of s7s-created sessions) and the scratch
workspace (`scratch.rs`).

## Screen order

`Profile ⇄ Workspaces ⇄ Session ⇄ Detail`.

- Profile `→` opens Workspaces with the list focused.
- Session list `←` (Table focus) opens Workspaces with its Sessions pane focused.
- Workspaces panes are `List | Detail | Sessions`, moved between with `←`/`→`
  (`h`/`l`). `←` on List goes to Profile; `→` on Sessions goes to the Session
  screen with the same scope and selected row.
- The Detail pane is skipped (not focusable) while "All" is open.

## The open workspace

- `WorkspaceStore::active` (workspace id, `None` = "All") is the single scope
  state. The List cursor *is* `active`: moving it changes what the Session
  screen lists. Palette rows set the same field. There is no separate cursor.
- `App::rebuild_filtered` applies the ordinary filter (keyword, agent, folder,
  profile, bookmark) and then `Workspace::matches` (AND). Bookmark grouping and
  activity order are unchanged. Clearing filters (`0`, Esc) does not close the
  workspace.
- Changing the scope resets the session cursor to the top and the Detail cursor
  to Name.
- The Session table title shows the scope: `Session[<workspace>, <filters>: N]`.
- A context-source jump (`ctrl+o`) whose target is outside the open workspace
  closes it after clearing filters; the Back action restores the origin's
  workspace together with its filter (`JumpOrigin::workspace`).

## Matching (`Workspace::matches`)

| Attribute | Rule |
| --- | --- |
| folders | Empty = no restriction. Otherwise the session's absolute `cwd` must equal one of them. Full paths, not the basename-keyed folder filter, so `~/a/api` and `~/b/api` differ |
| includes | Whitespace-separated, NFC-lowercased words; every word must match |
| excludes | Same tokenization; any matching word hides the session |

Words match through `filter::token_matches`, the same text as `/` search
(user turns, title, folder, last assistant answers, and long session-id tokens).

## Editing

- `+` (any pane) appends `New Workspace` (`New Workspace 2`, … when taken),
  opens it, and starts editing its name in the list with the whole value
  selected. Enter saves; Esc removes the unsaved workspace and reopens the
  previous scope.
- Enter on a List row renames it in place. Enter on the Detail Name/Includes/
  Excludes rows edits that row in place (`UiMode::WorkspaceEdit`).
- Include/exclude edits apply on every keystroke so the session list follows;
  Esc restores the value before the edit; Enter saves. Names change only on
  Enter.
- Names are trimmed, non-empty, unique case-insensitively, and may not be
  `All`: palette rows are labelled `Open Workspace <name>`, so a name must
  identify one row. A rejected name keeps the edit open with a status message.
- Detail folder rows list the workspace's selected folders first (selection
  order), then every other session cwd by latest activity. The order is
  captured when the scope changes and kept while toggling, so `space` never
  moves the cursor. A stored folder with no remaining session still appears so
  it can be unchecked. `[✓]` = selected.
- `ctrl+d`/`del` on List or Detail asks for confirmation (Cancel focused) and
  removes only the workspace; the cursor stays on the same row. "All" can be
  neither edited nor deleted.
- The Sessions pane forwards keys to the Session table handler (`on_key_table`)
  with `Focus::Table`, so `enter`, `ctrl+d` (delete *session*), `ctrl+r`,
  `ctrl+b`, `a`, `f`, `c`, `0`, `1..5` behave as on the Session screen.
- `/` opens the shared keyword search from any pane; Enter/Tab moves focus to
  the Sessions pane, Esc returns to the pane it was opened from.

## Palette (`ctrl+w`)

- `ctrl+w` on Session, Workspaces, Detail, and Profile opens the Quick Command
  palette with `open workspace ` typed. The query stays editable.
- `build_workspace_items` adds rows only for a non-empty query, so the plain
  `:` palette is unchanged: `Close Workspace` first, then `Open Workspace
  <name>` in list order, then the matching registry commands (for this query,
  `Open Workspace Window`). `Close Workspace` carries the `open` alias so the
  prefilled query lists it. While "All" is open it is disabled and sorts after
  the open rows.
- Running a workspace row opens that scope and switches to the **Session**
  screen, not Workspaces. Workspace rows are not recorded in palette history.

## Storage

`~/.config/s7s/workspaces.json`, versioned JSON (`STORE_VERSION`), mode `0600`:
`{ version, workspaces: [{ id, name, includes, excludes, folders }], active }`.

- Ids are stable; `active` survives renames. An `active` id that no longer
  exists loads as "All".
- Every save is a `WorkspaceChange` (`Upsert` / `Remove` / `Opened`) applied by
  id onto a freshly read file under `store_lock::with_store_lock`, then an
  atomic replace (temp file + rename). So a committed edit, folder toggle,
  add/delete, or scope change never drops workspaces another running s7s
  saved. No fsync: `Opened` is written on each List cursor move.
- An `Upsert` whose name another workspace already holds *in the file* is
  refused like a local duplicate; the name edit stays open.
- `Upsert` of an id another instance deleted re-adds it (the edit being saved
  wins).

### Multiple running instances

- The in-memory list changes only by this instance's own edits, at startup,
  and on `ctrl+u` (`App::reload_shared_stores`, start of a new refresh cycle).
  There is no file watching or polling.
- On reload, the open workspace stays this instance's own: the file's
  `active` (another instance's last scope) is used only at startup. If the open
  workspace was deleted elsewhere, "All" opens.
- An unreadable or newer-version store is never overwritten: startup keeps an
  empty in-memory store and reports it; each save re-reads the file and fails
  with a status message; `ctrl+u` shows a `Reload Failed` dialog and keeps the
  in-memory copy. Unit tests run with saving disabled
  (`App::workspaces_path = None`) unless a test sets a temporary path.

## Layout

See [ui-style-guide.md](./ui-style-guide.md) §Workspaces screen.

## Verification

- `ui::workspace::tests` (navigation, scope, add/rename/cancel, live include/
  exclude, folder toggles, delete, Sessions-pane delegation, search focus,
  palette open/close, context jump, persistence, render), `workspaces::tests`
  (matching, per-change commits, refused names, unreadable store),
  `ui::reload::tests` (`ctrl+u` reload), and `store_lock::tests`.
- Release PTY check (`s7s demo`): add a workspace, toggle folders, type
  includes and watch the list, open/close via `ctrl+w`, restart and confirm the
  scope is reopened, and check a narrow (80-column) terminal.
- Two release instances on `s7s demo`: add a workspace in each without
  reloading, confirm `workspaces.json` holds both, then `ctrl+u` in each.
