# UI Style Guide

> Status: Current
> Read when: Changing TUI layout, themes, focus, selected rows, dialogs, text
> omission, or expansion behavior.
> Entry points: `src/theme.rs`, `src/ui/render.rs`,
> `src/ui/components/modal.rs`, feature `render.rs` files

Use semantic theme roles and shared render primitives. Do not recreate a visual
rule inside an individual feature.

## Theme roles

`Theme` in `src/theme.rs` is the only palette contract. Render code consumes
semantic fields rather than literal colors.

| Role | Use |
| --- | --- |
| `bg`, `fg` | frame background and default text |
| `muted`, `dim` | labels/hints and faint separators |
| `accent`, `on_accent` | focused borders, titles, bullets, chips |
| `selection_bg`, `selection_fg` | focused table row |
| `selection_inactive_bg` | selected row in an unfocused table |
| `bookmark_bg` | subtle fill across an unselected bookmarked session row |
| `key_hint` | shortcut notation |
| `usage_high`, `usage_low` | usage status and logo accent |
| `button_*` | focused/unfocused dialog buttons |
| `success`, `warning`, `error` | severity |
| `agent_*` | CLD/CDX/AGY badges |

- Add a new visual role to `Theme`, every built-in palette, and custom-theme
  loading before using it. Do not inline arbitrary RGB values in render code.
- `Theme::soft_dim()` is secondary text. Add `Modifier::DIM` only for a third,
  deliberately fainter level such as omission markers.
- `Theme::base_style()` must be painted under the full frame and repainted after
  `Clear`; otherwise a custom background leaks terminal-default cells.
- Built-in `bookmark_bg` colors mix 12% accent into the base background.
  Custom themes inherit this role from `base` and can override it in `[colors]`.
- Severity and state must not rely on color alone. Pair them with text, symbols,
  border thickness, or modifiers.

## Focus and information hierarchy

`titled_block_nav` is the shared panel frame:

| State | Border and title | Style |
| --- | --- | --- |
| Focused | `Thick` | theme accent + bold, with the `←`/`→` hints |
| Faded (another pane has focus) | `Plain` | `soft_dim()` |
| Unfocused, nothing focused (overlay, search prompt) | `Plain` | default style |

The caller passes `dimmed` to `titled_block_nav`; the frame never derives it.

Every pane except the focused one fades, on every screen and whatever the pane
holds (a list or text to read). Fading a pane means:

- Its border and title use `soft_dim()` (table above); only the focused pane is
  thick.
- Its text, headers, and tags use `soft_dim()`; colored headings (accent, agent,
  key, success) drop their color and keep only their bold.
- A selected row uses `selection_inactive_bg`, `soft_dim()`, and a weak
  `REVERSED` signal.
- Separators and the third-level omission markers keep their own fainter styles.

Exception: while an overlay or the search prompt owns input no pane is focused
and none fades, so a dialog is not drawn over a faded screen. A selected row then
reads `selection_inactive_bg` + `selection_fg` + bold. Pane renderers therefore
compute `dimmed` as "another pane has focus *and* the mode is the table mode",
not as `!focused`. The workspace edit dialog (`UiMode::WorkspaceEdit`) is an
overlay like any other: the pane loses focus and the backdrop dims.

Where it applies: Session table, Prompt, and workspace pane (shown only while
focused); Detail Prompt and Work & Answer.

When Session is focused, its selected row uses `selection_bg`/`selection_fg`
plus bold, and the Prompt fades.

Inspect border, title, header, normal rows, and selected row together. The
selected-row style is applied last and can otherwise defeat the intended focus
hierarchy.

Labels and supplemental metadata use `soft_dim()`. Highlight only the value that
drives the current decision. Avoid using bold for every value.

## Session metadata grid

`session_meta_lines` and `meta_grid` own the shared Session/Detail header shape:

```text
● Session
- Project: <folder> (<full path>)
- Name: <title>
- Created at: <time>
- Updated at: <time>
- Id: [TAG] <id>
```

- `Project` folder and `Name` are primary values; labels, full path, timestamps,
  and ID are supplemental.
- A context-derived session inserts `Context Source` above Q1. Its conditional
  `ctrl+o` heading hint appears only when the source resolves and sufficient
  width remains. Do not put cursor-dependent actions in the global header.
- `ui/copy.rs` mirrors the metadata content without visual hints. Keep copied and
  rendered content aligned.
