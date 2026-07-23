//! Session Search feature: the main screen — left session table with the
//! composite filter, right per-turn preview panel, and the `/` keyword search
//! prompt.
//!
//! Extracted from `ui::mod` and `ui::render` per the refactoring plan (R8b),
//! following the same layout as `new_session` (R6), `profile` (R7), and
//! `detail` (R8a). The feature owns its focus model (`state`), its `App` key
//! handling for the table and keyword search (`input`), and its rendering
//! (`render`). The `Focus` type is re-exported from `ui` so the existing
//! `crate::ui::Focus` path stays stable.
//!
//! The Session screen has no dedicated state struct: its state lives directly in
//! `App` fields (`selected`, `filtered`, `filter`, `preview_scroll`, `focus`,
//! ...). The §8.1 `App` split is out of scope for R8b, so `state` owns only the
//! `Focus` enum; the `impl App` handlers keep operating on those `App` fields.
//! Cross-feature filter coordination (`clear_all_filters`, `set_single_profile`,
//! `recompute`, screen switching) and the filter/rename/delete overlays remain in
//! `ui::mod` and `ui::render`.

pub(crate) mod input;
pub(crate) mod render;
pub(crate) mod state;
#[cfg(test)]
mod tests;
