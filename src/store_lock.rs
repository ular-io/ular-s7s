//! Cross-process exclusion for app-owned JSON stores that several running s7s
//! instances read-modify-write (`profiles.json`, `workspaces.json`).
//!
//! A writer holds the lock from re-reading the file until its atomic replace
//! lands, so two instances saving at once cannot both start from the same old
//! copy and drop each other's change. Readers do not lock: stores are replaced
//! by rename, so a reader sees either the old or the new file, never a mix.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Runs `f` while holding an exclusive advisory lock on a sidecar
/// `.<store name>.lock` file next to `store`. The lock is released when the
/// lock file handle drops, including on error or panic.
pub(crate) fn with_store_lock<T>(store: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let parent = store.parent().context("store has no parent directory")?;
    fs::create_dir_all(parent)?;
    let name = store
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "store".to_string());
    let lock_path = parent.join(format!(".{name}.lock"));
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&lock_path)
        .with_context(|| format!("open {}", lock_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // SAFETY: `file` owns a valid descriptor for the duration of the call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("lock {}", lock_path.display()));
        }
    }
    let result = f();
    drop(file);
    result
}

/// Writes `data` to a fresh `0600` temp file beside `path` and renames it over
/// `path`, so concurrent readers never observe a partially written store.
pub(crate) fn replace_file(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().context("store has no parent directory")?;
    fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "store".to_string());
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temp = parent.join(format!(".{name}.tmp-{}-{nonce}", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<()> {
        let mut file = options.open(&temp)?;
        file.write_all(data)?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_read_modify_write_keeps_every_increment() {
        let root = crate::ui::test_support::TempBookmarkStore::new();
        let path = root.path.with_file_name("counter.json");
        replace_file(&path, b"0").unwrap();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        with_store_lock(&path, || {
                            let n: u32 = fs::read_to_string(&path)?.parse()?;
                            replace_file(&path, (n + 1).to_string().as_bytes())
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "160");
    }
}
