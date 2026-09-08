//! Shared scratch workspace: the working directory used by sessions started
//! from the `[SCRATCH]` option of the New Session folder dropdown.
//!
//! The folder exists so a question/inspection session can run without picking a
//! project. Two properties make that work, and both are enforced here rather
//! than by filesystem permissions (the agent runs as the same user and would
//! simply restore a write bit it hit):
//!
//! - It is emptied on every launch, so no earlier session's leftovers become
//!   context for the next one. A stale `CLAUDE.md`/`AGENTS.md` in the working
//!   directory would be loaded by the agent CLI on startup, which is exactly
//!   what the folder is meant to avoid.
//! - It carries a policy file telling the agent not to write here and to ask the
//!   user for a target directory instead. Blocking writes without naming an
//!   alternative only moves the file to an unpredictable location.
//!
//! The policy text is the source of truth for both agents: `AGENTS.md` holds it
//! and `CLAUDE.md` imports that file.

use std::io;
use std::path::{Path, PathBuf};

/// Policy file read by codex and agy, and imported by claude through
/// [`POLICY_CLAUDE_MD`]. Rewritten on every launch, so an agent that edited or
/// deleted it cannot disarm the policy for the next session.
const POLICY_AGENTS_MD: &str = r#"# Scratch Workspace

Managed by s7s. This file is rewritten every time a session starts here; edits
to it are lost.

This folder is a shared scratch workspace for sessions started without a
project. It is not a project and carries no project context.

- Every file in this folder is deleted without warning when a session starts.
  Nothing left here survives, including your own notes.
- Do not create files in this folder.
- When work produces a file worth keeping, do not choose a location yourself:
  ask the user which directory to save it in, and write it there.
- Temporary files and helper scripts belong in your own temporary location (or
  `$TMPDIR`), never in this folder.
"#;

/// Claude reads `CLAUDE.md`; pointing it at `AGENTS.md` keeps one source of
/// truth for the policy text.
const POLICY_CLAUDE_MD: &str = "@AGENTS.md\n";

/// Policy file names. Excluded from the launch purge (they are the policy) and
/// from the "folder is not empty" judgement.
const POLICY_FILES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

/// Display label used wherever the workspace would otherwise appear as an
/// ordinary folder name or path. Scratch sessions have no project, so the
/// internal config path is noise in a list.
pub const LABEL: &str = "[SCRATCH]";

/// Folder label for lists and metadata: [`LABEL`] for the workspace, the given
/// basename otherwise.
///
/// Exact match only — this runs in render paths, so it does no filesystem work;
/// a `cwd` recorded in a non-canonical form simply keeps its basename.
pub fn folder_label<'a>(cwd: &Path, folder: &'a str) -> &'a str {
    if cwd == dir() {
        LABEL
    } else {
        folder
    }
}

/// Path of the shared scratch workspace.
pub fn dir() -> PathBuf {
    crate::config::scratch_dir()
}

