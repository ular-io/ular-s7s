use super::claude::*;
use crate::probe::pty::CAPTURE_SEPARATOR;

/// Captured vt100 output of claude 2.1.220 `/model` screen from 2026-07-28.
/// Since 2.1.220 the long-context row is named `Opus (1M context)` instead of `Opus`.
const CLAUDE_MODEL_SCREEN: &str = "\
uu
 ▐▛███▜▌   Claude Code v2.1.220
▝▜█████▛▘  Fable 5 · Claude Team
  ▘▘ ▝▝    ~/projects/my-app

   Select model
   Switch between Claude models. Your pick becomes the default for new sessions. For other/previous model names, specify with --model.

     1. Default (recommended)  Sonnet 5 · Efficient for routine tasks
     2. Sonnet                 Sonnet 5 · Efficient for routine tasks
   ❯ 3. Fable ✔                Fable 5 · Most capable for your hardest and longest-running tasks
     4. Opus (1M context)      Opus 5 with 1M context · Best for everyday, complex tasks
     5. Haiku                  Haiku 4.5 · Fastest for quick answers

   ● High effort (default) ←/→ to adjust

   Enter to set as default · s to use this session only · Esc to cancel
";

#[test]
fn parse_claude_model_screen_extracts_aliases_and_default() {
    let (models, default_model) = parse_claude_model_screen(CLAUDE_MODEL_SCREEN).unwrap();
    let values: Vec<&str> = models.iter().map(|m| m.value.as_str()).collect();
    // Excludes "Default (recommended)" row; the long-context row becomes the `[1m]` alias
    // rather than its lowercased display name.
    assert_eq!(values, ["sonnet", "fable", "opus[1m]", "haiku"]);
    assert_eq!(default_model.as_deref(), Some("fable"));
    let fable = models.iter().find(|m| m.value == "fable").unwrap();
    assert_eq!(fable.label, "Fable");
    assert!(fable.note.starts_with("Fable 5"));
    // The label keeps the screen wording even though the value is normalized.
    let opus = models.iter().find(|m| m.value == "opus[1m]").unwrap();
    assert_eq!(opus.label, "Opus (1M context)");
}

#[test]
fn claude_model_value_handles_pinned_versions_and_legacy_aliases() {
    assert_eq!(claude_model_value("Opus").as_deref(), Some("opus"));
    assert_eq!(
        claude_model_value("Opus (1M context)").as_deref(),
        Some("opus[1m]")
    );
    assert_eq!(
        claude_model_value("claude-fable-5").as_deref(),
        Some("claude-fable-5")
    );
    assert_eq!(
        claude_model_value("Opus 4.8").as_deref(),
        Some("claude-opus-4-8")
    );
    // Unknown versions and qualifiers are not guessed.
    assert_eq!(claude_model_value("Opus 5.6"), None);
    assert_eq!(claude_model_value("Opus (Preview)"), None);
    assert_eq!(claude_model_value("(1M context)"), None);
}

#[test]
fn parse_claude_2_1_283_versioned_picker_rows() {
    let screen = "\
  Select model

    1.  Default (recommended)  Opus 5.5 · Best for everyday, complex tasks
  ❯ 2.  Opus 5.5 ✔             Most capable for ambitious work
    3.  Fable 5.1              For your toughest challenges
    4.  Sonnet 5               Most efficient for everyday tasks
    5.  Haiku 4.5              Fastest for quick answers
    6.  Opus 5                 Best for everyday, complex tasks
    7.  Fable 5                Most capable for your hardest and longest-running tasks
    8.  Opus 4.8               Best for everyday, complex tasks
    9.  Opus 4.7               Best for everyday, complex tasks
   10.  Opus 4.6               Best for everyday, complex tasks
    11. Sonnet 4.6             Most efficient for everyday tasks
";
    let (models, default_model) = parse_claude_model_screen(screen).unwrap();
    let values: Vec<&str> = models.iter().map(|m| m.value.as_str()).collect();
    assert_eq!(
        values,
        [
            "claude-opus-5-5",
            "claude-fable-5-1",
            "claude-sonnet-5",
            "claude-haiku-4-5-20251001",
            "claude-opus-5",
            "claude-fable-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-sonnet-4-6",
        ]
    );
    assert_eq!(default_model.as_deref(), Some("claude-opus-5-5"));
}

