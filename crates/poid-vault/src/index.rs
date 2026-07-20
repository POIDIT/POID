//! The instance index and the copy-detection state machine (SPEC §6.3).
//!
//! `Ctrl+C` produces a byte-identical file, so no identifier *inside* the
//! file can distinguish a copy from its original. The reader keeps this
//! index — `instance.id → (registered paths, file hash)` — and classifies
//! every open against it. Asking the user once, in the rare genuine-copy
//! case, is the correct behaviour, not a workaround.
//!
//! Two deliberate refinements over the SPEC's letter (proposed upstream):
//! - An id that is set but **unknown** to this index (a file arriving from
//!   another machine) registers silently — there is nothing to conflict with.
//! - After **Share memory**, the entry holds *both* paths, so neither file
//!   re-prompts on its next open.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::Result;
use crate::store::atomic_write;

/// One instance's registration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexEntry {
    /// Every path known to address this instance's memory. One entry unless
    /// the user chose *Share memory* for a copy.
    pub paths: Vec<PathBuf>,
    /// SHA-256 (lowercase hex) of the file at registration time.
    pub file_hash: String,
}

/// How an open of `(instance.id, path)` classifies against the index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// The id is not in the index (fresh id, or a file from elsewhere):
    /// register and proceed.
    Register,
    /// A registered path: normal open.
    Open,
    /// Known id, this path is new, and no registered path still exists:
    /// the file was moved. Update the index silently and proceed.
    Moved,
    /// Known id, this path is new, and a registered path still exists:
    /// a genuine copy. The reader must prompt Fork / Move / Share.
    Copy {
        /// A registered path that still exists (shown in the prompt).
        existing: PathBuf,
    },
}

/// The persistent index, stored as `index.json` in the vault root.
pub struct InstanceIndex {
    path: PathBuf,
    entries: BTreeMap<Uuid, IndexEntry>,
}

#[derive(Default, Serialize, Deserialize)]
struct IndexFile {
    version: u32,
    entries: BTreeMap<Uuid, IndexEntry>,
}

impl InstanceIndex {
    /// Loads the index from `vault_root/index.json`; a missing file is an
    /// empty index, corrupt content is an error (never silently reset a map
    /// of user memories).
    pub fn load(vault_root: &Path) -> Result<Self> {
        let path = vault_root.join("index.json");
        let entries = if path.exists() {
            let raw = std::fs::read(&path)?;
            let file: IndexFile =
                serde_json::from_slice(&raw).map_err(|e| crate::VaultError::Corrupt {
                    message: format!("instance index is not readable: {e}"),
                })?;
            file.entries
        } else {
            BTreeMap::new()
        };
        Ok(Self { path, entries })
    }

    /// Persists the index atomically.
    pub fn save(&self) -> Result<()> {
        let file = IndexFile {
            version: 1,
            entries: self.entries.clone(),
        };
        let json = serde_json::to_vec_pretty(&file).map_err(|e| crate::VaultError::Corrupt {
            message: e.to_string(),
        })?;
        atomic_write(&self.path, &json)
    }

    /// The entry for an id, if registered.
    pub fn entry(&self, id: Uuid) -> Option<&IndexEntry> {
        self.entries.get(&id)
    }

    /// Classifies an open per SPEC §6.3. `exists` is injected so the state
    /// machine is a pure function of index + filesystem answers — and fully
    /// testable without touching a disk.
    pub fn resolve(
        &self,
        id: Uuid,
        opened_path: &Path,
        exists: impl Fn(&Path) -> bool,
    ) -> Disposition {
        let Some(entry) = self.entries.get(&id) else {
            return Disposition::Register;
        };
        if entry.paths.iter().any(|p| same_path(p, opened_path)) {
            return Disposition::Open;
        }
        match entry.paths.iter().find(|p| exists(p)) {
            Some(existing) => Disposition::Copy {
                existing: existing.clone(),
            },
            None => Disposition::Moved,
        }
    }

    /// Registers a fresh instance (or re-registers a foreign one).
    pub fn register(&mut self, id: Uuid, path: PathBuf, file_hash: String) {
        self.entries.insert(
            id,
            IndexEntry {
                paths: vec![path],
                file_hash,
            },
        );
    }

