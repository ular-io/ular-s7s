use super::file::theme_from_file;
use super::*;

#[test]
fn builtins_have_unique_keys_and_expected_split() {
    let themes = builtin_themes();
    assert_eq!(themes.len(), 40);
    assert_eq!(themes.iter().filter(|t| t.dark).count(), 20);
    assert_eq!(themes.iter().filter(|t| !t.dark).count(), 20);
    let mut keys: Vec<&str> = themes.iter().map(|t| t.key.as_str()).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), 40);
    assert!(themes.iter().all(|t| !t.custom));
}

#[test]
fn ular_light_is_fully_hex_fixed_on_cream() {
    let t = builtin_themes()
        .into_iter()
        .find(|t| t.key == "ular-light")
        .expect("ular-light exists");
    assert_eq!(t.bg, Color::Rgb(0xFD, 0xF6, 0xE3));
    // No terminal-delegated (Reset / ANSI-named) roles remain: every role must
    // be a concrete Rgb so rendering matches on any terminal and the backdrop
    // fade can blend against known colors.
    let roles = [
        t.bg,
        t.fg,
        t.muted,
        t.dim,
        t.accent,
        t.on_accent,
        t.selection_bg,
        t.selection_fg,
        t.selection_inactive_bg,
        t.bookmark_bg,
        t.key_hint,
        t.usage_high,
        t.usage_low,
        t.button_focus_bg,
        t.button_focus_fg,
        t.button_bg,
        t.button_fg,
        t.success,
        t.warning,
        t.error,
        t.agent_claude,
        t.agent_codex,
        t.agent_antigravity,
    ];
    assert!(roles.iter().all(|c| matches!(c, Color::Rgb(..))));
}

#[test]
fn default_theme_is_nord() {
    let theme = default_theme();
    assert_eq!(theme.key, "nord");
    assert_eq!(theme.bg, Color::Rgb(0x2E, 0x34, 0x40));
}

#[test]
fn resolve_unknown_key_falls_back_to_default() {
    assert_eq!(resolve("no-such-theme").key, DEFAULT_THEME_KEY);
}

#[test]
fn parse_color_accepts_hex_and_names() {
    assert_eq!(parse_color("#FF8000"), Some(Color::Rgb(255, 128, 0)));
    assert_eq!(parse_color("ff8000"), Some(Color::Rgb(255, 128, 0)));
    assert_eq!(parse_color(" DarkGray "), Some(Color::DarkGray));
    assert_eq!(parse_color("default"), Some(Color::Reset));
    assert_eq!(parse_color("#12345"), None);
    assert_eq!(parse_color("not-a-color"), None);
}

#[test]
fn theme_file_overrides_partial_roles_over_base() {
    let data = r##"
name = "My Theme"
dark = false
base = "dracula"

[colors]
bg = "#101010"
bookmark_bg = "#202030"
error = "red"
bogus_role = "#FFFFFF"
warning = "not-a-color"
"##;
    let theme = theme_from_file("my-theme", data).expect("parses");
    assert_eq!(theme.key, "my-theme");
    assert_eq!(theme.name, "My Theme");
    assert!(!theme.dark);
    assert!(theme.custom);
    assert_eq!(theme.bg, Color::Rgb(0x10, 0x10, 0x10));
    assert_eq!(theme.bookmark_bg, Color::Rgb(0x20, 0x20, 0x30));
    assert_eq!(theme.error, Color::Red);
    // Unparsable value keeps the base (Dracula) color.
    let dracula = builtin_themes()
        .into_iter()
        .find(|t| t.key == "dracula")
        .unwrap();
    assert_eq!(theme.warning, dracula.warning);
    assert_eq!(theme.fg, dracula.fg);
}

#[test]
fn theme_file_defaults_name_and_base() {
    let theme = theme_from_file("bare", "").expect("empty file parses");
    assert_eq!(theme.name, "bare");
    assert!(theme.custom);
    let nord = default_theme();
    assert_eq!(theme.bg, nord.bg);
    assert_eq!(theme.accent, nord.accent);
    assert_eq!(theme.bookmark_bg, nord.bookmark_bg);
}

