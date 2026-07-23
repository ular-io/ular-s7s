//! Session Detail feature: the two-column detail screen (left questions list,
//! right per-turn work and final answer).
//!
//! Extracted from `ui::mod` and `ui::render` per the refactoring plan (R8a),
//! following the same layout as `new_session` (R6) and `profile` (R7). The
//! feature owns its detail state and pure transitions (`state`), its `App` key
//! handling and open/close transitions (`input`), and its rendering (`render`).
//! The public detail types are re-exported from `ui` so existing
//! `crate::ui::DetailFocus` / `crate::ui::SessionDetailState` paths stay stable.

pub(crate) mod input;
pub(crate) mod render;
pub(crate) mod state;
#[cfg(test)]
mod tests;