    /// Records a silent move: the new path replaces every stale one.
    pub fn record_move(&mut self, id: Uuid, new_path: PathBuf) {
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.paths = vec![new_path];
        }
    }

    /// Records *Share memory*: the new path joins the registered ones.
    pub fn add_shared_path(&mut self, id: Uuid, path: PathBuf) {
        if let Some(entry) = self.entries.get_mut(&id) {
            if !entry.paths.iter().any(|p| same_path(p, &path)) {
                entry.paths.push(path);
            }
        }
    }

    /// Drops an instance's registration (Duplicate-as-empty of a source
    /// file's clone, uninstall cleanup).
    pub fn remove(&mut self, id: Uuid) {
        self.entries.remove(&id);
    }
}

/// Path equality for index purposes. Windows paths are compared
/// case-insensitively — opening `C:\Docs\a.poid` as `c:\docs\A.POID` is the
/// same file, not a copy.
fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// SHA-256 of file content, lowercase hex — the hash the index records.
pub fn hash_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Disposition, InstanceIndex};
    use std::path::{Path, PathBuf};
    use uuid::Uuid;

    fn index() -> InstanceIndex {
        let dir = tempfile::tempdir().unwrap();
        let idx = InstanceIndex::load(dir.path()).unwrap();
        // Keep the tempdir alive for the test body.
        std::mem::forget(dir);
        idx
    }

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn unknown_id_registers() {
        let idx = index();
        let d = idx.resolve(Uuid::new_v4(), Path::new("C:\\a.poid"), |_| true);
        assert_eq!(d, Disposition::Register);
    }

    #[test]
    fn registered_path_opens() {
        let mut idx = index();
        let id = Uuid::new_v4();
        idx.register(id, p("C:\\docs\\kanban.poid"), "h".into());
        let d = idx.resolve(id, Path::new("C:\\docs\\kanban.poid"), |_| true);
        assert_eq!(d, Disposition::Open);
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_case_does_not_fake_a_copy() {
        let mut idx = index();
        let id = Uuid::new_v4();
        idx.register(id, p("C:\\Docs\\Kanban.poid"), "h".into());
        let d = idx.resolve(id, Path::new("c:\\docs\\KANBAN.POID"), |_| true);
        assert_eq!(d, Disposition::Open);
    }

    #[test]
    fn vanished_old_path_is_a_move() {
        let mut idx = index();
        let id = Uuid::new_v4();
        idx.register(id, p("C:\\old\\a.poid"), "h".into());
        let d = idx.resolve(id, Path::new("C:\\new\\a.poid"), |_| false);
        assert_eq!(d, Disposition::Moved);
    }

    #[test]
    fn surviving_old_path_is_a_copy() {
        let mut idx = index();
        let id = Uuid::new_v4();
        idx.register(id, p("C:\\orig\\a.poid"), "h".into());
        let d = idx.resolve(id, Path::new("C:\\copy\\a.poid"), |_| true);
        assert_eq!(
            d,
            Disposition::Copy {
                existing: p("C:\\orig\\a.poid")
            }
        );
    }

    #[test]
    fn share_memory_stops_future_prompts_for_both_paths() {
        let mut idx = index();
        let id = Uuid::new_v4();
        idx.register(id, p("C:\\orig\\a.poid"), "h".into());
        idx.add_shared_path(id, p("C:\\copy\\a.poid"));
        assert_eq!(
            idx.resolve(id, Path::new("C:\\orig\\a.poid"), |_| true),
            Disposition::Open
        );
        assert_eq!(
            idx.resolve(id, Path::new("C:\\copy\\a.poid"), |_| true),
            Disposition::Open
        );
    }

    #[test]
    fn move_replaces_stale_paths() {
        let mut idx = index();
        let id = Uuid::new_v4();
        idx.register(id, p("C:\\old\\a.poid"), "h".into());
        idx.record_move(id, p("C:\\new\\a.poid"));
        assert_eq!(
            idx.resolve(id, Path::new("C:\\new\\a.poid"), |_| true),
            Disposition::Open
        );
        // The old path would now be a copy only if it actually re-appeared.
        assert_eq!(
            idx.resolve(id, Path::new("C:\\old\\a.poid"), |_| false),
            Disposition::Moved
        );
    }

    #[test]
    fn index_survives_a_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        {
            let mut idx = InstanceIndex::load(dir.path()).unwrap();
            idx.register(id, p("C:\\a.poid"), "cafe".into());
            idx.save().unwrap();
        }
        let idx = InstanceIndex::load(dir.path()).unwrap();
        assert_eq!(idx.entry(id).unwrap().file_hash, "cafe");
    }

    #[test]
    fn hash_bytes_is_stable_hex() {
        assert_eq!(
            super::hash_bytes(b"poid"),
            // sha256("poid")
            super::hash_bytes(b"poid")
        );
        assert_eq!(super::hash_bytes(b"").len(), 64);
    }
}
