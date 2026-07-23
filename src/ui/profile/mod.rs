//! Profile feature: the profile table, add/edit form, deletion confirmation, and
//! config-directory creation confirmation.
//!
//! Extracted from `ui::mod` and `ui::render` per the refactoring plan (R7),
//! following the same layout as `new_session` (R6). The feature owns its form
//! state and pure transitions (`state`), its `App` key handling and profile
//! persistence/login-request logic (`input`), and its rendering (`render`). The
//! public form types are re-exported from `ui` so existing `crate::ui::FormFocus`
//! / `crate::ui::ProfileFormState` paths stay stable.

pub(crate) mod input;
pub(crate) mod render;
pub(crate) mod state;
#[cfg(test)]
mod tests;
