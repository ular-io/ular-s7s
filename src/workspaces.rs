//! User-defined session workspaces: named, s7s-owned scopes over the session
//! index (include words, exclude words, and a folder set). The Session screen's
//! workspace pane opens and deletes them and its edit dialog adds or edits one;
//! the open workspace narrows every session list in the TUI.
//!
//! Not to be confused with `session_workspace` (cwd facts captured for
//! s7s-created sessions) or the scratch workspace (`scratch.rs`): this module
//! never touches agent storage, it only filters what the index already holds.
//!
//! Several s7s instances may share the file. Each writes only its own change
//! ([`WorkspaceChange`]) onto a freshly read copy under a lock
//! ([`WorkspaceStore::commit`]), so one instance never drops another's
//! workspaces. An instance's in-memory list follows its own edits and is
//! replaced from disk only at startup and on `ctrl+u`.

use crate::filter::token_matches;
use crate::model::Session;
use crate::normalize;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const STORE_VERSION: u32 = 1;

/// Name given to a workspace created with `+`, before the user renames it.
pub(crate) const NEW_WORKSPACE_NAME: &str = "New Workspace";

/// Reserved label of the fixed "every session" row; never a stored workspace name.
pub(crate) const ALL_WORKSPACE_NAME: &str = "All";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Workspace {
    /// Stable identity; names can change and `active` must survive a rename.
    pub id: String,
    pub name: String,
    /// Whitespace-separated words that must all occur in a session.
    #[serde(default)]
    pub includes: String,
    /// Whitespace-separated words; a session containing any of them is excluded.
    #[serde(default)]
    pub excludes: String,
    /// Absolute session cwds. Empty means no folder restriction. Compared by
    /// full path, unlike the basename-keyed folder filter, so two projects that
    /// share a folder name stay distinct.
    #[serde(default)]
    pub folders: Vec<PathBuf>,
}

impl Workspace {
    pub(crate) fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            includes: String::new(),
            excludes: String::new(),
            folders: Vec::new(),
        }
    }

    /// Include/exclude words search the same text as the `/` keyword search
    /// (`filter::token_matches`); folders compare the session's absolute cwd.
    pub(crate) fn matches(&self, s: &Session) -> bool {
        if !self.folders.is_empty() && !self.folders.iter().any(|f| f == &s.cwd) {
            return false;
        }
        let includes = normalize::nfc_lower(&self.includes);
        if !includes.split_whitespace().all(|t| token_matches(s, t)) {
            return false;
        }
        let excludes = normalize::nfc_lower(&self.excludes);
        !excludes.split_whitespace().any(|t| token_matches(s, t))
    }

    pub(crate) fn has_folder(&self, folder: &Path) -> bool {
        self.folders.iter().any(|f| f == folder)
    }

    /// Adds or removes one folder, keeping the selection order otherwise intact.
    pub(crate) fn toggle_folder(&mut self, folder: &Path) {
        if let Some(pos) = self.folders.iter().position(|f| f == folder) {
            self.folders.remove(pos);
        } else {
            self.folders.push(folder.to_path_buf());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkspaceStore {
    version: u32,
    pub workspaces: Vec<Workspace>,
    /// Id of the open workspace; `None` is the fixed "All" scope. In memory it
    /// is this instance's scope; on disk it is the scope last opened by any
    /// instance, read only at startup so another instance's cursor never
    /// changes what this one lists.
    #[serde(default)]
    pub active: Option<String>,
}

/// One persisted edit, applied by id onto the current file contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceChange {
    /// Replace the workspace with this id, or append it when absent (also when
    /// another instance deleted it meanwhile: the edit being saved wins).
    Upsert(Workspace),
    Remove(String),
    /// Scope opened by this instance, remembered for the next startup.
    Opened(Option<String>),
}

impl Default for WorkspaceStore {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            workspaces: Vec::new(),
            active: None,
        }
    }
}

