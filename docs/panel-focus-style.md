# Panel Focus Style

This document outlines the focus style rules for the `session` panel and the `prompt` panel, along with the trial and error iterations made during these changes. The overall standards for color, thickness, and emphasis follow the [UI Standard Style Guide](./ui-style-guide.md).

## Goals

- When moving focus with `Tab`, the state of the active panel and inactive panel must be immediately distinguishable.
- Focus differentiation is represented by a dual signal of **color (ACCENT) + border thickness (BorderType)**.
- The body/selected rows of the inactive `session` panel are unified into the same pale tone (`soft_dim_style`).

## Current Rules

### Borders (Common to both panels, `titled_block_nav`)

- Focused: `BorderType::Thick` (thick line) + `focus_color + BOLD` (Generally `ACCENT` is injected, but individual colors can be specified for things like the session table)
- Unfocused: `BorderType::Plain` (thin line) + default color. **Borders are not dimmed.**
  (The past treatment of dimming with `soft_dim_style` has been reverted.)

### When `prompt` is focused

- The `prompt` panel border/title is a thick emphasis line
- The `session` panel border is a thin default line, and the header/normal rows/agent tag are `soft_dim_style`
- The selected row maintains the pale color scheme and applies only a weak inversion (`REVERSED`)
- Each Session and Detail Prompt `Qn` heading shows an available local submit
  timestamp in `YYYY-MM-DD HH:MM:SS` format; the timestamp always uses
  `soft_dim_style`, including while its Detail row is selected.

### When `session` is focused

- The `session` panel maintains the emphasis style (thick border)
- The selected row uses `Color::Cyan` background + black text + `BOLD` (fixed cyan regardless of the overall theme colors)
- The `prompt` panel only displays a thin inactive border/title

## Implementation Locations

- Common inactive style: `soft_dim_style` in `src/ui/render.rs`
- Border branching by focus: `titled_block`
- `session` panel focus branching: `draw_table`
- `prompt` header meta information: `draw_preview`
- Base style for omitted lines in long prompts: [Preview Omission Style](./preview-omission-style.md)

## Trial and Error

### Attempted to dim only the background of the selected row

- Initially, we only lowered the background color of the selected row in the inactive `session` panel to a `Gray` shade.
- In reality, only the selected row changed, while the header/body/border still looked strong, so the panel as a whole did not look inactive.
- Conclusion: The focus style cannot be resolved by changing only the `row_highlight_style`.

### Attempted to dim only the text of normal rows

- Next, we changed the body text and header colors to be pale.
- However, the selected row continued to be overridden by a separate highlight style, making it still too bold.
- Conclusion: `Table::style` and `row_highlight_style` must be designed together.

### Fine-tuning the gray RGB values

- Adjusted the highlight background with values like `Rgb(70, 70, 70)`.
- Depending on the terminal theme and the interpretation of `DIM`, the difference was either small, or the selected row sometimes looked even heavier.
- Conclusion: Rather than fine-tuning RGB, it is more consistent to first fix a common tone (`Gray + DIM`) and differentiate the selected state with a weak inversion.

### Emphasis hierarchy of labels and values

- The `prompt` header order is `Project` / `Name` / `Created at` / `Updated at` / `Id`.
- The labels are all `soft_dim_style`. Only the key values (`Project` folder name, `Name` value) are emphasized with `ACCENT + BOLD`, and the supplementary meta (`Created/Updated/Id` values, full path) values are also suppressed with `soft_dim_style` to create an information hierarchy.
- The standalone `Path` line was removed and integrated into the `Project` line in the form of `Folder Name (full path)`.

## Rules to Prevent Recurrence

- When changing the inactive panel style, inspect the `border`, `title`, `header`, `normal row`, and `selected row` as a set.
- If the selected row stands out, first check if `row_highlight_style` is ultimately overriding it.
- Instead of introducing a new inactive tone, first consider the possibility of reusing `soft_dim_style`.
- Do not try to solve terminal color differences merely with RGB values, but prioritize using attributes like `DIM`.

## Checklist

- Toggle panel focus several times with `Tab` and verify that only the focused panel gets the thick emphasis line.
- Check that the selected row does not stand out excessively in the inactive state.
- In the `prompt` header, ensure that only the `Project` folder name and `Name` value are emphasized, and the rest of the meta information is displayed palely.
