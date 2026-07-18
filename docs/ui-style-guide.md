# UI Standard Style Guide

A single reference document to maintain consistency in colors, text highlighting, and border highlighting in TUI rendering. All style constants/helpers are defined in `src/ui/render.rs`. When adding new widgets or changing colors, use existing tokens from this document before creating new values.

## 1. Color Tokens

The top constants in `src/ui/render.rs` and the `agent_tag` helper are the only places where colors are defined.

| Token | Value | Purpose |
| :--- | :--- | :--- |
| `ACCENT` | `Color::Black` | Highlights (focused border/title, header, primary values) |
| `KEYCOL` | `Color::Rgb(120, 170, 255)` | Key notation in the top shortcut header |
| `DIM` | `Color::DarkGray` | Low-priority text like separators, informational messages |
| `USAGE_HIGH` | `Color::Rgb(80, 150, 255)` | Usage 50% or more (Blue), ASCII logo '7' part (Light Blue) |
| `USAGE_LOW` | `Color::Rgb(235, 90, 90)` | Usage under 50% (Red) |
| agent `Claude` | `Rgb(217, 119, 87)` | Table `A` (agent) tag `CLD` |
| agent `Antigravity` | `Rgb(120, 170, 255)` | Table `A` tag `AGY` |
| agent `Codex` | `Rgb(140, 220, 160)` | Table `A` tag `CDX` |

- If a new color is needed, do not inline arbitrary RGB; elevate it to a constant and add it to this table.
- Differences in terminal themes are resolved first using "modifiers" (BOLD/DIM/REVERSED) below, not through RGB fine-tuning.
- Standard 16-color ANSI colors like `Color::LightBlue` might not render as a proper light blue or might be ignored depending on the user's terminal theme, so always use the true color RGB value `USAGE_HIGH` for bright blue highlights such as the logo.

## 2. Text Highlight (Modifier)

| State | Style | Example |
| :--- | :--- | :--- |
| Highlighted Value | `fg(ACCENT) + BOLD` | Prompt header `Name` value, `Project` folder name, table headers |
| Non-highlighted/Supplemental | `soft_dim_style()` = `fg(Gray) + DIM` | Entire Prompt header label, `Created/Updated/Id` values, full path |
| Low Priority Text | `fg(DIM)` | Separators, status bar text, "No sessions" |

- The information hierarchy is expressed as: **labels are always `soft_dim_style`**, and **only core values are highlighted**. Supplemental metadata (created/updated times, ID, path) should also have their values suppressed using `soft_dim_style`.
- Use `BOLD` only for "the one thing the user needs to see right now." Overusing it destroys the hierarchy.

## 3. Border Highlight (BorderType)

Focus/active states are distinguished by a dual signal of **color (ACCENT) + thickness (BorderType)**.
Using color alone is ineffective in monochrome or colorblind environments, so adjust the thickness alongside it.

| State | BorderType | border/title style |
| :--- | :--- | :--- |
| Focused/Active | `Thick` (thick line) | `fg(ACCENT) + BOLD` |
| Unfocused | `Plain` (thin line) | `Style::default()` (default color, **no dimming applied**) |

- Container shared by the two panels (Session/Prompt): `titled_block(title, focused)`.
- Always-active UIs (search bar, all dialogs) always have a `Thick` highlighted border:
  - Search block of `draw_search_prompt` (`ACCENT`)
  - `modal_block` (Agent/Folder filters, Rename) (`ACCENT`)
  - Input block of `draw_rename_modal` (`ACCENT`)
  - `draw_delete_confirm` (`Color::Red` — destructive action)
  - `draw_message_modal` (general notifications) — highlight color by severity (see below)
- **Severity Colors**: Notifications/confirmation dialogs change their border and button colors based on their meaning.
  `MessageKind::Info` → `ACCENT`, `Warn` → `Color::Yellow`, `Error` → `Color::Red`.
  Destructive/blocking situations are `Red`, warnings are `Yellow`, and simple information is `ACCENT`.
- Unfocused panel borders are **not dimmed.** Past application of `soft_dim_style` has been reverted — dimming is reserved only for text hierarchy representation, while border distinction relies on thickness.

### 3.1 Dialog Internal Separators, Padding, and Button Design Rules

To improve the visual stability and polish of dialogs (modals), adhere to the following rules:

