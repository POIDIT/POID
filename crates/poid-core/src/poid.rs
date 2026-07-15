//! The in-memory container: manifest + files, controlled mutation, atomic save.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::error::PoidError;
use crate::integrity::tree_digest;
use crate::manifest::{ExtraFields, Instance, Manifest, Storage, StorageMode};
use crate::write::{pack, PoidBuilder};

/// Path of the embedded application state (SPEC §6.2).
const DATA_STORE: &str = "data/store.json";

/// A validated, opened container.
///
/// The manifest is held as a structure and `manifest.json` / `mimetype` are
/// regenerated on every pack, so a stale manifest can never be written back.
/// Mutations are the operations the spec defines (SPEC §6); arbitrary edits
/// go through [`PoidBuilder`].
#[derive(Debug, Clone)]
pub struct Poid {
    manifest: Manifest,
    files: BTreeMap<String, Vec<u8>>,
}

impl Poid {
    pub(crate) fn from_parts(manifest: Manifest, files: BTreeMap<String, Vec<u8>>) -> Self {
        Self { manifest, files }
    }

    /// The parsed manifest.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Content of a file entry, if present.
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    /// All file entries except the generated `mimetype` and `manifest.json`,
    /// sorted by path.
    pub fn files(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.files.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }

    /// Embedded application state (`data/store.json`), if present (SPEC §6.2).
    pub fn data(&self) -> Option<&[u8]> {
        self.file(DATA_STORE)
    }

    /// Verifies the stored integrity digests against the content (SPEC §3.3).
    pub fn verify(&self) -> Result<(), PoidError> {
        let (stored_app, stored_deps) = match &self.manifest.integrity {
            Some(i) => (i.app.as_deref(), i.deps.as_deref()),
            None => (None, None),
        };
        if tree_digest(&self.files, "app/").as_deref() != stored_app {
            return Err(PoidError::IntegrityMismatch { tree: "app" });
        }
        if tree_digest(&self.files, "deps/").as_deref() != stored_deps {
            return Err(PoidError::IntegrityMismatch { tree: "deps" });
        }
        Ok(())
    }

    /// Writes the instance identity assigned by the reader on first open, or
    /// after a copy is forked (SPEC §6.3). Generating the UUID is the
    /// caller's job — this crate has no randomness source, so it stays
    /// buildable for `wasm32-unknown-unknown`.
    pub fn set_instance_id(&mut self, id: Uuid) {
        match &mut self.manifest.instance {
            Some(instance) => instance.id = Some(id),
            None => {
                self.manifest.instance = Some(Instance {
                    id: Some(id),
                    extra: ExtraFields::new(),
                });
            }
        }
    }

    /// Replaces the embedded application state (SPEC §6.2).
    pub fn set_data(&mut self, data: &[u8]) {
        self.files.insert(DATA_STORE.to_owned(), data.to_vec());
    }

    /// Implements *"Duplicate as empty"* (SPEC §6.3): clears `data/` and
    /// `slots/`, and resets `instance.id` to `null` so the reader assigns a
    /// fresh identity on next open.
    pub fn clear_data(&mut self) {
        self.remove_state_trees();
        if let Some(instance) = &mut self.manifest.instance {
            instance.id = None;
        }
    }

    /// Switches `storage.mode` (SPEC §6.1). Leaving `embedded` drops the
    /// `data/` and `slots/` trees from the container — extract [`Self::data`]
    /// first if it must migrate into the vault or a connection.
    pub fn convert_storage_mode(&mut self, mode: StorageMode) {
        let storage = self.manifest.storage.get_or_insert_with(|| Storage {
            mode: StorageMode::Embedded,
            slots: None,
            protected: None,
            quota_mb: None,
            requires: None,
            extra: ExtraFields::new(),
        });
        let leaving_embedded =
            storage.mode == StorageMode::Embedded && mode != StorageMode::Embedded;
        storage.mode = mode;
        if leaving_embedded {
            self.remove_state_trees();
        }
    }

    /// Packs the container deterministically (SPEC §2.1); integrity digests
    /// are recomputed on the way out (SPEC §3.3).
    pub fn to_bytes(&self) -> Result<Vec<u8>, PoidError> {
        pack(PoidBuilder {
            manifest: self.manifest.clone(),
            files: self.files.clone(),
        })
    }

    /// Atomically saves the container: write to a temporary file in the same
    /// directory, fsync, then rename over the destination. A crash mid-save
    /// never corrupts the user's file.
    #[cfg(feature = "fs")]
    pub fn save_path(&self, path: &std::path::Path) -> Result<(), PoidError> {
        use std::io::Write;

        let bytes = self.to_bytes()?;
        let dir = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => std::path::Path::new("."),
        };
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(&bytes)?;
        tmp.as_file().sync_all()?;
        tmp.persist(path).map_err(|e| PoidError::Io(e.error))?;
        Ok(())
    }

    fn remove_state_trees(&mut self) {
        self.files
            .retain(|k, _| !k.starts_with("data/") && !k.starts_with("slots/"));
    }
}
