//! Screen switching, profile/filter shortcuts, resume requests, and quit arming.

use super::{agent_status, App, Focus, MessageKind, Screen, UiMode};
use crate::filter::Filter;

impl App {
    /// Header number keys (`1..5`): Activates a single profile filter at the specified numbered index.
    pub(crate) fn set_single_profile(&mut self, idx: usize) {
        let Some((id, name)) = self
            .profiles
            .numbered_profiles()
            .get(idx)
            .map(|p| (p.id.clone(), p.name.clone()))
        else {
            return;
        };
        self.filter.profile_ids.clear();
        self.filter.profile_ids.insert(id);
        self.recompute();
        self.status_msg = Some(format!("Profile filter: {}", name));
    }

    /// Resolves profile ID to display name (used in filter descriptions).
    pub fn profile_name(&self, id: &str) -> Option<String> {
        self.profiles.find(id).map(|p| p.name.clone())
    }

    // ---- Screen Switching / Profile Screen ----

    /// Switched screen to target and reverts to table navigation mode.
    pub(crate) fn switch_screen(&mut self, screen: Screen) {
        self.screen = screen;
        self.mode = UiMode::Table;
        self.status_msg = None;
        if screen != Screen::Detail {
            self.detail = None;
        }
        // The workspace pane closes when its screen is left, so returning to
        // the Session screen lands on the session list unless told otherwise.
        if screen != Screen::Session && self.focus == Focus::Workspaces {
            self.focus = Focus::Table;
        }
        if screen == Screen::Profile {
            self.profile_selected = self
                .profile_selected
                .min(self.profiles.profiles.len().saturating_sub(1));
        }
    }

    pub(crate) fn clear_all_filters(&mut self) {
        self.filter = Filter::default();
        self.recompute();
        self.status_msg = Some("Filters cleared".to_string());
    }

    /// Requests resuming the targeted session. Triggers an alert instead of handover
    /// if the project directory no longer exists (pre-flight check).
    pub(crate) fn request_resume(&mut self, idx: usize) {
        let cwd = self.sessions[idx].cwd.clone();
        // Omit check if cwd is empty, as it runs without directory changes.
        if !cwd.as_os_str().is_empty() && !cwd.is_dir() {
            self.show_message(
                " Cannot Resume ",
                vec![
                    "The project folder no longer exists:".to_string(),
                    cwd.to_string_lossy().into_owned(),
                    String::new(),
                    "This session cannot be resumed.".to_string(),
                ],
                MessageKind::Error,
            );
            return;
        }
        if self.divert_live_session(idx, agent_status::LiveAction::Resume) {
            return;
        }
        self.resume_request = Some(idx);
    }

    pub(crate) fn arm_quit(&mut self) {
        if let Some(until) = self.quit_grace_until {
            let now = std::time::Instant::now();
            if now < until {
                // Ignore exits during grace period to defend against trailing spam; extends the grace window
                // slightly to ensure keystrokes only register after user halts spamming.
                const QUIT_GRACE_REPEAT: std::time::Duration =
                    std::time::Duration::from_millis(400);
                self.quit_grace_until = Some(until.max(now + QUIT_GRACE_REPEAT));
                return;
            }
            self.quit_grace_until = None;
        }
        if self.quit_armed {
            self.should_quit = true;
        } else {
            self.quit_armed = true;
            self.status_msg = Some("Press q or ctrl+c again to quit".to_string());
        }
    }

    /// Begins the exit key grace period upon returning from agent handover (called in main loop).
    pub fn begin_quit_grace(&mut self) {
        const QUIT_GRACE: std::time::Duration = std::time::Duration::from_millis(1200);
        self.quit_grace_until = Some(std::time::Instant::now() + QUIT_GRACE);
    }
}