- Claude live-session markers (`Ⓑ` background, `Ⓞ` open) follow `♥` in the
  same prefix (`♥ Ⓑ  title`), bold and display-only; they do not tint the row. See
  [background-sessions.md](./background-sessions.md).
- The Prompt pane's Session block explains each marker the session carries in
  `- <marker> : <meaning>` rows under Name (bold marker, soft-dim text,
  truncated to the pane). Detail and Context Source blocks and `ui/copy.rs`
  omit them.
- The Attach dialog for a background session follows the delete dialog: action
  button left, Cancel right and focused by default, warning border. Blocks for
  live sessions use the shared message dialog (`show_message`, `Warn`).
- Bookmarked titles carry `♥ ` in the table and metadata grids. This prefix is
  display-only; clipboard text retains the original title. In the table the
  marker is bold even on an unselected or unfocused row, without changing the
  title text's tone. The entire bookmarked row uses `bookmark_bg`, including
  column gaps and inner margins. The existing focused/unfocused selection style
  takes precedence. Bookmarked rows precede ordinary rows, retaining activity
  ordering within each group. See [bookmarks.md](./bookmarks.md).
- Each Prompt `Qn` heading shows an available local submit timestamp in
  `YYYY-MM-DD HH:MM:SS` using `soft_dim()`.

## Header width

- Preserve profile/usage and shortcut columns before the decorative logo.
- Show the logo only when the complete left side, gap, and logo fit.
- If still narrow, drop shortcut columns from right to left; never wrap or
  overlap them.
- The five-row header permits at most five actions per shortcut column. The
  `header_shortcut_columns_fit_the_five_row_header` test guards this ceiling.
- A longer label widens its column and hides later columns sooner. Keep action
  labels within existing widths.
- Header action labels must not duplicate control names used by render-buffer
  assertions elsewhere on the screen.
- Session/Detail operations include `<ctrl+b> Bookmark`; `ctrl+o` remains on the
  conditional Context Source heading and in Help. The Back action is palette-only.
- The common column carries `<ctrl+w> Open Workspace` (five rows, the ceiling).
- The Session screen shows `SHORTCUTS_WORKSPACE_LIST` instead of
  `SHORTCUTS_SESSION` while its workspace pane has focus.

## Dialogs and overlays

Use the shared primitives in `ui/components/modal.rs`:

- `modal_block` for thick titled framing;
- `render_modal` to clear/repaint the outer area and inset the frame by one cell,
  protecting borders from background double-width glyphs;
- `button_styles` for theme-aware focused and unfocused buttons;
- `form_input` for a single-line text input: a three-row box titled with its
  label, `Thick` accent while focused and `Plain` `dim` otherwise;
