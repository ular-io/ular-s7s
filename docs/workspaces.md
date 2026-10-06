# Workspaces

> Status: Current
> Read when: Changing the Session screen's workspace pane, the workspace edit
> dialog, workspace storage, ordering, or matching, the open-workspace scope,
> or the `ctrl+w` palette rows.
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
- Enter on a workspace row opens the **edit dialog** for it over the Session
  screen (`UiMode::WorkspaceEdit`, `App::open_workspace_dialog`). Enter on
  `[NEW WORKSPACE]`, or `+` on any row, opens it for a new workspace
  (`App::open_new_workspace_dialog`). Enter on "All" or `[NONE-WORKSPACE]` does nothing.
  Closing the dialog returns to the pane. There is no separate workspace screen.
- Palette `Open Workspace Window` shows the pane from any screen.

## The open workspace

- `WorkspaceStore::active` (`WorkspaceScope::All`, `Unassigned`, or
  `Workspace(id)`) is the single scope state. The pane cursor *is* `active`: moving it changes what the session
  list shows. Palette rows set the same field. There is no separate cursor.
- The pane's last row is the fixed `[NEW WORKSPACE]`, outside the stored list.
  On it `active` is `All`; `WorkspaceScreenState::new_row` records
  that the cursor is there and counts only while `active` is `All`
  (`App::workspace_pane_cursor`). Opening the pane starts on the open
  workspace. `+` opens the new-workspace dialog without moving the cursor or
  changing the scope.
- Stored workspaces are listed by name as text (`WorkspaceStore::sort`:
  NFC-lowercased name, then the exact name, then the id), so digits come
  before Latin letters and Latin letters before Hangul. The in-memory list is
  kept in that order (`load`, `upsert`, and every `commit` sort), so pane rows,
  `active_index`, and the palette's `Open Workspace <name>` rows share it. A
  rename moves the workspace to its new place and the pane cursor follows it.
- `App::rebuild_filtered` applies the ordinary filter (keyword, agent, folder,
  profile, bookmark) and then `App::retain_workspace_scope` (AND). Saved scopes
  use `Workspace::matches`. Bookmark grouping and activity order are unchanged. Clearing filters (`0`, Esc) does not close the
  workspace.
- Changing the scope (or saving the open workspace) resets the session cursor
  to the top.
- The Session table title shows the scope: `Session[<workspace>, <filters>: N]`.
- A context-source jump (`ctrl+o`) whose target is outside the open workspace
  closes it after clearing filters; the Back action restores the origin's
  workspace together with its filter (`JumpOrigin::workspace`).

### Sessions outside every workspace

- The fixed `[NONE-WORKSPACE]` row follows `[ALL]`, before the divider and
  stored workspaces. It opens `WorkspaceScope::Unassigned` as soon as the
  cursor moves onto it. The table title and palette use `None-Workspace`.
- A session is shown only if **none of the saved workspaces matches it**,
  including each workspace's full-path folder condition and include/exclude
  words. This is a complement of all saved scopes, not a test for an unselected
  folder. Ordinary filters still narrow the result; they never change membership.
- With no saved workspaces it shows every session. A saved workspace without
  folder or word restrictions covers every session, leaving this scope empty.
- Enter does not edit it and delete is refused. `+` still opens a new draft;
  Cancel keeps this scope, while Save opens the new workspace. Closing/reopening
  the pane preserves it. Context-source Back restores it like a saved scope.
- `workspaces::membership` prepares normalized words and full-path folder sets
  once, then tests the session index without reading session files.
  `WorkspaceScreenState::membership` lazily caches one boolean per session.
  Ordinary filters and pane/palette navigation reuse it. Successful workspace
  saves/deletions/reloads and `App::rebuild_all_folders` after session index
  replacement/removal invalidate it. Title/folder changes rescan the index.
  The folder filter's counts use the same `App::retain_workspace_scope` helper.

## Matching (`Workspace::matches`)

| Attribute | Rule |
| --- | --- |
| folders | Empty = no restriction. Otherwise the session's absolute `cwd` must equal one of them. Full paths, not the basename-keyed folder filter, so `~/a/api` and `~/b/api` differ |
| includes | Whitespace-separated, NFC-lowercased words; every word must match |
| excludes | Same tokenization; any matching word hides the session |

Words match through `filter::token_matches`, the same text as `/` search
(user turns, title, folder, last assistant answers, and long session-id tokens).

## Editing (the edit dialog)

