use super::*;

/// Recorded terminal operation, used to assert ordering and balance.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Enable(TerminalMode),
    Disable(TerminalMode),
    ShowCursor,
}

/// In-memory [`TerminalOps`] with per-mode failure injection. Touches no real
/// terminal, so these tests are safe under the default threaded test runner.
struct MockOps {
    calls: Vec<Call>,
    fail_enable: Vec<TerminalMode>,
    fail_disable: Vec<TerminalMode>,
    fail_show_cursor: bool,
    supports_enhancement: bool,
}

impl MockOps {
    fn new() -> Self {
        MockOps {
            calls: Vec::new(),
            fail_enable: Vec::new(),
            fail_disable: Vec::new(),
            fail_show_cursor: false,
            supports_enhancement: true,
        }
    }
}

fn failure(mode: TerminalMode) -> io::Error {
    io::Error::other(format!("injected failure: {}", mode.name()))
}

impl TerminalOps for MockOps {
    fn enable(&mut self, mode: TerminalMode) -> io::Result<()> {
        self.calls.push(Call::Enable(mode));
        if self.fail_enable.contains(&mode) {
            return Err(failure(mode));
        }
        Ok(())
    }

    fn disable(&mut self, mode: TerminalMode) -> io::Result<()> {
        self.calls.push(Call::Disable(mode));
        if self.fail_disable.contains(&mode) {
            return Err(failure(mode));
        }
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.calls.push(Call::ShowCursor);
        if self.fail_show_cursor {
            return Err(io::Error::other("injected failure: show cursor"));
        }
        Ok(())
    }

    fn supports_keyboard_enhancement(&mut self) -> bool {
        self.supports_enhancement
    }
}

fn enabled(modes: &TerminalModes<MockOps>) -> Vec<TerminalMode> {
    [
        (TerminalMode::Raw, modes.raw),
        (TerminalMode::AlternateScreen, modes.alternate),
        (TerminalMode::BracketedPaste, modes.bracketed_paste),
        (TerminalMode::KeyboardEnhancement, modes.keyboard_enhanced),
    ]
    .into_iter()
    .filter(|(_, on)| *on)
    .map(|(mode, _)| mode)
    .collect()
}

#[test]
fn enter_turns_on_raw_first_then_screen_paste_and_keyboard() {
    let mut modes = TerminalModes::new(MockOps::new());
    modes.enter().expect("enter");
    assert_eq!(
        modes.ops.calls,
        vec![
            // Raw mode must precede the keyboard-enhancement query.
            Call::Enable(TerminalMode::Raw),
            Call::Enable(TerminalMode::AlternateScreen),
            Call::Enable(TerminalMode::BracketedPaste),
            Call::Enable(TerminalMode::KeyboardEnhancement),
        ]
    );
    assert_eq!(
        enabled(&modes),
        vec![
            TerminalMode::Raw,
            TerminalMode::AlternateScreen,
            TerminalMode::BracketedPaste,
            TerminalMode::KeyboardEnhancement,
        ]
    );
}

#[test]
fn unsupported_keyboard_enhancement_is_skipped_not_failed() {
    let mut ops = MockOps::new();
    ops.supports_enhancement = false;
    let mut modes = TerminalModes::new(ops);
    modes.enter().expect("enter");
    assert!(!modes.keyboard_enhanced);
    assert!(!modes
        .ops
        .calls
        .contains(&Call::Enable(TerminalMode::KeyboardEnhancement)));
}

#[test]
fn failed_keyboard_enhancement_leaves_the_session_usable() {
    let mut ops = MockOps::new();
    ops.fail_enable = vec![TerminalMode::KeyboardEnhancement];
    let mut modes = TerminalModes::new(ops);
    modes
        .enter()
        .expect("enter must succeed without enhancement");
    assert!(!modes.keyboard_enhanced);
    assert!(modes.raw && modes.alternate && modes.bracketed_paste);
}

#[test]
fn partial_setup_restores_every_mode_already_enabled() {
    let mut ops = MockOps::new();
    ops.fail_enable = vec![TerminalMode::BracketedPaste];
    let mut modes = TerminalModes::new(ops);
    let err = modes.enter().expect_err("setup must fail");
    assert!(err.to_string().contains("bracketed paste"));
    assert!(enabled(&modes).is_empty(), "no mode may stay on");
    assert_eq!(
        modes.ops.calls,
        vec![
            Call::Enable(TerminalMode::Raw),
            Call::Enable(TerminalMode::AlternateScreen),
            Call::Enable(TerminalMode::BracketedPaste),
            // Cleanup of what actually took effect:
            Call::Disable(TerminalMode::AlternateScreen),
            Call::ShowCursor,
            Call::Disable(TerminalMode::Raw),
        ]
    );
}

