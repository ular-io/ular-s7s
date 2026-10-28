use super::parse::*;
use super::*;
use chrono::{NaiveDate, NaiveDateTime};

/// 2026-07-08(Wed) 00:09 - Captured screen time from live verification.
fn now() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 7, 8)
        .unwrap()
        .and_hms_opt(0, 9, 0)
        .unwrap()
}

fn cd(days: u16, hours: u8, minutes: u8) -> Option<ResetCountdown> {
    Some(ResetCountdown {
        days,
        hours,
        minutes,
    })
}

#[test]
fn claude_real_screen() {
    // claude v2.1.202 /usage live verification: % used + absolute reset time.
    let s = "\
   Current session
   ██▌                                                5% used
   Resets 5am (Asia/Seoul)

   Current week (all models)
   ██████████████████████▌                            45% used
   Resets Jul 10 at 5pm (Asia/Seoul)

   Current week (Fable)
   █████████████████▌                                 35% used
   Resets Jul 10 at 5pm (Asia/Seoul)";
    assert_eq!(
        parse_claude_at(s, now()),
        Some(UsageSnapshot {
            current: Some(UsageWindow {
                pct_left: 95,
                reset: cd(0, 4, 51)
            }),
            weekly: Some(UsageWindow {
                pct_left: 55,
                reset: cd(2, 16, 51)
            })
        })
    );
}

#[test]
fn codex_real_screen() {
    // codex v0.142.5 /status live verification: % left + absolute reset time (HH:MM).
    let s = "\
│  5h limit:             [███████████████████░] 95% left (resets 04:45) │
│  Weekly limit:         [██████████░░░░░░░░░░] 51% left (resets 13:25) │";
    assert_eq!(
        parse_codex_at(s, now()),
        Some(UsageSnapshot {
            current: Some(UsageWindow {
                pct_left: 95,
                reset: cd(0, 4, 36)
            }),
            weekly: Some(UsageWindow {
                pct_left: 51,
                reset: cd(0, 13, 16)
            })
        })
    );
}

#[test]
fn codex_weekday_and_date_resets() {
    // Weekday/date formats (%a %H:%M, %a %b %d). now is Wednesday -> next Monday = 7/13.
    let s = "\
5h limit: [████] 99% left (resets Mon 14:30)
Weekly limit: [██░░] 51% left (resets Mon Jul 13)";
    assert_eq!(
        parse_codex_at(s, now()),
        Some(UsageSnapshot {
            current: Some(UsageWindow {
                pct_left: 99,
                reset: cd(5, 14, 21)
            }),
            weekly: Some(UsageWindow {
                pct_left: 51,
                reset: cd(4, 23, 51)
            })
        })
    );
}

#[test]
fn codex_time_on_day_month_reset() {
    // codex weekly reset live verification: `HH:MM on DD Mon`.
    let n = NaiveDate::from_ymd_opt(2026, 7, 9)
        .unwrap()
        .and_hms_opt(17, 39, 0)
        .unwrap();
    let s = "\
│  Weekly limit:         [███████████░░░░░░░░░] 57% left (resets 17:33 on 15 Jul) │";
    assert_eq!(
        parse_codex_at(s, n),
        Some(UsageSnapshot {
            current: None,
            weekly: Some(UsageWindow {
                pct_left: 57,
                reset: cd(5, 23, 54)
            })
        })
    );
}

