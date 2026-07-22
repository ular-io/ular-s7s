//! Shared, feature-agnostic UI primitives reused across screens and dialogs:
//! Unicode-safe text input editing, modal framing/buttons/backdrop, width-aware
//! text truncation/wrapping, and the persistent vertical scrollbar.
//!
//! These were extracted from `ui::mod` and `ui::render` per the refactoring plan
//! (R5). They must stay free of feature-specific state so any dialog can reuse
//! them; feature widgets belong in their own feature module, not here.

pub(crate) mod input;
pub(crate) mod modal;
pub(crate) mod scrollbar;
pub(crate) mod text;
