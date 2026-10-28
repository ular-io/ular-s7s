use super::catalog::*;
use super::*;
use std::path::PathBuf;

#[test]
fn load_from_discards_cache_written_by_an_older_schema() {
    let dir = std::env::temp_dir().join(format!("ular-models-ver-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("models.json");
    // v1 caches can hold `opus (1m context)` as a claude `--model` value; discard wholesale
    // so the startup version gate re-queries instead of serving it.
    std::fs::write(
            &path,
            r#"{"version":1,"profiles":{"builtin-claude":{"agent":"Claude","cli_version":"2.1.220","models":[{"value":"opus (1m context)","label":"Opus (1M context)","note":""}],"default_model":"opus (1m context)"}}}"#,
        )
        .unwrap();
    assert!(ModelCatalog::load_from(&path)
        .cached_version("builtin-claude")
        .is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn load_from_discards_v3_claude_rows_missing_versioned_models() {
    let dir = std::env::temp_dir().join(format!("ular-models-claude-ver-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("models.json");
    std::fs::write(
            &path,
            r#"{"version":3,"profiles":{"builtin-claude":{"agent":"Claude","cli_version":"2.1.283","models":[{"value":"opus[1m]","label":"Opus (1M context)","note":""}],"default_model":"opus[1m]"}}}"#,
        )
        .unwrap();
    assert!(ModelCatalog::load_from(&path)
        .cached_version("builtin-claude")
        .is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn load_from_discards_v2_agy_rows_with_embedded_tabs() {
    let dir = std::env::temp_dir().join(format!("ular-models-agy-ver-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("models.json");
    std::fs::write(
            &path,
            r#"{"version":2,"profiles":{"builtin-antigravity":{"agent":"Antigravity","cli_version":"1.1.11","models":[{"value":"gemini-3.6-flash-high\tGemini 3.6 Flash (High)","label":"gemini-3.6-flash-high\tGemini 3.6 Flash (High)","note":""}],"default_model":null}}}"#,
        )
        .unwrap();
    assert!(ModelCatalog::load_from(&path)
        .cached_version("builtin-antigravity")
        .is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn catalog_falls_back_to_default_antigravity_entry() {
    let mut catalog = ModelCatalog::default();
    catalog.insert(
        "builtin-antigravity".to_string(),
        ProfileModels {
            agent: Agent::Antigravity,
            cli_version: Some("1.1.2".to_string()),
            models: vec![ModelEntry {
                value: "Gemini 3.1 Pro (High)".to_string(),
                label: "Gemini 3.1 Pro (High)".to_string(),
                note: String::new(),
            }],
            default_model: Some("Gemini 3.5 Flash (High)".to_string()),
            last_selected: None,
        },
    );
    // Extra profiles of agy that cannot have env injected share the default profile's model list.
    let extra = Profile {
        id: "profile-agy2".to_string(),
        agent: Agent::Antigravity,
        name: "Agy2".to_string(),
        path: PathBuf::from("/tmp/agy2"),
        oauth_token: None,
        active: true,
        shortcut: None,
        builtin: false,
    };
    let pm = catalog.for_profile(&extra).unwrap();
    assert_eq!(pm.models.len(), 1);

    // Other agents do not fallback.
    let claude = Profile {
        id: "profile-claude2".to_string(),
        agent: Agent::Claude,
        path: PathBuf::from("/tmp/claude2"),
        ..extra.clone()
    };
    assert!(catalog.for_profile(&claude).is_none());
}

#[test]
fn catalog_roundtrip_save_load() {
    let dir = std::env::temp_dir().join(format!("ular-models-cache-{}", std::process::id()));
    let path = dir.join("models.json");
    let mut catalog = ModelCatalog::default();
    catalog.insert(
        "builtin-codex".to_string(),
        ProfileModels {
            agent: Agent::Codex,
            cli_version: Some("codex-cli 0.144.3".to_string()),
            models: vec![ModelEntry {
                value: "gpt-5.6-sol".to_string(),
                label: "GPT-5.6-Sol".to_string(),
                note: String::new(),
            }],
            default_model: Some("gpt-5.6-sol".to_string()),
            last_selected: None,
        },
    );
    catalog.save_to(&path).unwrap();
    let loaded = ModelCatalog::load_from(&path);
    assert_eq!(
        loaded.cached_version("builtin-codex"),
        Some(Some("codex-cli 0.144.3".to_string()))
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn insert_preserves_last_selected_across_refetch() {
    let mut catalog = ModelCatalog::default();
    let base = |default_model: Option<&str>| ProfileModels {
        agent: Agent::Codex,
        cli_version: Some("codex-cli 0.144.6".to_string()),
        models: vec![ModelEntry {
            value: "gpt-5.6-sol".to_string(),
            label: "GPT-5.6-Sol".to_string(),
            note: String::new(),
        }],
        default_model: default_model.map(str::to_string),
        last_selected: None,
    };
    catalog.insert("builtin-codex".to_string(), base(Some("gpt-5.6-sol")));
    catalog.set_last_selected("builtin-codex", LastSelection::Model("gpt-5.5".to_string()));
    // A fresh fetch result (last_selected == None) must not wipe the stored pick.
    catalog.insert("builtin-codex".to_string(), base(Some("gpt-5.6-terra")));
    let pm = catalog.entries.get("builtin-codex").unwrap();
    assert_eq!(
        pm.last_selected,
        Some(LastSelection::Model("gpt-5.5".to_string()))
    );
    // But the rest of the catalog is refreshed as usual.
    assert_eq!(pm.default_model.as_deref(), Some("gpt-5.6-terra"));

    // set_last_selected on an unknown profile is a no-op (no panic).
    catalog.set_last_selected("ghost", LastSelection::Default);
    assert!(!catalog.entries.contains_key("ghost"));
}

#[test]
fn last_selected_survives_save_load_roundtrip() {
    let dir = std::env::temp_dir().join(format!("ular-models-last-sel-{}", std::process::id()));
    let path = dir.join("models.json");
    let mut catalog = ModelCatalog::default();
    catalog.insert(
        "builtin-antigravity".to_string(),
        ProfileModels {
            agent: Agent::Antigravity,
            cli_version: Some("1.1.5".to_string()),
            models: Vec::new(),
            default_model: None,
            last_selected: Some(LastSelection::Default),
        },
    );
    catalog.save_to(&path).unwrap();
    let loaded = ModelCatalog::load_from(&path);
    assert_eq!(
        loaded
            .entries
            .get("builtin-antigravity")
            .and_then(|m| m.last_selected.clone()),
        Some(LastSelection::Default)
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fallback_models_only_for_claude() {
    let claude = fallback_models(Agent::Claude);
    assert!(claude.iter().any(|m| m.value == "fable"));
    assert!(claude
        .iter()
        .all(|m| m.value == m.label.to_ascii_lowercase()));
    assert!(fallback_models(Agent::Codex).is_empty());
    assert!(fallback_models(Agent::Antigravity).is_empty());
}

#[test]
fn saves_from_two_instances_keep_each_others_profiles() {
    let dir = std::env::temp_dir().join(format!("s7s-models-merge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("models.json");
    let models = |agent: Agent| ProfileModels {
        agent,
        cli_version: Some("1".into()),
        models: Vec::new(),
        default_model: None,
        last_selected: None,
    };

    let mut a = ModelCatalog::load_from(&path);
    let mut b = ModelCatalog::load_from(&path);
    a.insert("claude".into(), models(Agent::Claude));
    a.save_to(&path).unwrap();
    // `b` never saw `claude`; its save must not drop it.
    b.insert("codex".into(), models(Agent::Codex));
    b.save_to(&path).unwrap();
    let merged = ModelCatalog::load_from(&path);
    assert!(merged.entries.contains_key("claude") && merged.entries.contains_key("codex"));
    // The saving instance adopts the merged file.
    assert!(b.entries.contains_key("claude"));

    // A removal reaches the file; an unrelated reload keeps unsaved changes.
    a.reload_from(&path);
    a.remove("codex");
    a.insert("agy".into(), models(Agent::Antigravity));
    a.reload_from(&path);
    assert!(!a.entries.contains_key("codex") && a.entries.contains_key("agy"));
    a.save_to(&path).unwrap();
    let saved = ModelCatalog::load_from(&path);
    assert!(!saved.entries.contains_key("codex") && saved.entries.contains_key("agy"));
    let _ = std::fs::remove_dir_all(&dir);
}
