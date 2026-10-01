//! Workspaces screen (between Profile and Session): a workspace list, a Detail
//! pane editing the selected workspace's name, include/exclude words, and
//! folders, and the session table scoped to it.
//!
//! The list cursor is the open workspace (`WorkspaceStore::active`), so moving
//! it changes what the Session screen lists too; the palette's
//! `Open Workspace <name>` / `Close Workspace` set the same state. The stored
//! model and its matching rule live in `crate::workspaces`.

pub(crate) mod input;
pub(crate) mod render;
pub(crate) mod state;
#[cfg(test)]
mod tests;

pub use state::{WorkspacePane, WorkspaceScreenState};
