//! Workspaces: the Session screen's workspace pane (`←` from the session list,
//! `Focus::Workspaces`) and the edit dialog it opens with Enter or `+`, which
//! edits a draft of one workspace's name, include/exclude words, and folders
//! and saves it only with its Save button.
//!
//! The pane cursor is the open workspace (`WorkspaceStore::active`), so moving
//! it changes what the session list shows; the palette's
//! `Open Workspace <name>` / `Close Workspace` set the same state. The stored
//! model and its matching rule live in `crate::workspaces`.

pub(crate) mod input;
pub(crate) mod render;
pub(crate) mod state;
#[cfg(test)]
mod tests;

pub use state::WorkspaceScreenState;
