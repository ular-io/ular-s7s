# Preview Omission Style

This document outlines the method for omitting the middle content of long user queries in the prompt preview, and things to check when adjusting terminal styles.

## Behavior

- If the user query is 8 lines or fewer, all lines are displayed.
- If it exceeds 8 lines, only the first 4 lines and the last 4 lines are displayed.
- In the middle, the actual number of omitted lines is displayed in the following format.

```text
────── ⋯ 5 lines omitted ⋯ ──────
```

It is first abbreviated based on the original number of lines, and then each display line is wrapped according to the preview panel width using `wrap_w`. Therefore, lines that are automatically wrapped due to panel width are not included in the abbreviation criteria.

## Implementation Locations

- Abbreviation logic: `preview_turn_lines` in `src/ui/render.rs`
- Omitted line representation: `PreviewTurnLine::Omission`
- Color and attributes: `Span::styled` in `draw_preview`

The final style is as follows.

```rust
Style::default()
    .fg(Color::Gray)
    .add_modifier(Modifier::DIM)
```

`Color` specifies the foreground color, and `Modifier::DIM` requests the dim text attribute from the terminal. The omission indicator must be rendered as a different `Span` from the normal body text so that the style does not propagate to the body.

## Trial and Error

### `DarkGray`

- Initially, `Color::DarkGray` was used.
- Depending on the background color or terminal theme, it became too dark, making it difficult to read the omission indicator.

### `Gray`

- Changed to `Color::Gray` to increase brightness.
- The omission indicator became easier to read, but there wasn't enough contrast with the normal body text.

### `Rgb(190, 190, 190)`

- Used an RGB color to specify a lighter gray.
- The brightness was similar to the existing `Gray`, and depending on the terminal's color approximation or theme handling, it could look identical.
- Adjusting only the RGB value slightly does not guarantee a consistent visual difference across terminals.

### `Gray + DIM`

- Ultimately, the `DIM` attribute, which is independent of color, was used together with the color.
- By not relying solely on color value differences, the omission indicator remains readable while being distinguishable as dimmer than the body text.

## Checklist When Modifying

- Run `cargo test -q` to verify the 8-line boundary and the calculation of omitted lines.
- Verify in the actual TUI whether the terminal being used supports the `DIM` attribute.
- Some terminals or user themes may ignore `DIM` or convert it into a color.
- If the style difference is not visible, adjust attributes like `BOLD`, `ITALIC`, `DIM`, along with the phrasing and separator lines, rather than fine-tuning the RGB.
