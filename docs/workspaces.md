# Workspaces

> Status: Current
> Read when: Changing the Session screen's workspace pane, the Workspaces
> screen, workspace storage or matching, the open-workspace scope, or the
> `ctrl+w` palette rows.
> Entry points: `src/workspaces.rs`, `src/ui/workspace/`, `src/ui/reload.rs`, `ui::App::rebuild_filtered`,
> `ui::quick::registry::build_workspace_items`

A workspace is an s7s-owned, named scope over the session index: include words,
exclude words, and a set of folders. It never touches agent storage. Unrelated to
`session_workspace.rs` (captured cwds of s7s-created sessions) and the scratch
workspace (`scratch.rs`).

## Screen order

`Profile ⇄ workspace pane ⇄ Session list ⇄ Prompt → Detail`.

- The workspace pane is part of the Session screen (`Focus::Workspaces`), drawn
  left of the session table only while it has focus. There is no separate
  open/closed state.
- Session list `←` (Table focus) shows the pane, focused
  (`App::open_workspace_pane`). In the pane, `→`/`l`/Esc close it and focus the
  table; `←`/`h` close it and go to Profile.
- Profile `→` goes back to the Session screen with the pane focused on the open
  workspace, so `←`/`→` retrace the same path. `switch_screen` to any other
  screen closes the pane, so a palette `Open Session Window` lands on the table.
- Enter on a workspace row opens the **Workspaces screen** for it
  (`Screen::Workspace`), Detail cursor on Name. Only its Detail pane takes keys;
  the list and session table beside it are display-only. Esc returns to the
  Session screen with the pane focused on the same workspace; `←`/`→` do not
  leave (the Search row and an edit use them as text cursor keys). Enter on
  "All" does nothing, so the Workspaces screen never shows "All": a scope change
  to "All" while it is shown (cancelled new workspace, `ctrl+u` after another
  instance deleted it) returns to the pane (`App::workspace_scope_changed`).
- Palette `Open Workspace Window` shows the pane from any screen.

## The open workspace

- `WorkspaceStore::active` (workspace id, `None` = "All") is the single scope
  state. The pane cursor *is* `active`: moving it changes what the session
  list shows. Palette rows set the same field. There is no separate cursor.
- The pane's last row is the fixed `[NEW WORKSPACE]`, outside the stored list.
  On it `active` is `None` ("All"); `WorkspaceScreenState::new_row` records
  that the cursor is there and counts only while `active` is `None`
  (`App::workspace_pane_cursor`). Opening the pane starts on the open
  workspace. `+` on any row moves the cursor to `[NEW WORKSPACE]` and adds a
  workspace as Enter there does (below), so an Esc on the new name returns to
  that row.
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

- Enter on `[NEW WORKSPACE]` appends `New Workspace` (`New Workspace 2`, …
  when taken), opens it, and shows it on the Workspaces screen with its Detail
  Name row in edit and the whole value selected (`App::add_workspace`). Enter
  saves it (`Upsert` + `Opened`) and stays on the screen to pick folders; Esc
  removes the unsaved workspace and returns to the pane on `[NEW WORKSPACE]`
  ("All").
- The pane has no rename: names change only on the Detail Name row. Enter on
  the Detail Name/Includes/Excludes rows edits that row in place
  (`UiMode::WorkspaceEdit`); Esc ends the edit before it can leave the screen.
- Include/exclude edits apply on every keystroke so the session list follows;
  Esc restores the value before the edit; Enter saves. Names change only on
  Enter.
- Names are trimmed, non-empty, unique case-insensitively, and may not be
  `All`: palette rows are labelled `Open Workspace <name>`, so a name must
  identify one row. A rejected name keeps the edit open with a status message.
- Detail folder rows list the workspace's selected folders first, then every
  other session cwd; each group is ordered by latest session activity (newest
  first, ties by path), and a selected folder with no remaining session sorts
  last in its group. The stored selection order is not used. The order is
  captured when the scope changes and kept while toggling, so `space` never
  moves the cursor. A stored folder with no remaining session still appears so
  it can be unchecked. `[✓]` = selected. Each row ends with a dim ` (N)`: the
  folder's sessions across every session, not narrowed by the session filters
  or the workspace's include/exclude words, so a stored folder with no session
  left reads `(0)`. Captured with the order (`WorkspaceScreenState::folder_counts`).
- A `Search` row sits between the `Folders` heading and the folder rows. It is
  an ordinary cursor row (`SEARCH_ROW`), reached and left only with `↑`/`↓`;
  while the cursor is on it, keys edit the query directly with no Enter and no
  edit mode (`App::workspace_search_focused`). Typed characters include the
  pane's letter shortcuts (`j`, `k`, `g`, `q`, …) and `space`; `←`/`→`/`Home`/
  `End` move the text cursor; Backspace/`Delete` delete at the cursor; Esc
  clears a non-empty query (an empty one leaves the screen); Enter moves to the
  first match; paste inserts. ctrl combinations keep their pane meaning.