* **Border Adhesion and Thickness Consistency**:
  * The bottom separator of a modal must tightly adhere to the left and right modal borders without any gaps.
  * Since the modal block is drawn in an area with a 1-character left/right margin (`block_area`), the separator's X coordinate must be `area.x + 1`, and the width `area.width - 2`.
  * Because the modal border is `Thick + BOLD`, the separator must perfectly match the thickness using thick line symbols (`┣`, `━`, `┫`) and `Modifier::BOLD`.
* **Uniform Bottom Padding and Button Margin Structure**:
  * Set bottom padding to `0` to eliminate unnecessary blank space at the bottom of the dialog (`Padding::new(1, 1, 1, 0)`).
  * With a bottom padding of `0`, the row of buttons strictly adheres to the border directly beneath it, and you must ensure there is **always 1 line of empty space (`Constraint::Length(1)`) immediately above the buttons**.
  * When there are error/info messages, dynamically increase the dialog's vertical height (`h`) and constraints (e.g., 12 lines -> 13 lines) to prevent the 1-line margin structure between the message and the button row from collapsing.
* **Uniform Dialog Button Colors and Order**:
  * The button styles across all dialogs use the same color scheme.
    * **Focused** state: Text is white (`Rgb(255, 255, 255)`), background is light blue (`Rgb(80, 150, 255)`), with Bold effect applied.
    * **Unfocused** state: Text is dark gray (`Color::DarkGray`), background is light gray (`Color::Gray`).
  * The button layout order is uniformly **[Confirm/Execute] [Cancel]** (e.g., `[OK] [Cancel]`, `[Save] [Cancel]`, `[Delete] [Cancel]`).
* **Dynamic Height Adjustment and Collapse Prevention**:
  * Dynamically calculate the modal's vertical height (`h`) based on the number of contents (like folder lists) to ensure no empty space is left at the bottom.
  * To prevent layout breakage when there are `0` search results and no content, enforce a minimum height (minimum `10` including basic offsets) (`clamp(10, max_h)`) to secure at least 1 row of space for an empty list.
* **Enter Shortcut Misoperation Prevention**:
  * Disable the shortcut behavior where pressing Enter immediately submits the form while focus is in a text input box.
  * Process events such that the form is submitted **only when the user manually moves focus to the bottom button row and presses Enter while the Confirm/Execute button is active**.
  * Pressing Enter while the Cancel button is focused must act as a cancel action that closes the dialog.

## 4. Selected Row (Table row highlight)

| State | Background | Foreground/Attribute |
| :--- | :--- | :--- |
| Session Focused | `Color::Cyan` | `fg(Black) + BOLD` |
| Session Unfocused (Prompt Active) | `Rgb(55,55,55)` | `soft_dim_style + REVERSED` |
| Other | `DIM` | `fg(Black) + BOLD` |

- When representing an inactive panel, verify the borders, titles, headers, standard rows, and selected row **as a complete set**. Since the selected row highlight overwrites at the end, check `row_highlight_style` first. (Background: [Panel Focus Style](./panel-focus-style.md))

### 4.1 List Items and Footer Layout Rules

* **Item Name Right Margin**:
  * For items that might have long text, such as folder/session lists, add a 1-character margin (` `) to the right of the name to enhance readability.
  * The selected inverted bar (background highlight) must maintain its default state of fully occupying the original entire area (Inner width).
* **Footer Information Layout**:
  * Metrics/status information (e.g., `xx matching folders`, errors) is placed on the bottom **left**.
  * User operation guides and shortcut explanations are placed on the bottom **right** and right-aligned (`Alignment::Right`).

## 5. Implementation Locations Summary

| Target | Function |
| :--- | :--- |
| Color constants / agent colors | Top of `render.rs`, `agent_tag` |
| Shared non-highlight style | `soft_dim_style` |
| Panel container (focus branching) | `titled_block_nav` |
| Dialog container | `modal_block` |
| General notification dialog (reused) | `draw_message_modal` / `App::show_message` |
| Session table/selected row | `draw_table` |
| Prompt header meta information | `draw_preview` |

## 6. Checklist (When changing styles)

- [ ] Are new colors defined as constants instead of being inlined?
- [ ] Are highlights using `ACCENT + BOLD`, non-highlights using `soft_dim_style`, and borders following the `Thick/Plain` rule?
- [ ] For bright blue highlights like the '7' in the logo, is the true color RGB (`USAGE_HIGH`) used instead of `Color::LightBlue`?
- [ ] When switching focus back and forth (`Tab`) multiple times, do the tones of the two panels change together?
- [ ] Do both the search bar and dialogs maintain `Thick` highlighted borders?
- [ ] Does it pass `cargo clippy` without warnings?
