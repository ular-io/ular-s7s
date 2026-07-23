//! Profile screen tests: the profile form (directory-create confirm, save
//! effect, Antigravity restrictions), shortcut-slot exhaustion, and delete
//! gating for built-in vs. normal profiles.

use crate::model::Agent;
use crate::ui::effect::AppEffect;
use crate::ui::test_support::*;
use crate::ui::*;
use crossterm::event::{KeyCode, KeyModifiers};
use std::path::PathBuf;

#[test]
fn profile_form_save_with_missing_dir_opens_create_confirm() {
    let mut app = empty_app();
    app.screen = Screen::Profile;
    app.open_profile_form(None);
    assert_eq!(app.mode, UiMode::ProfileForm);

    let missing = std::env::temp_dir().join("s7s-test-missing-config-dir-xyz");
    assert!(!missing.exists());
    let form = app.profile_form.as_mut().unwrap();
    form.name.value = "Team".to_string();
    form.path.value = missing.to_string_lossy().into_owned();

    app.confirm_profile_form();

    // Instead of saving, directory creation confirmation modal must be triggered (OK button focused).
    assert_eq!(app.mode, UiMode::ProfileDirConfirm);
    assert!(app.dir_create_ok_focused);
    assert!(app.profiles.profiles.is_empty());

    // Esc key returns back to form, preserving input values.
    app.on_key_profile_dir_confirm(key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, UiMode::ProfileForm);
    assert!(app.profile_form.is_some());
}

#[test]
fn confirm_profile_form_enqueues_profile_saved_effect() {
    let mut app = empty_app();
    app.screen = Screen::Profile;
    app.open_profile_form(None);

    // An existing directory skips the create-confirm modal and commits directly.
    let dir = std::env::temp_dir().join(format!(
        "s7s-test-existing-cfg-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let form = app.profile_form.as_mut().unwrap();
    form.name.value = "Team".to_string();
    form.path.value = dir.to_string_lossy().into_owned();

    app.confirm_profile_form();

    // The store is mutated in memory, but persistence + rescan are deferred to
    // the effect, so the form stays open until the boundary runs it.
    match app.pending_effect.as_ref() {
        Some(AppEffect::ProfileSaved {
            name,
            request_login,
            ..
        }) => {
            assert_eq!(name, "Team");
            assert!(!request_login);
        }
        other => panic!("expected ProfileSaved effect, got {other:?}"),
    }
    assert!(app.profiles.profiles.iter().any(|p| p.name == "Team"));
    assert!(app.profile_form.is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn space_on_unnumbered_profile_shows_error_when_all_slots_are_full() {
    let mut app = app_with_profiles();
    let template = app.profiles.profiles[1].clone();
    for slot in 3..=5 {
        let mut profile = template.clone();
        profile.id = format!("profile-{slot}");
        profile.name = format!("Profile {slot}");
        profile.path = PathBuf::from(format!("/tmp/profile-{slot}"));
        profile.shortcut = Some(slot);
        app.profiles.profiles.push(profile);
    }
    let mut unnumbered = template;
    unnumbered.id = "profile-6".to_string();
    unnumbered.name = "Profile 6".to_string();
    unnumbered.path = PathBuf::from("/tmp/profile-6");
    unnumbered.active = false;
    unnumbered.shortcut = None;
    app.profiles.profiles.push(unnumbered);
    app.screen = Screen::Profile;
    app.profile_selected = 5;

    app.on_key_profile_table(key(KeyCode::Char(' '), KeyModifiers::NONE));

    assert_eq!(app.mode, UiMode::Message);
    assert!(app
        .message
        .as_ref()
        .is_some_and(|message| message.kind == MessageKind::Error
            && message
                .lines
                .iter()
                .any(|line| line.contains("already assigned"))));
    assert_eq!(app.profiles.numbered_profiles().len(), 5);
    assert!(!app.profiles.profiles[5].active);
}

#[test]
fn builtin_profile_delete_blocked_and_normal_confirmed() {
    let mut app = app_with_profiles();
    app.screen = Screen::Profile;

    // Deletion of built-in profiles must be blocked with an alert dialog.
    app.profile_selected = 0;
    app.on_key_profile_table(key(KeyCode::Char('d'), KeyModifiers::CONTROL));
    assert_eq!(app.mode, UiMode::Message);
    app.on_key_message(key(KeyCode::Enter, KeyModifiers::NONE));

    // Deletion of normal profiles opens the deletion confirmation modal.
    app.profile_selected = 1;
    app.on_key_profile_table(key(KeyCode::Char('d'), KeyModifiers::CONTROL));
    assert_eq!(app.mode, UiMode::ProfileDeleteConfirm);
    // Esc cancels (skip saving validation verification tests).
    app.on_key_profile_delete_confirm(key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.profiles.profiles.len(), 2);
}

#[test]
fn profile_form_add_blocks_antigravity_agent() {
    let mut app = empty_app();
    app.open_profile_form(None);
    let form = app.profile_form.as_mut().unwrap();
    assert!(!form.agy_allowed);
    // Agent enum order: [Claude, Codex, Antigravity] - cycling bypasses Antigravity option.
    form.cycle_agent(1);
    assert_eq!(Agent::all()[form.agent_idx], Agent::Codex);
    form.cycle_agent(1);
    assert_eq!(Agent::all()[form.agent_idx], Agent::Claude);
    form.cycle_agent(-1);
    assert_eq!(Agent::all()[form.agent_idx], Agent::Codex);
}

#[test]
fn profile_form_save_rejects_new_antigravity_profile() {
    let mut app = empty_app();
    app.open_profile_form(None);
    let form = app.profile_form.as_mut().unwrap();
    // Force-selects Antigravity bypassing radio button restrictions (defensive validation test).
    form.agent_idx = 2;
    form.name.value = "Agy2".to_string();
    form.path.value = "/tmp".to_string();
    app.confirm_profile_form();
    let form = app.profile_form.as_ref().expect("form stays open");
    assert!(form.error.as_deref().unwrap().contains("Antigravity"));
    assert!(app.profiles.profiles.is_empty());
}