impl WorkspaceStore {
    /// Read errors are surfaced so a later save cannot overwrite an unreadable
    /// or newer store with an empty one (same contract as bookmarks).
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err.into()),
        };
        let mut store: Self =
            serde_json::from_slice(&data).with_context(|| format!("parse {}", path.display()))?;
        if store.version != STORE_VERSION {
            bail!("unsupported workspace version {}", store.version);
        }
        store.sort();
        if store.active_index().is_none() {
            store.active = None;
        }
        Ok(store)
    }

    /// `load` for a starting TUI: every start opens "All", so the stored
    /// `active` (the last scope of whichever instance saved last) is dropped.
    pub(crate) fn load_at_startup(path: &Path) -> Result<Self> {
        let mut store = Self::load(path)?;
        store.active = None;
        Ok(store)
    }

    /// Atomic whole-file replace. Callers outside tests go through
    /// [`Self::commit`] so the replace starts from the current file. No fsync:
    /// the file is rewritten on every workspace cursor move, and losing the
    /// latest move in a crash only reopens the previous scope.
    fn save(&self, path: &Path) -> Result<()> {
        crate::store_lock::replace_file(path, &serde_json::to_vec_pretty(self)?)
    }

    fn apply(&mut self, change: &WorkspaceChange) {
        match change {
            WorkspaceChange::Upsert(ws) => {
                match self.workspaces.iter_mut().find(|w| w.id == ws.id) {
                    Some(stored) => *stored = ws.clone(),
                    None => self.workspaces.push(ws.clone()),
                }
            }
            WorkspaceChange::Remove(id) => self.workspaces.retain(|w| &w.id != id),
            WorkspaceChange::Opened(id) => self.active = id.clone(),
        }
    }

    /// Re-reads the file under the store lock, applies `changes`, and writes it
    /// back. An unreadable or newer-version file is left untouched (error).
    /// A name already taken in the file by a different workspace (created by
    /// another instance since this one last read it) is refused the same way
    /// as a local duplicate.
    pub(crate) fn commit(path: &Path, changes: &[WorkspaceChange]) -> Result<()> {
        crate::store_lock::with_store_lock(path, || {
            let mut store = Self::load(path)?;
            for change in changes {
                if let WorkspaceChange::Upsert(ws) = change {
                    let stored = store.workspaces.iter().position(|w| w.id == ws.id);
                    let renamed = stored.is_none_or(|i| store.workspaces[i].name != ws.name);
                    if renamed && !store.name_available(&ws.name, stored) {
                        bail!("a workspace named '{}' already exists", ws.name.trim());
                    }
                }
                store.apply(change);
            }
            store.sort();
            store.save(path)
        })
    }

    /// Orders workspaces by name as text (case-insensitive, NFC), so Latin
    /// names precede Hangul ones and digits precede both. Every list shows this
    /// order; ties fall back to the exact name, then the id.
    pub(crate) fn sort(&mut self) {
        self.workspaces.sort_by_cached_key(|w| {
            (
                normalize::nfc_lower(w.name.trim()),
                w.name.clone(),
                w.id.clone(),
            )
        });
    }

    /// Replaces the workspace with the same id, or adds it, keeping the order.
    pub(crate) fn upsert(&mut self, ws: Workspace) {
        self.apply(&WorkspaceChange::Upsert(ws));
        self.sort();
    }

    pub(crate) fn active_index(&self) -> Option<usize> {
        let id = self.active.as_deref()?;
        self.workspaces.iter().position(|w| w.id == id)
    }

    pub(crate) fn active_workspace(&self) -> Option<&Workspace> {
        self.active_index().map(|i| &self.workspaces[i])
    }

    /// Opens the workspace at `idx`, or "All" for `None` / an out-of-range index.
    pub(crate) fn set_active(&mut self, idx: Option<usize>) {
        self.active = idx
            .and_then(|i| self.workspaces.get(i))
            .map(|w| w.id.clone());
    }

    /// Whether `name` is free for the workspace at `except` (case-insensitive,
    /// trimmed). "All" is reserved: the palette lists every workspace as
    /// `Open Workspace <name>`, so names must identify one row.
    pub(crate) fn name_available(&self, name: &str, except: Option<usize>) -> bool {
        let key = normalize::nfc_lower(name.trim());
        if key == normalize::nfc_lower(ALL_WORKSPACE_NAME) {
            return false;
        }
        !self
            .workspaces
            .iter()
            .enumerate()
            .any(|(i, w)| Some(i) != except && normalize::nfc_lower(w.name.trim()) == key)
    }

    /// First free `New Workspace`, `New Workspace 2`, ... name.
    pub(crate) fn next_new_name(&self) -> String {
        let mut n = 1;
        loop {
            let name = if n == 1 {
                NEW_WORKSPACE_NAME.to_string()
            } else {
                format!("{NEW_WORKSPACE_NAME} {n}")
            };
            if self.name_available(&name, None) {
                return name;
            }
            n += 1;
        }
    }

    /// Fresh id that no stored workspace uses.
    pub(crate) fn new_id(&self) -> String {
        let base = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut n = 0u32;
        loop {
            let id = format!("ws-{base:x}-{n}");
            if !self.workspaces.iter().any(|w| w.id == id) {
                return id;
            }
            n += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_support::app_with_session;

    fn session(cwd: &str, blob: &str) -> Session {
        let mut s = app_with_session().sessions.remove(0);
        s.cwd = PathBuf::from(cwd);
        s.folder = Path::new(cwd)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        s.search_blob = blob.to_string();
        s
    }

    #[test]
    fn includes_require_every_word_and_excludes_reject_any_word() {
        let mut ws = Workspace::new("a".into(), "A".into());
        let both = session("/p/api", "deploy release notes");
        let one = session("/p/api", "deploy only");
        ws.includes = "Deploy release".into();
        assert!(ws.matches(&both));
        assert!(!ws.matches(&one));

        ws.includes.clear();
        ws.excludes = "nothing notes".into();
        assert!(!ws.matches(&both));
        assert!(ws.matches(&one));
    }

    #[test]
    fn folders_compare_full_paths_not_basenames() {
        let mut ws = Workspace::new("a".into(), "A".into());
        ws.folders = vec![PathBuf::from("/a/api")];
        assert!(ws.matches(&session("/a/api", "x")));
        assert!(!ws.matches(&session("/b/api", "x")));
        ws.toggle_folder(Path::new("/a/api"));
        assert!(ws.folders.is_empty());
        assert!(ws.matches(&session("/b/api", "x")));
    }

    #[test]
    fn names_are_unique_case_insensitively_and_all_is_reserved() {
        let mut store = WorkspaceStore::default();
        store
            .workspaces
            .push(Workspace::new("1".into(), "Api".into()));
        assert!(!store.name_available(" api ", None));
        assert!(store.name_available("api", Some(0)));
        assert!(!store.name_available("ALL", None));
        assert_eq!(store.next_new_name(), NEW_WORKSPACE_NAME);
        store
            .workspaces
            .push(Workspace::new("2".into(), NEW_WORKSPACE_NAME.into()));
        assert_eq!(store.next_new_name(), format!("{NEW_WORKSPACE_NAME} 2"));
    }

    #[test]
    fn workspaces_are_ordered_by_name_as_text() {
        let root = crate::ui::test_support::TempBookmarkStore::new();
        let path = root.path.with_file_name("workspaces.json");
        let mut store = WorkspaceStore::default();
        for (id, name) in [
            ("1", "\u{D55C}\u{AE00}"),
            ("2", "beta"),
            ("3", "Alpha"),
            ("4", "10x"),
        ] {
            store
                .workspaces
                .push(Workspace::new(id.into(), name.into()));
        }
        store.save(&path).unwrap();
        let names = |store: &WorkspaceStore| -> Vec<String> {
            store.workspaces.iter().map(|w| w.name.clone()).collect()
        };
        let loaded = WorkspaceStore::load(&path).unwrap();
        assert_eq!(names(&loaded), ["10x", "Alpha", "beta", "\u{D55C}\u{AE00}"]);

        let mut loaded = loaded;
        loaded.upsert(Workspace::new("5".into(), "Gamma".into()));
        let mut renamed = loaded.workspaces[0].clone();
        renamed.name = "zeta".into();
        loaded.upsert(renamed);
        assert_eq!(
            names(&loaded),
            ["Alpha", "beta", "Gamma", "zeta", "\u{D55C}\u{AE00}"]
        );
    }

    #[test]
    fn store_round_trips_and_drops_a_dangling_active_id() {
        let root = crate::ui::test_support::TempBookmarkStore::new();
        let path = root.path.with_file_name("workspaces.json");
        let mut store = WorkspaceStore::default();
        let mut ws = Workspace::new(store.new_id(), "Api".into());
        ws.includes = "deploy".into();
        ws.folders = vec![PathBuf::from("/a/api")];
        store.workspaces.push(ws);
        store.set_active(Some(0));
        store.save(&path).unwrap();
        assert_eq!(WorkspaceStore::load(&path).unwrap(), store);

        store.active = Some("missing".into());
        store.save(&path).unwrap();
        assert_eq!(WorkspaceStore::load(&path).unwrap().active, None);
    }

    #[test]
    fn a_start_opens_all_whatever_was_open_last() {
        let root = crate::ui::test_support::TempBookmarkStore::new();
        let path = root.path.with_file_name("workspaces.json");
        let mut store = WorkspaceStore::default();
        store
            .workspaces
            .push(Workspace::new(store.new_id(), "Api".into()));
        store.set_active(Some(0));
        store.save(&path).unwrap();

        let started = WorkspaceStore::load_at_startup(&path).unwrap();
        assert_eq!(started.active, None);
        assert_eq!(started.workspaces, store.workspaces);
    }

    #[test]
    fn commit_applies_only_this_change_onto_another_instances_file() {
        let root = crate::ui::test_support::TempBookmarkStore::new();
        let path = root.path.with_file_name("workspaces.json");
        // Another instance saved "Api" after this one started with an empty list.
        let mut other = WorkspaceStore::default();
        other
            .workspaces
            .push(Workspace::new("api".into(), "Api".into()));
        other.save(&path).unwrap();

        let web = Workspace::new("web".into(), "Web".into());
        WorkspaceStore::commit(
            &path,
            &[
                WorkspaceChange::Upsert(web.clone()),
                WorkspaceChange::Opened(Some("web".into())),
            ],
        )
        .unwrap();
        let stored = WorkspaceStore::load(&path).unwrap();
        let names: Vec<&str> = stored.workspaces.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["Api", "Web"]);
        assert_eq!(stored.active.as_deref(), Some("web"));

        let mut renamed = web;
        renamed.name = "Frontend".into();
        WorkspaceStore::commit(
            &path,
            &[
                WorkspaceChange::Upsert(renamed),
                WorkspaceChange::Remove("api".into()),
            ],
        )
        .unwrap();
        let stored = WorkspaceStore::load(&path).unwrap();
        let names: Vec<&str> = stored.workspaces.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["Frontend"]);
    }

    #[test]
    fn commit_refuses_a_name_another_instance_took() {
        let root = crate::ui::test_support::TempBookmarkStore::new();
        let path = root.path.with_file_name("workspaces.json");
        let mut other = WorkspaceStore::default();
        other
            .workspaces
            .push(Workspace::new("theirs".into(), "Api".into()));
        other.save(&path).unwrap();

        let mine = Workspace::new("mine".into(), "api".into());
        let err = WorkspaceStore::commit(&path, &[WorkspaceChange::Upsert(mine)]).unwrap_err();
        assert!(err.to_string().contains("already exists"));
        assert_eq!(WorkspaceStore::load(&path).unwrap(), other);
    }

    #[test]
    fn commit_never_overwrites_an_unreadable_store() {
        let root = crate::ui::test_support::TempBookmarkStore::new();
        let path = root.path.with_file_name("workspaces.json");
        fs::write(&path, "not json").unwrap();
        let change = WorkspaceChange::Opened(None);
        assert!(WorkspaceStore::commit(&path, &[change]).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "not json");
    }

    #[test]
    fn unreadable_or_newer_store_fails_to_load() {
        let root = crate::ui::test_support::TempBookmarkStore::new();
        let path = root.path.with_file_name("workspaces.json");
        for data in ["not json", r#"{"version":999,"workspaces":[]}"#] {
            fs::write(&path, data).unwrap();
            assert!(WorkspaceStore::load(&path).is_err());
        }
    }
}