- `draw_combo` for a closed combo box (the New Session `Profile`, `Model`,
  `Folder` and the workspace `Folders`): framed like `form_input`, titled with
  the name alone, and a `▾` at the inner right end (just inside the right
  padding) in the border's style — accent + bold while focused or open, `dim`
  otherwise, never the error color of an invalid value. It returns the value
  area, the inner width minus the `▾` and one gap cell; draw the value, any
  right-aligned note (the `N folders` count), a placeholder, and a text-input
  combo's `input_view` width and cursor only there, so nothing covers the
  `▾`. Below three inner cells the `▾` is dropped. The `▾` stays while the
  popup is open (the popup overlaps only the combo's bottom border), and a
  read-only control (the Context Source) has none;
- `dropdown_frame` / `dropdown_divider` for a dropdown popup's frame and its
  thin `┠─┨` rows;
- `dim_backdrop` only for modes selected by `backdrop_dimmed`.

Additional rules:

- A dialog with buttons uses `Padding::new(1, 1, 1, 0)`: one blank row under the
  title. Only the search-backed lists without buttons (Quick Command, Select
  Folders) drop the top padding so their input line sits under the title.
- Wrap a form's text inputs in `form_input` boxes. The one unboxed text input
  in a form is a search line that filters a list right below it (the search
  line of the workspace dialog's folder checklist), drawn like the Select
  Folders search line.
- A form that also edits a long list keeps one column of stacked boxes and
  moves the list into a combo's dropdown popup (the workspace dialog's
  `Folders`), so the dialog keeps the standard 80% width cap and a fixed
  height. Put such a combo high in the form: the popup opens downward over
  the rows below it, and the rows above it cost list rows.
- Keep action order `[Confirm/Execute] [Cancel]`.
- A text-input Enter must not submit a form. Submission occurs only when the
  confirm button owns focus; Enter on Cancel closes the dialog.
- Folder filter is the exception: it is a search-backed selection list, so Enter
  selects the focused item and confirms idempotently.
- Keep at least one blank row above buttons. Expand modal height when an error or
  information row would collapse that spacing.
- Prefer a notice line that every state fills over one that appears conditionally:
  the profile form always fills it, so the dialog keeps a single height and the
  buttons do not shift between its add and edit variants. When a dialog has
  nothing standing to say there, reserve the line and leave it empty (the
  workspace edit dialog shows only a refused Save's reason) rather than adding
  the row only on an error or filling it with hints that repeat the labels or
  the status bar.
- Put a field's rule in its title when the label alone would not tell it
  (`Includes · all words` / `Excludes · any word`) instead of in a hint line.
- A locked control stays visible and dim rather than disappearing, and the notice
  line says why it is locked. Keep the selected entry of a locked radio row readable
  with `Modifier::BOLD` over `soft_dim()`; the marker alone is not enough signal.
- Clamp dynamic list modals to a usable minimum height when there are no results.
- A popup drawn over background text must clear enough adjacent cells to remove
  both halves of a clipped double-width glyph before painting its border.
  Limit those extra cells to rows over the backdrop: a popup that starts inside
  its dialog (the New Session and workspace dropdowns) must not clear the
  dialog's own border, or one side of the dialog's bottom edge loses its `━`
  next to the popup. `components::modal::dropdown_frame` applies this rule and
  joins the popup to its combo as `┣━┫`; `dropdown_divider` draws the popup's
  `┠─┨` rows. Both take the focused combo's accent + bold border style, so the
  join reads as one line; a divider's middle `─` is `dim`.
- Theme selection does not dim its backdrop because the background is the live
  preview.

### Dropdown rows

- The profile and model lists carry a `soft_dim()` note beside each label.
- Single-select rows (the profile and model lists) mark only the committed
  value, with a bold `●`; every other row gets a one-cell blank in its place,
  so labels stay aligned and no `○` is drawn. An open popup already reads as a
  list of choices, so an empty-circle mark on every other row is noise. The
  cursor row is told apart by `selection_bg`, the committed value by `●`.
- Every folder list (folder dropdown, Change Folder pick list, folder filter,
  workspace folder checklist) ends each row with its session count as ` (N)`, right
  aligned and in `soft_dim()` (`text::count_note` + `text::fit_before_note`).
  Both the brackets and the dim color are required: several folder rows are
  already dim (unmatched rows, an unfocused pane), and a folder name can end in
  a number (`release 2`). A narrow row truncates the label and never drops the
  count. Folder labels themselves stay bare basenames.
- The folder dropdown resolves the focused row instead, in a footer inside the
  popup: a `┠─┨` divider joined to the thick side borders, then the full path in
  `soft_dim()`. The footer is what makes basename rows distinguishable before
  selection, so keep it aligned with the list rows.
- Reserve the footer rows out of the popup height, and drop the footer when the
  available height leaves no usable list. A preview never costs the list its last
  rows.
- A text-input combo box paints a whole-value selection
  ([terminal-input-hardening.md](./terminal-input-hardening.md)) with the
  selection colors, and its dropdown opens with the highlight left in the input
  instead of on a row: while the selection is live the value is the subject, and
  the footer names both ways out of it (replace by typing, edit with `→`).
- Paint that selection only while the field itself is focused. The selection
  survives a focus move (the folder dialog opens with a prefill already selected
  while focus sits on another control), so an ungated paint leaves an inactive
  control permanently reversed instead of showing a pending edit.
- A multi-select dropdown (the workspace `Folders` checklist) follows the
  checkbox-list keys rather than the single-select dropdown ones: Enter or
  `space` opens the closed combo, `space` toggles the cursor row at once, and
  Enter, Esc, and Tab only close (Tab also moves on), so closing never
  changes the selection. Its rows carry `[✓]`/`[ ]`. A search-backed one takes
  typed characters into a search line at the top of the popup; Esc clears a
  non-empty query before it closes. The closed combo shows the selection as a
  summary: the "everything" value for an empty selection, else the selected
  names in the default color, cut at name boundaries with `…`, and for two or
  more a count right-aligned in `soft_dim()` at the right end of the value
  area, one gap cell left of the `▾`. Spell the count out
  (`12 folders`): a bare `(N)` already means a session count in folder rows.
  Never dim the names themselves — in a form, dim marks an empty value
  (`(none)`) or a note, not a selection.
- A fixed option (the folder dropdown's `[SCRATCH]` row, the checklist's
  `[ALL FOLDERS]` row) renders outside the
  query-ordered list and always first. With no note column, its label carries the
  distinction: bracketed and upper case against bare basenames.
- Do not encode a fixed option as an entry of the data list: query reordering
  moves it, and the cursor then addresses the wrong row.
- The selected-row style is applied over span styles, so a note or footer stays
  dim on the cursor row. Never make one the only signal a row is selected.

## Preview omission and expansion

- A user turn of at most eight original lines is shown in full. Longer turns
  show the first four and last four lines with the exact omitted-line count.
- Count original lines before width wrapping. `wrap_w` display rows do not alter
  the omission count.
- Render the omission marker as its own `soft_dim() + DIM` span.
- Session Prompt `.` toggles every turn through `App.preview_expanded`; changing
  the selected session resets it.
- Detail Prompt `.` expands only the selected truncated turn. Up/Down scroll
  within a tall expanded turn instead of moving the selection.
- Detail Work & Answer `.` toggles tool visibility and removes per-entry display
  caps.
- Both preview paths use `preview_turn_display(turn, expanded)` so collapsed and
  expanded forms cannot drift.

## Session table width

- The Session table keeps A, FOLDER, and TITLE. FOLDER is `FOLDER_MIN_W` (12)
  cells, widened to the longest folder label of all loaded sessions (not the
  filtered ones, so typing a search does not shift columns) up to
  `FOLDER_MAX_PERCENT` (20%) of the table width, never below the minimum.
- When TITLE would drop below `TITLE_MIN_W` (25) cells, `table_layout` in
  `src/ui/session/render.rs` first shrinks FOLDER back toward 12, then hides
  SIZE, then Q, then UPDATED. FOLDER widens only while every column is shown,
  so narrowing the window never widens it. Q and UPDATED stay visible in the
  Prompt panel (`Qn` headings, `Updated at:`); SIZE has no other display.
- Header cells, row cells, width constraints, and TITLE truncation all come from
  the same `TableLayout`, so they cannot disagree. The last visible fixed column
  keeps the one-cell right margin before the border (Q takes it over from SIZE).
- The `table_layout` unit tests in `src/ui/session/render.rs` pin the
  thresholds and FOLDER bounds; `session_table_hides_optional_columns_on_narrow_terminals`
  checks the frame.

## Workspace pane

- The Session screen's workspace pane is drawn left of the session table only
  while it has focus (`render::body_layout`). It is 24 cells wide including
  borders (`workspace::render::LIST_MAX_W`); the table and Prompt split the rest
  58/42 as usual. When that leaves the Prompt under `PROMPT_MIN_W` (40) cells,
  the Prompt is hidden and the table takes its width: the pane exists to show
  which sessions a workspace holds. The Prompt never has focus while the pane
  is shown, so hiding it moves no focus.
- The pane is a focused `titled_block_nav` with both arrows (`←` Profile,
  `→` close). The session table and the Prompt fade like any unfocused pane.
- Rows: the fixed `[ALL]` first, the stored workspaces, `[NO WORKSPACE]`, then
  the fixed `[NEW WORKSPACE]` last. All fixed rows stay outside the stored
  list and use bracketed upper-case labels (as with `[SCRATCH]`) and the
  `key_hint` color on an unselected row. A `dim` divider follows `[ALL]` only
  when stored workspaces exist; another divider separates `[NO WORKSPACE]`
  from `[NEW WORKSPACE]`. The cursor never lands on a divider.
  The labels are list-only
  (`workspace::render::{ALL_WORKSPACE_LABEL, UNASSIGNED_WORKSPACE_LABEL}`);
  palette rows and messages keep the names "All" and "No Workspace".
  The cursor row is the open scope
  (or `[NEW WORKSPACE]`, showing "All"). Stored workspaces are in name order (see
  [workspaces.md](./workspaces.md) §The open workspace).
- While an overlay (the edit dialog included) or the search prompt owns input
  the pane is drawn unfocused, its cursor row in `selection_inactive_bg` +
  `selection_fg` + bold.

## Workspace edit dialog

- `workspace::render::draw_workspace_dialog`: a `modal_block` titled
  ` Edit Workspace ` or ` New Workspace ` with the standard dialog padding (one
  blank row under the title), over the dimmed Session screen. Width 86
  including the outer margin, capped at 80% of the terminal width.
- One column, top to bottom: the `Name` box, the `Folders` combo, the
  `Includes · all words` and `Excludes · any word` boxes, then
  `Matches  N of M sessions` (N in the default color, the rest `soft_dim()`).
  Below it: the notice line, a blank row, and the buttons. Like every form
  (the profile form included) it has no divider above the buttons; thin
  dividers belong to list dialogs, between a search line, the list, and a
  footer.
- Height (`workspace::render::dialog_size`): a fixed 19 rows (clipped by a
  shorter terminal). The folder list is a popup, so neither the folder count
  nor a query changes the dialog.
- The three text fields are `form_input` boxes. A whole-value selection (a
  new workspace's suggested name) paints only the text with
  `selection_fg`/`selection_bg`, as with the combo-box selection rule. An
  empty unfocused Includes/Excludes box reads `(none)` in `soft_dim()`.
- `Folders` is a `draw_combo` box (`Thick` accent while focused or open,
  `Plain` `dim` otherwise, `▾` at the inner right end) and shows
  `folder_summary` sized to the value area: `All folders`, one basename, or
  the basenames joined by `, ` (whole names only, `…` when some are left out)
  with `N folders` right-aligned in `soft_dim()` just left of the `▾` gap and
  at least one blank cell before it. A row narrower than the count keeps only
  the count.
- The open checklist (`draw_folder_list`) is a popup joined under the combo
  by `dropdown_frame` with an accent border, drawn after the buttons. It
  covers the rows below the combo and extends past the dialog down to the
  terminal bottom when it needs to; its height comes from every folder, not
  the search matches, so typing a query never resizes it. Inside: the search
  line, a `┠─┨` divider, the rows, and a footer (a divider and the cursor
  row's full path in `soft_dim()`; `[ALL FOLDERS]` reads `Every folder: no
  folder restriction`). The footer is dropped when fewer than two rows would
  remain. Rows past the height scroll, with the scrollbar on the popup's right
  border.
- The search line: `Search` in a bold accent label column with the hardware
  cursor (it always has focus while the checklist is open) and no
  placeholder. The cursor row style is never applied to it.
- Checklist rows: `[✓]`/`[ ]`, the bare basename (or the fixed
  `[ALL FOLDERS]`), and a right-aligned ` (N)` session count. A checked mark
  is accent + bold, so selection does not rely on the mark alone. The cursor
  row uses `selection_bg` + bold. An empty result adds `No matching folders`
  (or `No session folders`) in `soft_dim()` under `[ALL FOLDERS]`.
- The notice line shows only a refused Save's reason, in the error color, and
  is empty otherwise; it stays reserved so a refusal never moves the buttons.
  Fields carry no hint text: the titles state the word rules, the combo shows
  its selection, and the status bar shows the keys.
- Buttons `[Save] [Cancel]` are centered with one blank row above them and are
  highlighted only while the button row has focus (Save first).
- The status bar shows the dialog's keys while it is open, switching to the
  checklist keys while the checklist is open; both fit an 80-column terminal.

## Width and root layout

- Use `ui/components/text.rs` for width-aware padding, truncation, and wrapping.
  Never use byte or scalar counts as terminal-cell width.
- Preserve one-cell right margins for long list names while allowing the selected
  highlight to occupy the complete row.
- `draw_status_bar` renders inside `area.inner(Margin::new(1, 0))`. Do not encode
  the root footer inset as spaces in each message branch.
- Padding inside a colored status chip is content styling and remains in the
  string.

## Verification

- Run render-buffer tests and the baseline in [testing.md](./testing.md).
- Perform a real TUI or PTY check for layout, focus, theme, or popup changes.
- Check narrow and wide terminals, a light and dark theme, and content containing
  CJK or emoji where clipping boundaries changed.
- Toggle focus repeatedly and verify the complete hierarchy, not just borders.
- Verify the 8-line omission boundary and all three `.` behaviors.
- Confirm every dialog can reach all controls and that text-input Enter does not
  execute the primary action.
