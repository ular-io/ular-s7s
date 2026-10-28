use super::agy::*;
use super::codex::*;

#[test]
fn parse_codex_models_filters_hidden_entries() {
    let json = r#"{"models":[
            {"slug":"gpt-5.6-sol","display_name":"GPT-5.6-Sol","description":"Latest frontier agentic coding model.","visibility":"list"},
            {"slug":"gpt-5.4","display_name":"GPT-5.4","description":"","visibility":"list"},
            {"slug":"codex-auto-review","display_name":"Codex Auto Review","visibility":"hide"}
        ]}"#;
    let models = parse_codex_models(json).unwrap();
    let values: Vec<&str> = models.iter().map(|m| m.value.as_str()).collect();
    assert_eq!(values, ["gpt-5.6-sol", "gpt-5.4"]);
    assert_eq!(models[0].label, "GPT-5.6-Sol");
    assert!(models[0].note.starts_with("Latest frontier"));
}

#[test]
fn parse_agy_models_splits_slug_and_display_name() {
    let models = parse_agy_models(
        "gemini-3.6-flash-high\tGemini 3.6 Flash (High)\n\
             claude-opus-4-6-thinking\tClaude Opus 4.6 (Thinking)\n",
    );
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].value, "gemini-3.6-flash-high");
    assert_eq!(models[0].label, "gemini-3.6-flash-high");
    assert_eq!(models[0].note, "Gemini 3.6 Flash (High)");
    assert_eq!(models[1].value, "claude-opus-4-6-thinking");
    assert!(!models.iter().any(|model| {
        model.value.contains('\t') || model.label.contains('\t') || model.note.contains('\t')
    }));
}

#[test]
fn parse_agy_models_keeps_legacy_display_name_rows() {
    let models = parse_agy_models("Gemini 3.1 Pro (High)\nGemini 3 Flash\n");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].value, "Gemini 3.1 Pro (High)");
    assert_eq!(models[0].label, "Gemini 3.1 Pro (High)");
    assert!(models[0].note.is_empty());
}

#[test]
fn normalize_agy_default_maps_legacy_display_name_to_slug() {
    let models = parse_agy_models(
        "gemini-3.6-flash-low\tGemini 3.6 Flash (Low)\n\
             gemini-3.5-flash-low\tGemini 3.5 Flash (Low)\n",
    );
    assert_eq!(
        normalize_agy_model_value(Some("Gemini 3.6 Flash (Low)".to_string()), &models).as_deref(),
        Some("gemini-3.6-flash-low")
    );
    assert_eq!(
        normalize_agy_model_value(Some("removed-model".to_string()), &models).as_deref(),
        Some("removed-model")
    );
    assert_eq!(
        normalize_agy_model_value(Some(String::new()), &models).as_deref(),
        Some("")
    );
}

#[test]
fn codex_default_model_reads_top_level_key_only() {
    let dir = std::env::temp_dir().join(format!("ular-models-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    // Skips model_reasoning_effort (similar prefix key) and ignores model key below section headers.
    std::fs::write(
            &path,
            "model_reasoning_effort = \"medium\"\nmodel = \"gpt-5.6-sol\"\n\n[features]\nmodel = \"ignored\"\n",
        )
        .unwrap();
    assert_eq!(codex_default_model(&path).as_deref(), Some("gpt-5.6-sol"));
    std::fs::remove_dir_all(&dir).ok();
}
