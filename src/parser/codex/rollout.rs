//! Physical Codex rollout storage: plain JSONL or Zstandard-compressed JSONL.
//!
//! Codex can compress an inactive rollout or materialize it again while s7s is
//! open. Resolve either representation at read time without modifying the store.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// The logical JSONL name shared by a rollout's plain and compressed files.
pub(crate) fn logical_name(name: &str) -> Option<&str> {
    let plain = name.strip_suffix(".zst").unwrap_or(name);
    (plain.starts_with("rollout-") && plain.ends_with(".jsonl")).then_some(plain)
}

/// The plain path is also the identity used to deduplicate physical variants.
pub(crate) fn plain_path(path: &Path) -> PathBuf {
    if path.extension().is_some_and(|ext| ext == "zst") {
        path.with_extension("")
    } else {
        path.to_path_buf()
    }
}

pub(crate) fn compressed_path(path: &Path) -> PathBuf {
    let mut name = plain_path(path).into_os_string();
    name.push(".zst");
    PathBuf::from(name)
}

/// Prefer the writable plain file when both variants coexist during maintenance.
pub(crate) fn resolve_path(path: &Path) -> PathBuf {
    let plain = plain_path(path);
    if plain.is_file() {
        return plain;
    }
    let compressed = compressed_path(path);
    if compressed.is_file() {
        return compressed;
    }
    path.to_path_buf()
}

pub(crate) fn read(path: &Path) -> io::Result<String> {
    let resolved = resolve_path(path);
    let (file, resolved) = match File::open(&resolved) {
        Ok(file) => (file, resolved),
        // Compression/materialization can finish after resolve_path's checks.
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let alternate = if resolved.extension().is_some_and(|ext| ext == "zst") {
                plain_path(&resolved)
            } else {
                compressed_path(&resolved)
            };
            (File::open(&alternate)?, alternate)
        }
        Err(err) => return Err(err),
    };
    let mut text = String::new();
    if resolved.extension().is_some_and(|ext| ext == "zst") {
        zstd::stream::read::Decoder::new(file)?.read_to_string(&mut text)?;
    } else {
        let mut file = file;
        file.read_to_string(&mut text)?;
    }
    Ok(text)
}

#[cfg(test)]
mod tests;