- The dialog edits a draft (`WorkspaceDialog`): a copy of the workspace, or a
  fresh `New Workspace` (`New Workspace 2`, … when taken) that is not in the
  store. Nothing is written and the session list does not change until
  **Save**. Cancel or Esc drops the draft; a new workspace then never existed.
- Rows: the left column holds the `Name`, `Includes`, and `Excludes` boxes and
  a read-only `Matches  N of M sessions` (sessions the draft matches over every
  session, recounted on every edit and folder toggle); the right column holds
  the `Folders` heading, a `Search` row, and the folder rows; the
  `Save`/`Cancel` buttons sit below both. Name, Includes, Excludes, and Search are text
  rows: typing edits them directly with no edit mode. A new workspace's
  suggested name starts selected, so typing replaces it.
- Keys: `↑`/`↓` move one row in field order across both columns (`↓` on
  Excludes moves to Search at the top of the folder column), past the last
  folder onto the buttons; Tab /
  BackTab move between groups (each field, Search, the first folder when one is
  shown, the buttons), wrapping. On text rows `←`/`→`/`Home`/`End` move the text
  cursor and Enter moves to the next row — a text row never submits. On folder
  rows `space` or Enter toggles, `j`/`k` move, `g`/`G`/`Home`/`End` jump to the
  first/last folder, and `←`/`h` return to the left column, on the field last
  edited (Name until one is; moving over a field does not count). On the
  buttons `←`/`→`/`h`/`l` switch between Save
  (focused first) and Cancel, and Enter runs the focused one. ctrl/alt
  combinations do nothing, so the palette, `ctrl+u`, `ctrl+w`, `/`, and
  delete are unavailable while the dialog is open.
- Save validates the trimmed name: empty, `All`, and `None-Workspace`
  (reserved: palette rows are labelled `Open Workspace <name>`, so a name must
  identify one row), and a name another workspace holds case-insensitively are
  refused. A refusal, or a
  failed write (e.g. another instance saved that name meanwhile), keeps the
  dialog open, puts the reason on the notice line in the error color, and
  returns the cursor to Name; the next edit clears it.
- A saved new workspace is opened (`Upsert` + `Opened`) and the pane cursor
  lands on its row. A saved edit is one `Upsert`. Either way the session list
  restarts at the top under the saved scope.
- Folder rows list the draft's selected folders first, then every other
  session cwd; each group is ordered by latest session activity (newest
  first, ties by path), and a selected folder with no remaining session sorts
  last in its group. The stored selection order is not used. The order is
  captured when the dialog opens (and rebuilt by a session rescan,
  `App::refresh_workspace_folders`) and kept while toggling, so `space` never
  moves the cursor. A stored folder with no remaining session still appears so
  it can be unchecked. `[✓]` = selected. Each row ends with a dim ` (N)`: the
  folder's sessions across every session, not narrowed by the session filters
  or the workspace's include/exclude words, so a stored folder with no session
  left reads `(0)`.
- On the Search row typed characters include letters that are shortcuts
  elsewhere (`j`, `k`, `g`, `q`, …) and `space`; Backspace/`Delete` delete at
  the cursor; Esc clears a non-empty query (an empty one cancels the dialog);
  Enter moves to the first match; paste inserts. Arriving on the row with a
  non-empty query selects the whole query (`TextInput::select_all`): typing or
  paste replaces it, Backspace/`Delete` clear it, and `←`/`→` drop the
  selection to the start/end, keeping the text. Leaving the row drops the
  selection.
- Every whitespace-separated query word must occur, case-insensitively, in the
  folder's full path or its displayed label. Non-matching rows are hidden, not
  reordered; their selection is kept and still counted in `· N selected`. The
  query is not stored; every dialog opens with it empty.
- A paste goes to the text row under the cursor; on a folder row or the
  buttons it is dropped.
- `ctrl+d`/`del` in the pane asks for confirmation (Cancel focused) and removes
  only the workspace; the cursor stays on the same row. It is the only place a
  workspace is deleted. "All", `[NONE-WORKSPACE]`, and `[NEW WORKSPACE]` cannot
  be deleted. On the Session screen `ctrl+d` deletes the workspace only while the pane has focus;
  on the table it deletes the session.
- `/` opens the shared keyword search from the pane. Esc returns to it;
  Enter/Tab focus the session list, closing it.

## Palette (`ctrl+w`)

- `ctrl+w` on Session (table or pane), Detail, and Profile opens the Quick Command
  palette with `open workspace ` typed. The query stays editable.