#[test]
fn agy_real_screen_claude_group() {
    // agy 1.0.16 /usage live verification: reads group for the active model (Claude),
    // leaving the Disabled 5h window as None.
    let s = "\
GEMINI MODELS
  Models within this group: Gemini Flash, Gemini Pro

  Weekly Limit
    [░░░░░░░░░░] 0.00%
    Refreshes in 16h 51m

  Five Hour Limit
    Disabled: You have hit your weekly limit, the 5-hour limit does not currently apply.

CLAUDE AND GPT MODELS
  Models within this group: Claude Opus, Claude Sonnet, GPT-OSS

  Weekly Limit
    [█████░░░░░] 59.00%
    Refreshes in 17h 46m

  Five Hour Limit
    [████████░░] 80.00%
    Refreshes in 2h 02m

esc to cancel                              Claude Sonnet 4.6 (Thinking)";
    assert_eq!(
        parse_agy_at(s, now()),
        Some(UsageSnapshot {
            current: Some(UsageWindow {
                pct_left: 80,
                reset: cd(0, 2, 2)
            }),
            weekly: Some(UsageWindow {
                pct_left: 59,
                reset: cd(0, 17, 46)
            })
        })
    );
}

#[test]
fn agy_gemini_group_and_disabled_5h() {
    // Disabled 5h: remaining usage 0% + countdown parsed from the "fully refresh in ..." message.
    let s = "\
GEMINI MODELS
  Weekly Limit
    [░░░░░░░░░░] 0.00%
    Refreshes in 16h 36m

  Five Hour Limit
    Disabled: You have hit your weekly limit, the 5-hour limit does not currently apply. Your weekly limit will fully refresh in 16 hours, 37 minutes.

CLAUDE AND GPT MODELS
  Weekly Limit
    [█████░░░░░] 59.00%
    Refreshes in 17h 46m

esc to cancel                              Gemini Pro";
    assert_eq!(
        parse_agy_at(s, now()),
        Some(UsageSnapshot {
            current: Some(UsageWindow {
                pct_left: 0,
                reset: cd(0, 16, 37)
            }),
            weekly: Some(UsageWindow {
                pct_left: 0,
                reset: cd(0, 16, 36)
            })
        })
    );
}

#[test]
fn agy_disabled_without_refresh_text() {
    // Retains 0% but omits the countdown if the refresh message is truncated.
    let s = "\
CLAUDE AND GPT MODELS
  Weekly Limit
    [░░░░░░░░░░] 0.00%
    Refreshes in 17h 46m

  Five Hour Limit
    Disabled: You have hit your weekly limit, the 5-hour limit does not currently app

esc to cancel                              Claude Sonnet 4.6 (Thinking)";
    assert_eq!(
        parse_agy_at(s, now()),
        Some(UsageSnapshot {
            current: Some(UsageWindow {
                pct_left: 0,
                reset: None
            }),
            weekly: Some(UsageWindow {
                pct_left: 0,
                reset: cd(0, 17, 46)
            })
        })
    );
}

#[test]
fn reset_spec_variants() {
    let n = now();
    // Absolute time: shifts to next day if the time has already passed today.
    assert_eq!(parse_reset_spec("5am (asia/seoul)", n), cd(0, 4, 51));
    assert_eq!(parse_reset_spec("11:30pm", n), cd(0, 23, 21));
    assert_eq!(parse_reset_spec("00:05", n), cd(0, 23, 56)); // 00:05 < now -> tomorrow
    assert_eq!(parse_reset_spec("jul 10 at 5pm", n), cd(2, 16, 51));
    assert_eq!(parse_reset_spec("17:33 on 15 jul", n), cd(7, 17, 24));
    assert_eq!(parse_reset_spec("15 jul at 17:33", n), cd(7, 17, 24));
    // Relative notation.
    assert_eq!(parse_reset_spec("in 1d 14h", n), cd(1, 14, 0));
    assert_eq!(parse_reset_spec("in 2 hours", n), cd(0, 2, 0));
    assert_eq!(parse_reset_spec("in 163h", n), cd(6, 19, 0));
    assert_eq!(parse_reset_spec("", n), None);
}

#[test]
fn parse_failure_returns_none() {
    assert_eq!(parse_claude_at("no numbers here", now()), None);
    assert_eq!(parse_codex_at("", now()), None);
    assert_eq!(parse_agy_at("plain text", now()), None);
}