#[test]
fn cleanup_order_pops_keyboard_and_paste_before_leaving_the_screen() {
    let mut modes = TerminalModes::new(MockOps::new());
    modes.enter().expect("enter");
    modes.ops.calls.clear();
    modes.restore().expect("restore");
    assert_eq!(
        modes.ops.calls,
        vec![
            Call::Disable(TerminalMode::KeyboardEnhancement),
            Call::Disable(TerminalMode::BracketedPaste),
            Call::Disable(TerminalMode::AlternateScreen),
            // Cursor is shown after leaving the alternate screen.
            Call::ShowCursor,
            Call::Disable(TerminalMode::Raw),
        ]
    );
    assert!(enabled(&modes).is_empty());
}

#[test]
fn one_cleanup_failure_does_not_skip_later_steps() {
    let mut ops = MockOps::new();
    ops.fail_disable = vec![TerminalMode::AlternateScreen];
    let mut modes = TerminalModes::new(ops);
    modes.enter().expect("enter");
    modes.ops.calls.clear();

    let err = modes.restore().expect_err("cleanup must report failure");
    assert!(err.to_string().contains("alternate screen"));
    // Raw mode and the cursor are still handled after the failing step.
    assert_eq!(
        modes.ops.calls,
        vec![
            Call::Disable(TerminalMode::KeyboardEnhancement),
            Call::Disable(TerminalMode::BracketedPaste),
            Call::Disable(TerminalMode::AlternateScreen),
            Call::ShowCursor,
            Call::Disable(TerminalMode::Raw),
        ]
    );
    // Only the failed step keeps its flag, so a retry redoes just that one.
    assert_eq!(enabled(&modes), vec![TerminalMode::AlternateScreen]);
}

#[test]
fn cleanup_returns_the_first_error_in_cleanup_order() {
    let mut ops = MockOps::new();
    ops.fail_disable = vec![TerminalMode::BracketedPaste, TerminalMode::Raw];
    ops.fail_show_cursor = true;
    let mut modes = TerminalModes::new(ops);
    modes.enter().expect("enter");

    let err = modes.restore().expect_err("cleanup must report failure");
    assert!(
        err.to_string().contains("bracketed paste"),
        "expected the first failure, got: {err}"
    );
    assert_eq!(
        enabled(&modes),
        vec![TerminalMode::Raw, TerminalMode::BracketedPaste]
    );
}

#[test]
fn a_retry_redoes_only_what_stayed_active() {
    let mut ops = MockOps::new();
    ops.fail_disable = vec![TerminalMode::AlternateScreen];
    let mut modes = TerminalModes::new(ops);
    modes.enter().expect("enter");
    modes.restore().expect_err("first cleanup fails");

    // The retry that `Drop` performs, with the terminal now cooperating.
    modes.ops.fail_disable.clear();
    modes.ops.calls.clear();
    modes.restore().expect("retry succeeds");
    assert_eq!(
        modes.ops.calls,
        vec![
            Call::Disable(TerminalMode::AlternateScreen),
            Call::ShowCursor
        ]
    );
    assert!(enabled(&modes).is_empty());
}

#[test]
fn panic_restore_undoes_exactly_the_modes_recorded_as_active() {
    // What the panic hook does: rebuild the flags from the global mask (the
    // panicking thread does not own the session) and clean up in order.
    let mask = TerminalMode::Raw.bit() | TerminalMode::AlternateScreen.bit();
    let mut modes = TerminalModes::from_active_mask(MockOps::new(), mask);
    assert_eq!(
        enabled(&modes),
        vec![TerminalMode::Raw, TerminalMode::AlternateScreen]
    );
    modes.restore().expect("restore");
    assert_eq!(
        modes.ops.calls,
        vec![
            Call::Disable(TerminalMode::AlternateScreen),
            Call::ShowCursor,
            Call::Disable(TerminalMode::Raw),
        ],
        "modes that were never enabled must not be touched"
    );
    assert!(enabled(&modes).is_empty());
}

#[test]
fn suspend_and_resume_are_balanced_and_idempotent() {
    let mut modes = TerminalModes::new(MockOps::new());
    modes.enter().expect("enter");
    modes.restore().expect("suspend");
    modes.enter().expect("resume");
    modes.restore().expect("final cleanup");

    let enables = modes
        .ops
        .calls
        .iter()
        .filter(|c| matches!(c, Call::Enable(_)))
        .count();
    let disables = modes
        .ops
        .calls
        .iter()
        .filter(|c| matches!(c, Call::Disable(_)))
        .count();
    assert_eq!(enables, 8, "4 modes enabled twice");
    assert_eq!(disables, enables, "every enable has a matching disable");

    // Re-entering an already-entered session issues no duplicate enables.
    modes.enter().expect("enter");
    modes.ops.calls.clear();
    modes.enter().expect("re-enter");
    assert!(modes.ops.calls.is_empty());
}
