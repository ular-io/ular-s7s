//! Quick Command window (`:` palette / `!` terminal): command registry, alias searching,
//! terminal command history, and execution history.
//!
//! Palette mode (`:`):
//! - Matching: Splits input by whitespace. Matches if all words are present as substrings
//!   (case-insensitive) in the label, aliases, or shortcut keys (multi-word AND).
//! - Sorting: Enabled commands on the active screen appear at the top, disabled below.
//!   Within each group, sorted by most recently used first, falling back to registry order.
//! - History: Saved to `~/.config/s7s/quick_history.json` to persist execution keys
//!   across application restarts.
//!
//! Terminal mode (`!`):
//! - Runs a user shell command in the selected session's folder (main loop handover).
//! - The list below the input shows terminal command history (most recent first) filtered
//!   by the typed text; moving the selection recalls the command into the editable input,
//!   and Enter always runs the input content.
//! - History: Saved to `~/.config/s7s/terminal_history.json` as raw command strings.
//!
//! Mode switching: pressing `:`/`!` while the input is EMPTY switches window mode
//! (both characters are ordinary typeable characters otherwise).
//!
//! Extracted from `ui::quick` (a single file) into a feature module, following the
//! layout of `new_session`, `profile`, `detail`, and `session`. The command
//! registry data table lives in `registry`, the window/query state and history
//! I/O in `state`, the `App` key handling and command execution in `input`, and
//! the modal rendering in `render`.

pub(crate) mod input;
pub(crate) mod registry;
pub(crate) mod render;
pub(crate) mod state;
#[cfg(test)]
mod tests;

pub use state::{load_history, load_terminal_history, QuickMode, QuickState};

pub(crate) use render::draw_quick_command;

/// Maximum rows visible in the viewport. Exceeding this triggers cursor-following scroll.
const VIEWPORT: usize = 10;
