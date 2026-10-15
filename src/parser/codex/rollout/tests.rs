use super::*;
use crate::ui::test_support::TempBookmarkStore;

#[test]
fn logical_names_accept_only_plain_or_zstd_rollouts() {
    for name in ["rollout-test.jsonl", "rollout-test.jsonl.zst"] {
        assert_eq!(logical_name(name), Some("rollout-test.jsonl"));
    }
    for name in [
        "history.jsonl.zst",
        "rollout-test.jsonl.tmp",
        "rollout-test.zst",
    ] {
        assert_eq!(logical_name(name), None);
    }
}

#[test]
fn reads_stale_paths_in_both_storage_directions_without_materializing() {
    let temp = TempBookmarkStore::new();
    let plain = temp.path.with_file_name("rollout-test.jsonl");
    let compressed = compressed_path(&plain);
    let text = "{\"message\":\"preserve Unicode: é 🦀\"}\n";
    let bytes = zstd::stream::encode_all(text.as_bytes(), 0).unwrap();
    std::fs::write(&compressed, &bytes).unwrap();

    assert_eq!(resolve_path(&plain), compressed);
    assert_eq!(read(&plain).unwrap(), text);
    assert_eq!(read(&compressed).unwrap(), text);
    assert!(!plain.exists());
    assert_eq!(std::fs::read(&compressed).unwrap(), bytes);

    std::fs::write(&plain, text).unwrap();
    std::fs::remove_file(&compressed).unwrap();
    assert_eq!(resolve_path(&compressed), plain);
    assert_eq!(read(&compressed).unwrap(), text);
    assert!(!compressed.exists());
}

#[test]
fn prefers_plain_when_both_variants_exist() {
    let temp = TempBookmarkStore::new();
    let plain = temp.path.with_file_name("rollout-test.jsonl");
    let compressed = compressed_path(&plain);
    std::fs::write(&plain, "new content\n").unwrap();
    std::fs::write(
        &compressed,
        zstd::stream::encode_all("old content\n".as_bytes(), 0).unwrap(),
    )
    .unwrap();
    assert_eq!(read(&compressed).unwrap(), "new content\n");
}

#[test]
fn reads_concatenated_frames_and_rejects_corrupt_or_truncated_storage() {
    let temp = TempBookmarkStore::new();
    let compressed = temp.path.with_file_name("rollout-test.jsonl.zst");
    let mut frames = zstd::stream::encode_all("first\n".as_bytes(), 0).unwrap();
    frames.extend(zstd::stream::encode_all("second\n".as_bytes(), 0).unwrap());
    std::fs::write(&compressed, &frames).unwrap();
    assert_eq!(read(&compressed).unwrap(), "first\nsecond\n");

    frames.pop();
    std::fs::write(&compressed, frames).unwrap();
    assert!(read(&compressed).is_err());
    std::fs::write(&compressed, "not a zstd frame").unwrap();
    assert!(read(&compressed).is_err());
    std::fs::remove_file(&compressed).unwrap();
    assert_eq!(
        read(&compressed).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
}
