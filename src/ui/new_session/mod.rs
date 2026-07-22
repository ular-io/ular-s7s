//! New Session feature: the profile/model/folder/context creation dialog.
//!
//! Extracted from `ui::mod` and `ui::render` per the refactoring plan (R6). The
//! feature owns its state and pure transitions (`state`), its `App` key handling
//! and launch logic (`input`), and its rendering (`render`). The public dialog
//! types are re-exported from `ui` so existing `crate::ui::NewSession*` paths
//! stay stable.

pub(crate) mod input;
pub(crate) mod render;
pub(crate) mod state;
