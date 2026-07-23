//! Overlay features: the modal dialogs and transient screens layered over the
//! main Session/Profile/Detail views.
//!
//! Extracted from `ui::mod` and `ui::render` per the refactoring plan (R9),
//! following the same discipline as the earlier feature modules (`new_session`
//! R6, `profile` R7, `detail` R8a, `session` R8b). Ownership is already clear
//! per overlay, so each overlay is a single file combining its state, `App` key
//! handling, and rendering rather than the four-file feature layout (§7 "stop
//! splitting when ownership is already clear").
//!
//! - `filters` — agent and folder multi-select filter modals (`ModalState`).
//! - `confirm` — session rename and delete confirmation dialogs.
//! - `message` — the reusable alert dialog (`show_message`).
//! - `help` — the `?` keyboard-shortcuts screen.
//! - `theme` — the theme selection dialog with live preview.
//!
//! The overlay state types are re-exported from `ui` so the existing
//! `crate::ui::{ModalState, RenameFocus, RenameModalState, MessageKind,
//! MessageDialog, ThemeSelectState}` paths stay stable. The Quick Command
//! palette keeps its own `ui::quick` module (its render moved there in R9 to
//! complete its state/input/render boundary — §9.5); the New Session project-
//! directory confirmation stays with the New Session flow in `ui::render`. The
//! session-deletion filesystem work invoked by the delete dialog stays in
//! `ui::mod` as cross-feature `App` coordination.

pub(crate) mod confirm;
pub(crate) mod filters;
pub(crate) mod help;
pub(crate) mod message;
#[cfg(test)]
mod tests;
pub(crate) mod theme;
