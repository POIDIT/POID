//! The in-memory container: manifest + files, controlled mutation, atomic save.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::error::PoidError;
use crate::integrity::tree_digest;
use crate::manifest::{ExtraFields, Instance, Manifest, Storage, StorageMode};
use crate::write::{pack, PoidBuilder};

/// Path of the embedded application state (SPEC §6.2).
const DATA_STORE: &str = "data/store.json";
/// Path of the encrypted embedded state when `storage.protected` (SPEC §9.2).
const DATA_STORE_ENC: &str = "data/store.enc";

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

    /// Signs the application content with an Ed25519 private key seed
    /// (SPEC §9.3), writing `signature/signature.json`. Integrity digests are
    /// refreshed first, so the signature always covers the current content.
    pub fn sign(&mut self, private_key_seed: &[u8; 32]) -> Result<(), PoidError> {
        crate::integrity::refresh(&mut self.manifest, &self.files);
        let block = crate::signature::sign_manifest(&self.manifest, private_key_seed)?;
        let bytes =
            serde_json::to_vec_pretty(&block).map_err(|e| PoidError::SignatureMalformed {
                reason: format!("block serialization: {e}"),
            })?;
        self.files
            .insert(crate::signature::SIGNATURE_PATH.to_owned(), bytes);
        Ok(())
    }

    /// Checks the container's signature (SPEC §9.3). `Unsigned` is a normal
    /// state, not an error; a malformed signature file is an error.
    pub fn signature_status(&self) -> Result<crate::SignatureStatus, PoidError> {
        match self.files.get(crate::signature::SIGNATURE_PATH) {
            None => Ok(crate::SignatureStatus::Unsigned),
            Some(bytes) => crate::signature::verify_block(&self.manifest, bytes),
        }
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

    /// Replaces one named slot's embedded state, `slots/<name>/store.json`
    /// (SPEC §6.4). The slot name becomes a container path segment, so it
    /// must be a plain name — no separators, no traversal.
    pub fn set_slot_data(&mut self, slot: &str, data: &[u8]) -> Result<(), PoidError> {
        if slot.is_empty()
            || slot.chars().any(|c| c == '/' || c == '\\' || c == '\0')
            || slot == "."
            || slot == ".."
            || slot == "current"
        {
            return Err(PoidError::InvalidPath {
                path: format!("slots/{slot}"),
                why: "slot names must be plain names (no separators, not `current`)",
            });
        }
        self.files
            .insert(format!("slots/{slot}/store.json"), data.to_vec());
        Ok(())
    }

    /// Writes the `slots/current` pointer (SPEC §6.4): the name of the
    /// active slot, as plain UTF-8.
    pub fn set_current_slot_pointer(&mut self, slot: &str) {
        self.files
            .insert("slots/current".to_owned(), slot.as_bytes().to_vec());
    }

    /// The encrypted embedded blob (`data/store.enc`), if present (SPEC §9.2).
    pub fn protected_blob(&self) -> Option<&[u8]> {
        self.file(DATA_STORE_ENC)
    }

    /// Whether `storage.protected` is set (SPEC §9.2).
    pub fn is_protected(&self) -> bool {
        self.manifest
            .storage
            .as_ref()
            .and_then(|s| s.protected)
            .unwrap_or(false)
    }

    /// Turns on `protected` (SPEC §9.2): stores the encrypted `envelope` as
    /// `data/store.enc`, removes the plaintext `data/store.json`, and sets the
    /// manifest flag. The caller performs the encryption (this crate holds no
    /// key material and no randomness).
    pub fn set_protected_blob(&mut self, envelope: &[u8]) {
        self.files.remove(DATA_STORE);
        self.files
            .insert(DATA_STORE_ENC.to_owned(), envelope.to_vec());
        self.storage_mut().protected = Some(true);
    }

    /// Turns off `protected`: stores decrypted `plaintext` as
    /// `data/store.json`, removes `data/store.enc`, and clears the flag.
    pub fn set_plain_data(&mut self, plaintext: &[u8]) {
        self.files.remove(DATA_STORE_ENC);
        self.files.insert(DATA_STORE.to_owned(), plaintext.to_vec());
        self.storage_mut().protected = Some(false);
    }

    fn storage_mut(&mut self) -> &mut Storage {
        self.manifest.storage.get_or_insert_with(|| Storage {
            mode: StorageMode::Embedded,
            slots: None,
            protected: None,
            quota_mb: None,
            requires: None,
            extra: ExtraFields::new(),
        })
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
        let storage = self.storage_mut();
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
