//! Profile creation/edit form state, focus model, and the pure transition
//! helpers driven by key handling.

use crate::model::Agent;
use crate::ui::TextInput;

/// Focused fields in the profile form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormFocus {
    /// Agent radio button (cycled via ←/→/Space).
    Agent,
    Name,
    Path,
    /// Bottom Save/Cancel buttons row.
    Buttons,
}

/// Profile creation/edit form state.
pub struct ProfileFormState {
    /// Target profile ID (None for a new profile). Also the single source of the
    /// lock: an existing profile keeps the agent type it was created with.
    pub editing_id: Option<String>,
    /// Radio selection index (referencing `Agent::all()`).
    pub agent_idx: usize,
    pub name: TextInput,
    pub path: TextInput,
    pub focus: FormFocus,
    /// Focused button in the button row: Save (true) or Cancel (false).
    pub save_focused: bool,
    /// Validation error message (rendered on the bottom line of the form).
    pub error: Option<String>,
}

impl ProfileFormState {
    /// Whether the identity fields are locked. Derived from `editing_id` rather
    /// than stored so the two can never disagree: creation picks the agent type,
    /// editing keeps it.
    pub fn locked(&self) -> bool {
        self.editing_id.is_some()
    }

    /// Focus rotation order. Bypasses the Agent field once locked, as the agent type is fixed.
    fn focus_order(&self) -> &'static [FormFocus] {
        if self.locked() {
            &[FormFocus::Name, FormFocus::Path, FormFocus::Buttons]
        } else {
            &[
                FormFocus::Agent,
                FormFocus::Name,
                FormFocus::Path,
                FormFocus::Buttons,
            ]
        }
    }

    pub(crate) fn focus_move(&mut self, delta: isize) {
        let order = self.focus_order();
        let cur = order.iter().position(|f| *f == self.focus).unwrap_or(0) as isize;
        let next = (cur + delta).rem_euclid(order.len() as isize) as usize;
        self.focus = order[next];
    }

    /// Whether the radio item can be picked while creating a profile. Antigravity
    /// does not support config directory overrides, so an extra agy profile would
    /// have no function; existing agy profiles stay editable but keep their type.
    pub fn agent_enabled(&self, idx: usize) -> bool {
        Agent::all()[idx] != Agent::Antigravity
    }

    pub(crate) fn cycle_agent(&mut self, delta: isize) {
        if self.locked() {
            return;
        }
        let n = Agent::all().len() as isize;
        // Skip disabled items (Antigravity).
        let mut idx = self.agent_idx as isize;
        for _ in 0..n {
            idx = (idx + delta).rem_euclid(n);
            if self.agent_enabled(idx as usize) {
                self.agent_idx = idx as usize;
                return;
            }
        }
    }

    /// Currently focused text input (returns None if focus is not on a text field).
    pub(crate) fn focused_input(&mut self) -> Option<&mut TextInput> {
        match self.focus {
            FormFocus::Name => Some(&mut self.name),
            FormFocus::Path => Some(&mut self.path),
            _ => None,
        }
    }
}
