use super::*;

fn val(json: &str) -> Value {
    serde_json::from_str(json).expect("valid json")
}

#[test]
fn decode_classifies_user_text_kinds() {
    let turn = val(r#"{"type":"user","message":{"role":"user","content":"질문"}}"#);
    let noise = val(r#"{"type":"user","message":{"role":"user","content":"/usage"}}"#);
    let blank = val(r#"{"type":"user","message":{"role":"user","content":" \n "}}"#);
    let tool = val(
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}"#,
    );
    let compacted = val(
        r#"{"type":"user","isCompactSummary":true,"message":{"role":"user","content":"This session is being continued"}}"#,
    );

    for (v, expect) in [
        (&turn, "turn"),
        (&noise, "boundary"),
        (&blank, "blank"),
        (&tool, "notext"),
        (&compacted, "compact"),
    ] {
        let Some(RecordKind::User(u)) = decode(v).map(|d| d.kind) else {
            panic!("expected user record");
        };
        let got = match u.text_kind {
            UserTextKind::Turn { ref cleaned } => {
                assert_eq!(cleaned, "질문");
                "turn"
            }
            UserTextKind::Boundary => "boundary",
            UserTextKind::Blank => "blank",
            UserTextKind::NoText => "notext",
            UserTextKind::CompactSummary { .. } => "compact",
        };
        assert_eq!(got, expect);
    }
}

#[test]
fn task_notification_identified_by_origin_kind_without_text_prefix() {
    // The unified R12 identity: `origin.kind` marks a notification even when
    // the text lacks the legacy `<task-notification>` prefix, and vice versa.
    let by_origin = val(
        r#"{"type":"user","origin":{"kind":"task-notification"},"message":{"role":"user","content":"background job done"}}"#,
    );
    let by_prefix = val(
        r#"{"type":"user","message":{"role":"user","content":"<task-notification>done</task-notification>"}}"#,
    );
    let plain = val(r#"{"type":"user","message":{"role":"user","content":"질문"}}"#);

    for (v, expect) in [(&by_origin, true), (&by_prefix, true), (&plain, false)] {
        let Some(RecordKind::User(u)) = decode(v).map(|d| d.kind) else {
            panic!("expected user record");
        };
        assert_eq!(u.is_task_notification, expect);
    }
}

#[test]
fn title_events_never_join_the_chain() {
    // Even a hypothetical uuid-bearing title event must not divert the leaf
    // (unified R12 rule — the chain belongs to conversation records only).
    let values = vec![
        val(
            r#"{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문"}}"#,
        ),
        val(r#"{"type":"custom-title","uuid":"t","parentUuid":null,"customTitle":"제목"}"#),
    ];
    let filter = chain_filter(&values);
    assert!(filter.is_active(Some("a")));
    assert!(!filter.is_active(Some("t")));
}

#[test]
fn sidechain_records_neither_decode_nor_join_the_chain() {
    let side = val(
        r#"{"type":"user","uuid":"s","parentUuid":null,"isSidechain":true,"message":{"role":"user","content":"사이드체인"}}"#,
    );
    assert!(decode(&side).is_none());

    let values = vec![
        val(
            r#"{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문"}}"#,
        ),
        side,
    ];
    let filter = chain_filter(&values);
    // The trailing sidechain entry must not become the leaf.
    assert!(filter.is_active(Some("a")));
}

#[test]
fn broken_chain_keeps_every_record_active() {
    let values = vec![
        val(
            r#"{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}"#,
        ),
        val(
            r#"{"type":"user","uuid":"c","parentUuid":"ghost","message":{"role":"user","content":"질문2"}}"#,
        ),
    ];
    let filter = chain_filter(&values);
    assert!(filter.is_active(Some("a")));
    assert!(filter.is_active(Some("c")));
    assert!(filter.is_active(None));
}

#[test]
fn compact_boundary_keeps_pre_compaction_records_active() {
    // `/compact` (and automatic compaction) writes the boundary with a null
    // parentUuid; only `logicalParentUuid` reaches the record it followed.
    // Without that link the pre-compaction half reads as a dead branch.
    let values = vec![
        val(
            r#"{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}"#,
        ),
        val(
            r#"{"type":"assistant","uuid":"b","parentUuid":"a","message":{"content":[{"type":"text","text":"답1"}]}}"#,
        ),
        val(
            r#"{"type":"system","subtype":"compact_boundary","uuid":"c","parentUuid":null,"logicalParentUuid":"b","content":"Conversation compacted"}"#,
        ),
        val(
            r#"{"type":"user","uuid":"d","parentUuid":"c","isCompactSummary":true,"message":{"role":"user","content":"This session is being continued"}}"#,
        ),
        val(
            r#"{"type":"user","uuid":"e","parentUuid":"d","message":{"role":"user","content":"질문2"}}"#,
        ),
    ];
    let filter = chain_filter(&values);
    for uuid in ["a", "b", "c", "d", "e"] {
        assert!(filter.is_active(Some(uuid)), "{uuid} must stay active");
    }
}

#[test]
fn assistant_items_preserve_content_order() {
    let v = val(
        r#"{"type":"assistant","uuid":"b","parentUuid":"a","message":{"content":[{"type":"text","text":"먼저"},{"type":"tool_use","name":"Bash"},{"type":"text","text":"나중"}]}}"#,
    );
    let Some(RecordKind::Assistant { items, .. }) = decode(&v).map(|d| d.kind) else {
        panic!("expected assistant record");
    };
    let shape: Vec<&str> = items
        .iter()
        .map(|i| match i {
            AssistantItem::Text(t) => *t,
            AssistantItem::ToolUse(_) => "<tool>",
        })
        .collect();
    assert_eq!(shape, vec!["먼저", "<tool>", "나중"]);
}

#[test]
fn empty_title_event_decodes_to_nothing() {
    let v = val(r#"{"type":"custom-title","customTitle":"  "}"#);
    assert!(decode(&v).is_none());
}