- `build_workspace_items` adds rows only for a non-empty query, so the plain
  `:` palette is unchanged: `Close Workspace` first, then the fixed
  `Open Workspace None-Workspace`, then `Open Workspace
  <name>` in list order, then the matching registry commands (for this query,
  `Open Workspace Window`). `Close Workspace` carries the `open` alias so the
  prefilled query lists it. While "All" is open it is disabled and sorts after
  the open rows.
- Running a workspace row opens that scope and switches to the **Session**
  screen with the table focused (the pane closed). Workspace
  rows are not recorded in palette history.
- `Open Workspace Window` shows the workspace pane; it is disabled only while
  the pane already has focus.

## Storage

`~/.config/s7s/workspaces.json`, versioned JSON (`STORE_VERSION`), mode `0600`:
`{ version, workspaces: [{ id, name, includes, excludes, folders }], active }`.

- The JSON format is unchanged: `WorkspaceScope::Workspace(id)` serializes
  as the id string; `All` and `Unassigned` serialize as null. No synthetic
  workspace is added to the stored list. A null field loads as `All`.
- Ids are stable; `active` survives renames. An `active` id that no longer
  exists loads as "All".
- The TUI does not restore `active`: every start opens "All"
  (`WorkspaceStore::load_at_startup`). The field is still written (`Opened`)
  and read by `load`, so the format is unchanged.
- Every save is a `WorkspaceChange` (`Upsert` / `Remove` / `Opened`) applied by
  id onto a freshly read file under `store_lock::with_store_lock`, then an
  atomic replace (temp file + rename). So a committed edit, folder toggle,
  add/delete, or scope change never drops workspaces another running s7s
  saved. No fsync: `Opened` is written on each pane cursor move. A commit
  also writes the workspaces in the sorted order.
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
  workspace was deleted elsewhere, "All" opens. An open `[NONE-WORKSPACE]`
  stays open and its membership cache is invalidated on a successful reload.
- An unreadable or newer-version store is never overwritten: startup keeps an
  empty in-memory store and reports it; each save re-reads the file and fails
  with a status message; `ctrl+u` shows a `Reload Failed` dialog and keeps the
  in-memory copy. Unit tests run with saving disabled
  (`App::workspaces_path = None`) unless a test sets a temporary path.

## Layout

See [ui-style-guide.md](./ui-style-guide.md) §Workspace pane and §Workspace
edit dialog.

## Verification

- `ui::workspace::tests` (pane open/close and Profile moves, scope and the
  `[NEW WORKSPACE]` row, name ordering, dialog open/cancel/save, refused
  names, the match count, Enter/Tab/arrow movement, folder toggles, folder
  search, paste routing, delete, keys the dialog ignores, palette open/close,
  context jump, persistence, dialog render and height),
  `ui::workspace::render::tests` (dialog size),
  `ui::render::tests::workspace_pane_shrinks_the_body_and_hides_a_narrow_prompt`,
  `workspaces::tests` (matching, ordering, per-change commits, refused names,
  unreadable store),
  `ui::reload::tests` (`ctrl+u` reload), and `store_lock::tests`.
- Release PTY check (`s7s demo`): `←` from the session list and move through
  workspaces, add one with `+` and from `[NEW WORKSPACE]` (Save and Cancel),
  check the pane lists names in text order, rename one and see it move, toggle
  folders and type includes/excludes and watch `Matches` (the session list
  changes only after Save), filter folders on the Search row and toggle a
  match, delete a workspace, open/close via `ctrl+w`, restart with a workspace
  open and confirm "All" opens. Check the dialog height on a 24-row and a
  40-row terminal (it grows with the folders up to 90% of the height and does
  not change while typing a query), and an 80-column terminal (Prompt hidden
  while the pane is open) and a 120-column one (Prompt kept).
- Release PTY check for `[NONE-WORKSPACE]`: confirm the complement across
  folder and word conditions, the empty-store and unrestricted-workspace cases,
  Enter/delete protection, `+` Cancel/Save, pane reopen, keyword filters,
  `ctrl+w` open/close, and recomputation after workspace edits and `ctrl+u`.
  Automated scenarios live in `ui::workspace::tests::unassigned_*`,
  `workspace_save_and_delete_invalidate_unassigned_membership`,
  `session_scan_replaces_membership_even_when_the_session_count_is_unchanged`,
  and `reload_keeps_unassigned_open_and_recalculates_changed_workspace_rules`.
- Two release instances on `s7s demo`: add a workspace in each without
  reloading, confirm `workspaces.json` holds both, then `ctrl+u` in each.