/// Whether `path` is the scratch workspace itself. Both sides are compared after
/// canonicalization when possible so a launch through a symlinked home still
/// matches (the New Session dialog canonicalizes its input before launching).
pub fn is_scratch(path: &Path) -> bool {
    let scratch = dir();
    if path == scratch {
        return true;
    }
    // Cheap prune before touching the filesystem: this runs once per indexed
    // session every time the New Session dialog builds its folder list.
    if path.file_name() != scratch.file_name() {
        return false;
    }
    match (path.canonicalize(), scratch.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Creates the folder if missing, without touching its contents. Used by the
/// dialog so path validation succeeds before the launch purge runs.
pub fn ensure() -> io::Result<PathBuf> {
    let dir = dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Empties the workspace and rewrites the policy files. Called immediately
/// before handing the terminal to the agent.
///
/// Refuses any directory that is not the scratch workspace: this deletes files
/// without a confirmation prompt, so the target is verified rather than trusted
/// from the caller.
pub fn prepare(path: &Path) -> io::Result<()> {
    if !is_scratch(path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not the scratch workspace",
        ));
    }
    let dir = ensure()?;
    purge(&dir)?;
    std::fs::write(dir.join("AGENTS.md"), POLICY_AGENTS_MD)?;
    std::fs::write(dir.join("CLAUDE.md"), POLICY_CLAUDE_MD)?;
    Ok(())
}

/// Deletes every direct entry except the policy files.
///
/// Only direct entries are touched and symlinks are removed as links, never
/// followed: a link left in the workspace must not turn the purge into a delete
/// somewhere else. A single failing entry does not abort the rest — the launch
/// is more useful than a perfectly empty folder.
fn purge(dir: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        if POLICY_FILES.iter().any(|p| name == *p) {
            continue;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let _ = if kind.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_label_marks_only_the_workspace() {
        assert_eq!(folder_label(&dir(), "scratch"), LABEL);
        // A folder that merely shares the name keeps its own label.
        assert_eq!(
            folder_label(std::path::Path::new("/tmp/scratch"), "scratch"),
            "scratch"
        );
        assert_eq!(
            folder_label(std::path::Path::new("/tmp/work/web"), "web"),
            "web"
        );
    }

    /// `prepare` is the only destructive path in the app that runs without a
    /// confirmation prompt, so the target check is asserted directly.
    #[test]
    fn prepare_refuses_a_directory_that_is_not_the_scratch_workspace() {
        let tmp = std::env::temp_dir().join(format!("s7s-scratch-guard-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("temp dir");
        let keep = tmp.join("keep.txt");
        std::fs::write(&keep, "important").expect("write");

        let err = prepare(&tmp).expect_err("must refuse a foreign directory");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(keep.exists(), "a refused target must not be touched");

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn purge_clears_leftovers_and_keeps_policy_files() {
        let tmp = std::env::temp_dir().join(format!("s7s-scratch-purge-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("temp dir");
        std::fs::write(tmp.join("AGENTS.md"), "policy").expect("write");
        std::fs::write(tmp.join("CLAUDE.md"), "import").expect("write");
        std::fs::write(tmp.join("report.md"), "leftover").expect("write");
        std::fs::create_dir_all(tmp.join("out/nested")).expect("nested dir");
        std::fs::write(tmp.join("out/nested/data.json"), "{}").expect("write");

        purge(&tmp).expect("purge");

        assert!(tmp.join("AGENTS.md").exists());
        assert!(tmp.join("CLAUDE.md").exists());
        assert!(!tmp.join("report.md").exists());
        assert!(!tmp.join("out").exists());

        std::fs::remove_dir_all(&tmp).ok();
    }

    /// Real-path check: `prepare` against the actual workspace, so the guard, the
    /// purge, and the policy text are verified where they run. Ignored because it
    /// writes under the user's config root.
    ///
    /// `cargo test real_scratch_prepare -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_scratch_prepare_clears_and_rewrites_the_policy() {
        let dir = ensure().expect("scratch dir");
        std::fs::write(dir.join("leftover.txt"), "stale").expect("write");
        std::fs::create_dir_all(dir.join("out")).expect("dir");
        // A tampered policy file must be restored, not left as the agent wrote it.
        std::fs::write(dir.join("AGENTS.md"), "tampered").expect("write");

        prepare(&dir).expect("prepare");

        assert!(!dir.join("leftover.txt").exists());
        assert!(!dir.join("out").exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("AGENTS.md")).expect("read"),
            POLICY_AGENTS_MD
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("CLAUDE.md")).expect("read"),
            POLICY_CLAUDE_MD
        );
    }

    /// A symlink in the workspace must be unlinked, leaving its target alone.
    #[cfg(unix)]
    #[test]
    fn purge_removes_symlinks_without_following_them() {
        let tmp = std::env::temp_dir().join(format!("s7s-scratch-link-{}", std::process::id()));
        let outside = std::env::temp_dir().join(format!("s7s-scratch-out-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("temp dir");
        std::fs::create_dir_all(&outside).expect("outside dir");
        let target = outside.join("precious.txt");
        std::fs::write(&target, "keep me").expect("write");
        std::os::unix::fs::symlink(&outside, tmp.join("link")).expect("symlink");

        purge(&tmp).expect("purge");

        assert!(!tmp.join("link").exists());
        assert!(target.exists(), "the symlink target must survive");

        std::fs::remove_dir_all(&tmp).ok();
        std::fs::remove_dir_all(&outside).ok();
    }
}
