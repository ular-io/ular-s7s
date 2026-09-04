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
- Severity and state must not rely on color alone. Pair them with text, symbols,
  border thickness, or modifiers.

## Focus and information hierarchy

`titled_block_nav` is the shared panel frame:

| State | Border | Style |
| --- | --- | --- |
| Focused | `Thick` | theme accent + bold |
| Unfocused | `Plain` | default style; do not dim the border |

When Prompt is focused:

- Session header, rows, and agent tag use `soft_dim()`.
- The selected Session row uses `selection_inactive_bg`, `soft_dim()`, and a
  weak `REVERSED` signal.
- Prompt alone keeps the thick accent border.

When Session is focused:

- Its selected row uses `selection_bg`/`selection_fg` plus bold.
- Prompt uses a plain inactive border.

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

## Dialogs and overlays

Use the shared primitives in `ui/components/modal.rs`:

- `modal_block` for thick titled framing;
- `render_modal` to clear/repaint the outer area and inset the frame by one cell,
  protecting borders from background double-width glyphs;
- `button_styles` for theme-aware focused and unfocused buttons;
- `dim_backdrop` only for modes selected by `backdrop_dimmed`.

Additional rules:

- Keep action order `[Confirm/Execute] [Cancel]`.
- A text-input Enter must not submit a form. Submission occurs only when the
  confirm button owns focus; Enter on Cancel closes the dialog.
- Folder filter is the exception: it is a search-backed selection list, so Enter
  selects the focused item and confirms idempotently.
- Keep at least one blank row above buttons. Expand modal height when an error or
  information row would collapse that spacing.
- Clamp dynamic list modals to a usable minimum height when there are no results.
- A popup drawn over background text must clear enough adjacent cells to remove
  both halves of a clipped double-width glyph before painting its border.
- Theme selection does not dim its backdrop because the background is the live
  preview.

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