- Arriving on the row with a non-empty query selects the whole query
  (`TextInput::select_all`): typing or paste replaces it, Backspace/`Delete`
  clear it, and `←`/`→` drop the selection to the start/end, keeping the text.
  Leaving the row drops the selection.
- Every whitespace-separated query word must occur, case-insensitively, in the
  folder's full path or its displayed label. Non-matching rows are hidden, not
  reordered; their selection is kept and still counted in `· N selected`. The
  query is not stored and is cleared when the scope changes.
- `ctrl+d`/`del` in the pane asks for confirmation (Cancel focused) and removes
  only the workspace; the cursor stays on the same row. It is the only place a
  workspace is deleted: the Workspaces screen neither deletes nor adds. "All"
  and `[NEW WORKSPACE]` cannot be deleted. On the Session screen `ctrl+d`
  deletes the workspace only while the pane has focus; on the table it deletes
  the session.
- `/` opens the shared keyword search from the pane and the Workspaces screen.
  From the pane, Esc returns to it; Enter/Tab focus the session list, closing
  it. The Workspaces screen keeps its Detail pane either way.

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
  screen with the table focused (the pane closed), not Workspaces. Workspace
  rows are not recorded in palette history.
- `Open Workspace Window` shows the workspace pane; it is disabled only while
  the pane already has focus.

## Storage

`~/.config/s7s/workspaces.json`, versioned JSON (`STORE_VERSION`), mode `0600`:
`{ version, workspaces: [{ id, name, includes, excludes, folders }], active }`.

- Ids are stable; `active` survives renames. An `active` id that no longer
  exists loads as "All".
- The TUI does not restore `active`: every start opens "All"
  (`WorkspaceStore::load_at_startup`). The field is still written (`Opened`)
  and read by `load`, so the format is unchanged.
- Every save is a `WorkspaceChange` (`Upsert` / `Remove` / `Opened`) applied by
  id onto a freshly read file under `store_lock::with_store_lock`, then an
  atomic replace (temp file + rename). So a committed edit, folder toggle,
  add/delete, or scope change never drops workspaces another running s7s
  saved. No fsync: `Opened` is written on each pane cursor move.
- An `Upsert` whose name another workspace already holds *in the file* is
  refused like a local duplicate; the name edit stays open.
- `Upsert` of an id another instance deleted re-adds it (the edit being saved
  wins).

### Multiple running instances

- The in-memory list changes only by this instance's own edits, at startup,
  and on `ctrl+u` (`App::reload_shared_stores`, start of a new refresh cycle).
  There is no file watching or polling.
- On reload, the open workspace stays this instance's own: the file's
  `active` (another instance's last scope) is never applied. If the open
  workspace was deleted elsewhere, "All" opens.
- An unreadable or newer-version store is never overwritten: startup keeps an
  empty in-memory store and reports it; each save re-reads the file and fails
  with a status message; `ctrl+u` shows a `Reload Failed` dialog and keeps the
  in-memory copy. Unit tests run with saving disabled
  (`App::workspaces_path = None`) unless a test sets a temporary path.

## Layout

See [ui-style-guide.md](./ui-style-guide.md) §Workspace pane and §Workspaces
screen.

## Verification

- `ui::workspace::tests` (pane open/close and Profile moves, scope and the
  `[NEW WORKSPACE]` row, Enter/Esc between pane and Workspaces screen,
  add/rename/cancel, live include/exclude, folder toggles, folder search,
  delete, search focus, palette open/close, context jump, persistence, render),
  `ui::render::tests::workspace_pane_shrinks_the_body_and_hides_a_narrow_prompt`,
  `workspaces::tests` (matching, per-change commits, refused names, unreadable
  store),
  `ui::reload::tests` (`ctrl+u` reload), and `store_lock::tests`.
- Release PTY check (`s7s demo`): `←` from the session list and move through
  workspaces, add one with `+` and from `[NEW WORKSPACE]`, toggle folders,
  filter folders on the Search row and toggle a match, type includes and watch
  the list, Esc back to the pane, delete a workspace, open/close via `ctrl+w`,
  restart with a workspace open and confirm "All" opens, and check an
  80-column terminal (Prompt hidden while
  the pane is open) and a 120-column one (Prompt kept).
- Two release instances on `s7s demo`: add a workspace in each without
  reloading, confirm `workspaces.json` holds both, then `ctrl+u` in each.
