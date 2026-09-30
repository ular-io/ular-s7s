# Session Bookmarks

> Status: Current
> Read when: Changing bookmark storage, title markers, the bookmark filter, or Ctrl+B.
> Entry points: `src/bookmarks.rs`, `src/ui/bookmarks.rs`, `src/filter.rs`, `ui::App::rebuild_filtered`

## Behavior

- On Session (Table or Prompt focus) and Detail (either panel), `ctrl+b`
  toggles the focused session's bookmark. An empty list has no target.
- Quick Command exposes **Toggle Bookmark** with the same effect. It is disabled
  on Profile and when no session is selected.
- A bookmarked title has the prefix `Ⓑ  ` in the Session table, Prompt metadata,
  Detail metadata, and resolved Context Source metadata. The two separating ASCII
  spaces are intentional. Truncation measures the entire decorated title with the
  shared Unicode-width helpers; it never changes the original title.
- The marker is bold, including on unselected rows and with Prompt focus. The
  table title text keeps its existing tone; metadata titles are already bold.
- Bookmarked sessions appear before unbookmarked sessions. Each group retains
  the scan's latest-activity-first order; filtering, rescanning, and restarting
  apply the same priority. A toggle immediately reorders the list while keeping
  the cursor on the same session. Its preview scroll/expansion is preserved
  unless removing a bookmark excludes the session from the active filter.
- **Filter Bookmarked Sessions** is a palette-only toggle on Session. It combines
  with keyword, agent, folder, and profile filters using AND, preserves activity
  ordering within the bookmarked group, and adds `bookmarked` to the table's
  filter description. `0`, the ordinary clear-filter action, and list-focus
  `esc` clear it with the other filters. Removing a bookmark under this filter
  immediately removes its row.
- Context-source navigation retains its existing filter capture/restore rules,
  including the bookmark filter. **Back to Previous Session** is now palette-only;
  `ctrl+b` no longer navigates backwards. `ctrl+o` is unchanged and remains in
  Help and on the reachable Context Source heading.
- The Session and Detail headers show `<ctrl+b> Bookmark`. The fixed five-row
  header does not repeat the conditional source-jump shortcut.

## Storage and boundaries

- `config::bookmarks_path()` resolves `~/.config/s7s/bookmarks.json`, redirected
  under the isolated demo configuration in demo mode.
- `bookmarks::BookmarkStore` stores a versioned JSON set of identities containing
  agent, profile ID, and session ID. Titles, folders, and row indices are not keys.
  Bookmarks survive renames, rescans, cache rebuilds, and app restarts.
- The session index and agent-owned transcript/metadata remain unchanged.
  Clipboard and session CLI projections retain their original titles; `Ⓑ` is a
  TUI decoration rather than title content.
- Missing storage starts empty. Read/parse/version errors are shown at startup;
  a toggle re-reads the store and refuses to replace an unreadable/newer file.
  Successful toggles use a private (`0600` on Unix), unique temporary file,
  flush it, then atomically rename it over the store. In-memory state changes
  only after persistence succeeds. A failed write leaves the marker unchanged.
- Each toggle re-reads completed changes from other app instances. Simultaneous
  writes are not locked; the last atomic write wins. The active app does not poll
  the bookmark file for external changes.
- Records for unavailable sessions are retained, since a missing session may
  belong to a temporarily unscanned profile. They never create rows in the list.
- Key handlers enqueue `AppEffect::ToggleBookmark`; filesystem work remains at
  the existing synchronous effect boundary. Rendering and filtering use the
  loaded in-memory store, never filesystem access.

## Verification

- Baseline: `scripts/check.sh`.
- Storage/UI regression coverage: `bookmarks::tests`, `ui::bookmarks::tests`.
- Source-jump return behavior: `ui::context_jump::tests::palette_back_*` plus
  `ctrl_o_reopens_the_detail_screen_on_the_source`.
- Release TUI/PTY: use `s7s demo`, press `ctrl+b`, inspect `Ⓑ  ` before the table
  title and Prompt Name, restart, and confirm it persists. Toggle off again.
  Open Detail and repeat, then use the palette's bookmark filter and `0`.
- Bookmark an older row: it must move to the first group with the cursor
  following it; unbookmarking moves it back into normal activity order. Mark a
  second newer row and verify the bookmark group is ordered by activity, not
  the sequence of toggles. Restart/rescan and verify the same ordering. Move
  selection off a bookmarked row and inspect the marker's bold style.
- Check narrow/wide widths, dark/light themes, focus changes, and a CJK title.
  `Ⓑ` has ambiguous East Asian width; actual font/terminal rendering must be
  inspected as well as the buffer tests. Verify the glyph and next title
  character do not overlap. The supported layout uses the usual one-cell
  ambiguous-width setting, matching `unicode-width`/ratatui.
