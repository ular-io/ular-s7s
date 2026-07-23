//! Cross-feature `ui` tests: behaviors that span more than one screen, such as
//! global refresh reachable from every screen and left/right screen switching.

use super::effect::AppEffect;
use super::test_support::*;
use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

#[test]
fn ctrl_u_is_global_across_profile_and_detail_screens() {
    // Profile view: the effect prepares (status + scheduled scan); the scan then
    // refreshes the session list after the preparing frame renders.
    let mut app = empty_app();
    app.screen = Screen::Profile;
    app.on_key_profile_table(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(app.pending_effect, Some(AppEffect::RefreshAll));
    app.apply_effect();
    assert_eq!(
        app.status_msg.as_deref(),
        Some("updating sessions and usage…")
    );
    assert!(app.refresh_scan_scheduled());
    app.run_scheduled_refresh_scan();
    assert!(matches!(
        app.status_msg.as_deref(),
        Some(msg) if msg.starts_with("session update complete · ")
    ));
    app.finish_refresh_cycle();

    // Details view: returning to search view if target session vanishes (e.g. empty profile rescan) post-update.
    let mut app = app_with_session();
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Detail);
    app.on_key_detail(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(app.pending_effect, Some(AppEffect::RefreshAll));
    app.apply_effect();
    // The prepare step leaves the screen untouched; the scan does the rescan.
    assert_eq!(app.screen, Screen::Detail);
    app.run_scheduled_refresh_scan();
    assert!(matches!(
        app.status_msg.as_deref(),
        Some(msg) if msg.starts_with("session update complete · ")
    ));
    assert_eq!(app.screen, Screen::Session);
    assert!(app.detail.is_none());
}

#[test]
fn refresh_all_requests_merge_into_one_cycle_until_completion_render() {
    let mut app = empty_app();

    // First request: prepare and schedule exactly one scan.
    app.on_key_table(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
    app.apply_effect();
    assert!(app.refresh_scan_scheduled());

    // Repeat before the preparing frame renders: merged into the same cycle.
    app.on_key_table(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
    app.apply_effect();
    assert!(app.refresh_scan_scheduled());
    assert_eq!(
        app.status_msg.as_deref(),
        Some("updating sessions and usage…")
    );

    // The loop renders the preparing frame, then runs the single scan.
    app.run_scheduled_refresh_scan();
    assert!(!app.refresh_scan_scheduled());
    assert!(matches!(
        app.status_msg.as_deref(),
        Some(msg) if msg.starts_with("session update complete · ")
    ));

    // A Ctrl+U queued during the scan (drained before the completion render)
    // merges into the finished cycle: no second scan, status untouched.
    app.on_key_table(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
    app.apply_effect();
    assert!(!app.refresh_scan_scheduled());
    assert!(matches!(
        app.status_msg.as_deref(),
        Some(msg) if msg.starts_with("session update complete · ")
    ));

    // After the completion frame renders, a new Ctrl+U starts a fresh cycle.
    app.finish_refresh_cycle();
    app.on_key_table(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
    app.apply_effect();
    assert!(app.refresh_scan_scheduled());
}

#[test]
fn refresh_prepare_never_marks_usage_loading_without_a_spawned_probe() {
    // The prepare step must go through `start_usage_fetch` (phase flip and probe
    // spawn stay atomic). Under the test spawn guard no probe starts, so no
    // profile may be left stranded in `Loading` with no job to resolve it.
    let mut app = app_with_profiles();
    app.on_key_table(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
    app.apply_effect();
    assert!(!app.usage_in_flight());
    for p in &app.profiles.profiles {
        assert_ne!(
            app.usage.entry(&p.id).phase,
            crate::usage::UsagePhase::Loading
        );
    }
}

#[test]
fn left_right_switch_between_profile_and_session_screens() {
    let mut app = app_with_session();

    // Session list (with table focus) -> Left key -> Profile list screen.
    app.on_key_table(key(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Profile);

    // Profile view -> Right key -> Session list screen (independent of selected profile).
    app.on_key_profile_table(key(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Session);
    assert_eq!(app.focus, Focus::Table);
}
