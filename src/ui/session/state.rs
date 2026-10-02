//! Session search screen focus model. The rest of the screen state lives in
//! `App` fields (selected/filtered/filter/preview_scroll/...); the §8.1 `App`
//! split is deferred, so this module owns only the `Focus` enum.

/// Focused panel in the main search screen, left to right: the workspace pane
/// (drawn only while it has focus), the table, and the preview. Moved via ←/→.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Workspaces,
    Table,
    Preview,
}