#[test]
fn parse_claude_2_1_284_picker_with_rows_below_the_fold() {
    // 2.1.284 added `Sonnet 5.5`; twelve rows need two captures to see them all.
    let first = "\
  Select model

    1.  Default (recommended)  Opus 5.5 · Best for everyday, complex tasks
  ❯ 2.  Opus 5.5 ✔             For complex work and everyday tasks
    3.  Fable 5.1              For your toughest challenges
    4.  Sonnet 5.5             Most efficient for simpler tasks
    5.  Haiku 4.5              Fastest for quick answers
    6.  Sonnet 5               Efficient for routine tasks
    7.  Opus 5                 Best for everyday, complex tasks
    8.  Fable 5                Most capable for your hardest and longest-running tasks
    9.  Opus 4.8               Best for everyday, complex tasks
  ↓ 10. Opus 4.7               Best for everyday, complex tasks
     … +2 models
";
    let last = "\
  Select model

  ↑ 3.  Fable 5.1              For your toughest challenges
    4.  Sonnet 5.5             Most efficient for simpler tasks
    5.  Haiku 4.5              Fastest for quick answers
    6.  Sonnet 5               Efficient for routine tasks
    7.  Opus 5                 Best for everyday, complex tasks
    8.  Fable 5                Most capable for your hardest and longest-running tasks
    9.  Opus 4.8               Best for everyday, complex tasks
    10. Opus 4.7               Best for everyday, complex tasks
    11. Opus 4.6               Best for everyday, complex tasks
  ❯ 12. Sonnet 4.6             Efficient for routine tasks
";
    let complete = format!("{first}{CAPTURE_SEPARATOR}{last}");
    assert!(!claude_picker_has_more_rows(first, &complete));
    let (models, default_model) = parse_claude_model_screen(&complete).unwrap();
    let values: Vec<&str> = models.iter().map(|m| m.value.as_str()).collect();
    assert_eq!(
        values,
        [
            "claude-opus-5-5",
            "claude-fable-5-1",
            "claude-sonnet-5-5",
            "claude-haiku-4-5-20251001",
            "claude-sonnet-5",
            "claude-opus-5",
            "claude-fable-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-sonnet-4-6",
        ]
    );
    assert_eq!(default_model.as_deref(), Some("claude-opus-5-5"));
}

#[test]
fn claude_picker_rejects_incomplete_or_unknown_catalogs() {
    let first = "\
  Select model
    1. Default (recommended)  Opus 5.5
  ❯ 2. Opus 5.5 ✔             Most capable
    3. Sonnet 5               Efficient
     … +1 model
";
    assert!(claude_picker_has_more_rows(first, first));
    assert!(parse_claude_model_screen(first).is_none());

    let last = "\
  Select model
    1. Default (recommended)  Opus 5.5
    2. Opus 5.5 ✔             Most capable
    3. Sonnet 5               Efficient
  ↓ 4. Haiku 4.5              Fastest
     … +1 model
";
    let complete = format!("{first}{CAPTURE_SEPARATOR}{last}");
    assert!(!claude_picker_has_more_rows(first, &complete));
    assert_eq!(parse_claude_model_screen(&complete).unwrap().0.len(), 3);

    let unknown = first
        .replace("Sonnet 5", "Sonnet 5.1")
        .replace("     … +1 model", "");
    assert!(parse_claude_model_screen(&unknown).is_none());
}

#[test]
fn parse_claude_model_screen_drops_rows_without_a_usable_alias() {
    // A custom model whose name is not an alias must not become a selectable entry, and
    // must not become default_model even when ✔ sits on it.
    let screen = "\
   Select model

     1. Default (recommended)  Sonnet 5 · Efficient for routine tasks
     2. Sonnet                 Sonnet 5 · Efficient for routine tasks
   ❯ 3. opus (turbo mode) ✔    Custom model
";
    let (models, default_model) = parse_claude_model_screen(screen).unwrap();
    let values: Vec<&str> = models.iter().map(|m| m.value.as_str()).collect();
    assert_eq!(values, ["sonnet"]);
    assert_eq!(default_model, None);
}

#[test]
fn parse_claude_model_screen_folds_duplicate_alias_rows() {
    // Real 2.1.220 state after a stale `--model 'opus (1m context)'` launch: the CLI shows
    // the bogus value as an extra "Custom model" row that normalizes to the same alias.
    let screen = "\
   Select model

     1. Default (recommended)  Sonnet 5 · Efficient for routine tasks
     2. Opus (1M context)      Opus 5 with 1M context · Best for everyday, complex tasks
   ❯ 3. opus (1m context) ✔    Custom model
";
    let (models, default_model) = parse_claude_model_screen(screen).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].value, "opus[1m]");
    assert_eq!(models[0].label, "Opus (1M context)");
    assert_eq!(default_model.as_deref(), Some("opus[1m]"));
}

#[test]
fn parse_claude_model_screen_default_row_checked_means_cli_default() {
    // If ✔ is on the Default row, default_model is None (relying on CLI Default).
    let screen = "\
   Select model

   ❯ 1. Default (recommended) ✔  Opus 4.8 · Best for everyday tasks
     2. Sonnet                   Sonnet 5 · Efficient for routine tasks
";
    let (models, default_model) = parse_claude_model_screen(screen).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].value, "sonnet");
    assert_eq!(default_model, None);
}

#[test]
fn parse_claude_model_screen_requires_header() {
    // Numbered lines before "Select model" header (e.g. boot banners) are ignored.
    assert!(parse_claude_model_screen("  1. Fable  Fable 5\n").is_none());
}