#[test]
fn custom_themes_load_from_dir_sorted() {
    let dir = std::env::temp_dir().join(format!("s7s-theme-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("zeta.toml"), "name = \"Zeta\"\n").unwrap();
    std::fs::write(dir.join("alpha.toml"), "name = \"Alpha\"\n").unwrap();
    std::fs::write(dir.join("ignored.txt"), "not a theme").unwrap();
    std::fs::write(dir.join("broken.toml"), "name = [not toml").unwrap();
    let themes = load_custom_themes_from(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        themes.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        vec!["Alpha", "Zeta"]
    );
    assert!(themes.iter().all(|t| t.custom));
}

#[test]
fn themes_sorting_order() {
    let themes = all_themes();

    // 1. All non-CVD themes must be placed before CVD themes
    let cvd_start_idx = themes
        .iter()
        .position(|t| t.name.starts_with("(CVD) "))
        .unwrap();

    for theme in &themes[0..cvd_start_idx] {
        assert!(!theme.name.starts_with("(CVD) "));
    }

    for theme in &themes[cvd_start_idx..] {
        assert!(theme.name.starts_with("(CVD) "));
    }

    // 2. Non-CVD themes must be sorted alphabetically (case-insensitive)
    for i in 0..cvd_start_idx - 1 {
        let name_a = themes[i].name.to_lowercase();
        let name_b = themes[i + 1].name.to_lowercase();
        assert!(
            name_a <= name_b,
            "Non-CVD order mismatch: {} vs {}",
            themes[i].name,
            themes[i + 1].name
        );
    }

    // 3. CVD themes must be sorted alphabetically (case-insensitive)
    for i in cvd_start_idx..themes.len() - 1 {
        let name_a = themes[i].name.to_lowercase();
        let name_b = themes[i + 1].name.to_lowercase();
        assert!(
            name_a <= name_b,
            "CVD order mismatch: {} vs {}",
            themes[i].name,
            themes[i + 1].name
        );
    }
}

#[test]
fn ular_light_keeps_user_tuned_markdown_colors() {
    let theme = builtin_themes()
        .into_iter()
        .find(|t| t.key == "ular-light")
        .expect("ular-light exists");
    assert_eq!(theme.md, super::markdown::ULAR_LIGHT);
}

#[test]
fn derived_markdown_colors_meet_contrast_targets() {
    use super::markdown::contrast;
    for t in builtin_themes()
        .into_iter()
        .filter(|t| t.key != "ular-light")
    {
        let (md, bg) = (t.md, t.bg_rgb());
        let on = |fg: Color, bg| contrast(color_rgb(fg), bg);
        assert!(
            on(md.heading_fg, color_rgb(md.heading_bg)) >= 4.5,
            "{}",
            t.key
        );
        assert!(on(md.code_fg, color_rgb(md.code_bg)) >= 4.5, "{}", t.key);
        for (role, color) in [
            ("code_block_fg", md.code_block_fg),
            ("table_head", md.table_head),
            ("link", md.link),
            ("quote", md.quote),
        ] {
            assert!(on(color, bg) >= 4.5, "{} {role}", t.key);
        }
        assert!(on(md.marker, bg) >= 3.0, "{} marker", t.key);
        for fill in [
            md.alert_note,
            md.alert_tip,
            md.alert_important,
            md.alert_warning,
            md.alert_caution,
        ] {
            assert!(on(md.alert_fg, color_rgb(fill)) >= 4.5, "{} alert", t.key);
        }
    }
}

#[test]
fn custom_theme_rederives_markdown_only_when_base_colors_change() {
    let kept = theme_from_file("kept", "base = \"ular-light\"\n").expect("parses");
    assert_eq!(kept.md, super::markdown::ULAR_LIGHT);
    let changed = theme_from_file(
        "changed",
        "base = \"ular-light\"\n[colors]\nbg = \"#ffffff\"\nmd_link = \"#123456\"\n",
    )
    .expect("parses");
    assert_ne!(
        changed.md.heading_bg,
        super::markdown::ULAR_LIGHT.heading_bg
    );
    assert_eq!(changed.md.link, Color::Rgb(0x12, 0x34, 0x56));
}
