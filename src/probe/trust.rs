//! Folder-trust dialog recognition for PTY probes: which dialogs block a CLI's boot,
//! and which key the probe may send to them without declining or blindly confirming.

use std::time::Duration;

/// Screen text of the folder-trust dialogs that block a CLI before it reaches the
/// prompt. Version-specific wording (claude 2.1.220 / 2.1.318, agy 1.1.8) — recheck on upgrade.
pub(super) const TRUST_PROMPT_MARKERS: &[&str] = &[
    "Do you trust the files in this folder",
    "Is this a project you created or one you trust",
    "Do you trust the contents of this project",
];

/// Row label of the approval choice in those dialogs (identical across claude and agy).
const TRUST_APPROVE_ROW: &str = "Yes, I trust this folder";

/// How long the dialog may stay on screen after Enter was sent before the probe gives
/// up: if confirming did not dismiss it, the dialog is not the one we recognized.
pub(super) const TRUST_CONFIRM_GRACE: Duration = Duration::from_secs(8);

/// Most caret moves the probe makes to reach the approval row. claude 2.1.318 puts
/// `No, exit` first and selected with the approval row right below it; a larger gap
/// means a menu we do not recognize, so it fails as untrusted instead.
pub(super) const TRUST_MAX_MOVES: usize = 3;

/// How long the probe waits after a caret move for the dialog to redraw before it
/// reads the selection again, so one unrendered move is never followed by another.
pub(super) const TRUST_MOVE_SETTLE: Duration = Duration::from_millis(600);

/// What the probe may do with the trust dialog currently on screen.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum TrustStep {
    /// The caret sits on the approval row: Enter grants trust.
    Confirm,
    /// The caret sits on a row above the approval row, at most `TRUST_MAX_MOVES`
    /// rows up: Down moves it toward approval.
    MoveDown,
    /// Anything else (approval row missing, caret below it or too far up): neither
    /// Enter nor a move is safe.
    Refuse,
}

fn is_caret_row(row: &str) -> bool {
    row.starts_with('❯') || row.starts_with('>')
}

/// Decides the next trust-dialog key from the *highlighted* row. Enter is only ever
/// sent with the caret on the approval row, so a reordered menu can cost a move but
/// never decline or confirm the wrong choice. claude 2.1.220 and agy 1.1.8 select
/// approval first; claude 2.1.318 selects `No, exit` with approval one row below.
pub(super) fn trust_step(screen: &str) -> TrustStep {
    let rows: Vec<&str> = screen.lines().map(str::trim_start).collect();
    let Some(approve) = rows.iter().position(|r| r.contains(TRUST_APPROVE_ROW)) else {
        return TrustStep::Refuse;
    };
    if is_caret_row(rows[approve]) {
        return TrustStep::Confirm;
    }
    let caret_above = rows[..approve].iter().rposition(|r| is_caret_row(r));
    match caret_above {
        Some(caret) if approve - caret <= TRUST_MAX_MOVES => TrustStep::MoveDown,
        _ => TrustStep::Refuse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// agy 1.1.8 dialog, approval row selected.
    const AGY_TRUST: &str = "\n Accessing workspace:\n\n /Users/x/.config/s7s/probe\n\n \
         Do you trust the contents of this project?\n\n > Yes, I trust this folder\n   No, exit\n";

    /// claude 2.1.220 dialog, approval row selected (numbered rows).
    const CLAUDE_TRUST: &str = "\n Quick safety check: Is this a project you created or one \
         you trust?\n\n ❯ 1. Yes, I trust this folder\n   2. No, continue without these \
         permissions\n";

    /// claude 2.1.318 dialog: the decline row comes first and is selected, the
    /// approval row sits right below it (fresh install, no trusted ancestor folder).
    const CLAUDE_TRUST_DECLINE_FIRST: &str =
        "\n Accessing workspace:\n\n /Users/x/.config/s7s/probe\n\n \
         Quick safety check: Is this a project you created or one you trust? (Like your own \
         code, a well-known open source project, or work from your team).\n\n \
         ❯ No, exit\n   Yes, I trust this folder\n\n Enter to confirm · Esc to cancel\n";

    #[test]
    fn approval_row_is_recognized_for_every_dialog_layout() {
        assert_eq!(trust_step(AGY_TRUST), TrustStep::Confirm);
        assert_eq!(trust_step(CLAUDE_TRUST), TrustStep::Confirm);
        for screen in [AGY_TRUST, CLAUDE_TRUST, CLAUDE_TRUST_DECLINE_FIRST] {
            assert!(TRUST_PROMPT_MARKERS.iter().any(|m| screen.contains(m)));
        }
    }

    #[test]
    fn caret_above_the_approval_row_moves_down_then_confirms() {
        assert_eq!(trust_step(CLAUDE_TRUST_DECLINE_FIRST), TrustStep::MoveDown);
        // The screen claude 2.1.318 redraws after one Down.
        let moved = CLAUDE_TRUST_DECLINE_FIRST
            .replace("❯ No, exit", "  No, exit")
            .replace("   Yes, I trust this folder", " ❯ Yes, I trust this folder");
        assert_eq!(trust_step(&moved), TrustStep::Confirm);
    }

    #[test]
    fn confirm_and_moves_are_withheld_unless_approval_is_reachable_by_moving_down() {
        // Decline row selected *below* approval: Enter would decline and Down moves
        // away, so the probe must report the folder as untrusted.
        let decline_below = AGY_TRUST
            .replace("> Yes, I trust this folder", "  Yes, I trust this folder")
            .replace("   No, exit", " > No, exit");
        assert_eq!(trust_step(&decline_below), TrustStep::Refuse);
        // Caret further above approval than the probe is willing to move.
        let far = "❯ a\n  b\n  c\n  d\n  Yes, I trust this folder\n";
        assert_eq!(trust_step(far), TrustStep::Refuse);
        // Approval text alone (no caret anywhere) is not a selection either.
        assert_eq!(
            trust_step("  Yes, I trust this folder\n"),
            TrustStep::Refuse
        );
        // A dialog without the approval row offers nothing to confirm.
        assert_eq!(trust_step("❯ No, exit\n"), TrustStep::Refuse);
        // An ordinary boot screen carries neither the prompt nor a selection.
        assert_eq!(trust_step("? for shortcuts\n"), TrustStep::Refuse);
    }
}
