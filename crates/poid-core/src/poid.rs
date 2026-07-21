//! The in-memory container: manifest + files, controlled mutation, atomic save.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::error::PoidError;
use crate::integrity::tree_digest;
use crate::manifest::{
    ExtraFields, FilesystemAccess, Instance, Manifest, Permissions, Storage, StorageMode,
};
use crate::write::{pack, PoidBuilder};

/// Path of the embedded application state (SPEC §6.2).
const DATA_STORE: &str = "data/store.json";
/// Path of the encrypted embedded state when `storage.protected` (SPEC §9.2).
const DATA_STORE_ENC: &str = "data/store.enc";
/// Path of the embedded SQL state (M10): a human-readable SQL text dump —
/// the same archival rationale as `data/store.json` (SPEC §6.2).
const DATA_SQL: &str = "data/database.sql";

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

    /// Embedded SQL state (`data/database.sql`), if present: a UTF-8 SQL
    /// text dump the reader executes into a fresh database on first open.
    pub fn sql_data(&self) -> Option<&[u8]> {
        self.file(DATA_SQL)
    }

    /// Replaces the embedded SQL state (see [`Poid::sql_data`]).
    pub fn set_sql_data(&mut self, dump: &[u8]) {
        self.files.insert(DATA_SQL.to_owned(), dump.to_vec());
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
            schema_version: None,
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

    /// Updates this container's **program** in place from `newer`, preserving
    /// the user's **data** and identity — the "Update program, keep data"
    /// flow (SPEC §12). Both containers MUST be `type: app` and share the same
    /// `app.id`.
    ///
    /// Swapped (the program, from `newer`): `app/`, `deps/`, `migrations/`,
    /// `signature/`, and every program-describing manifest field (`app`,
    /// `runtime`, `entry`, `permissions`, and the app-declared `storage`
    /// fields `quota_mb` / `schema_version`).
    ///
    /// Preserved (the instance's data, from `self`): `data/`, `slots/`,
    /// `instance.id`, and the user's data-placement choices
    /// `storage.mode` / `storage.protected` (which the reader may have
    /// converted, SPEC §6.1 / §9.2). `storage.slots` stays enabled if the
    /// user turned it on.
    ///
    /// The returned [`UpdateReport`] tells the reader whether permissions
    /// widened (re-request consent) and whether the schema version advanced
    /// (apply `migrations/` on next open).
    pub fn update_program(&mut self, newer: &Poid) -> Result<UpdateReport, PoidError> {
        let old_app = self
            .manifest
            .app
            .as_ref()
            .ok_or_else(|| PoidError::UpdateMismatch {
                why: "the file being updated is not an application".to_owned(),
            })?;
        let new_app = newer
            .manifest
            .app
            .as_ref()
            .ok_or_else(|| PoidError::UpdateMismatch {
                why: "the update source is not an application".to_owned(),
            })?;
        if old_app.id != new_app.id {
            return Err(PoidError::UpdateMismatch {
                why: format!(
                    "app.id differs: `{}` cannot be updated by `{}`",
                    old_app.id, new_app.id
                ),
            });
        }

        let permissions_widened = permissions_widened(
            self.manifest.permissions.as_ref(),
            newer.manifest.permissions.as_ref(),
        );
        let old_schema_version = schema_version_of(&self.manifest);
        let new_schema_version = schema_version_of(&newer.manifest);

        // Preserve the instance's data-placement decisions before overwriting.
        let preserved_instance = self.manifest.instance.clone();
        let preserved_mode = self.manifest.storage.as_ref().map(|s| s.mode);
        let preserved_protected = self.manifest.storage.as_ref().and_then(|s| s.protected);
        let user_enabled_slots = self.manifest.storage.as_ref().and_then(|s| s.slots) == Some(true);

        // Swap program files: keep only the data trees, then copy in the new
        // program's files (which never include the data trees of a packed
        // app, nor the generated manifest.json / mimetype).
        self.files
            .retain(|k, _| k.starts_with("data/") || k.starts_with("slots/"));
        for (path, bytes) in &newer.files {
            if path.starts_with("data/") || path.starts_with("slots/") {
                continue;
            }
            self.files.insert(path.clone(), bytes.clone());
        }

        // Take the new manifest wholesale, then restore preserved fields.
        let mut manifest = newer.manifest.clone();
        manifest.instance = preserved_instance;
        if let Some(storage) = &mut manifest.storage {
            if let Some(mode) = preserved_mode {
                storage.mode = mode;
            }
            storage.protected = preserved_protected;
            if user_enabled_slots {
                storage.slots = Some(true);
            }
        }
        self.manifest = manifest;

        // The digests must match the swapped app/ and deps/ trees. The
        // signature payload excludes instance and storage (SPEC §9.3.2), so
        // `newer`'s signature — copied above — stays valid over the result.
        crate::integrity::refresh(&mut self.manifest, &self.files);

        Ok(UpdateReport {
            permissions_widened,
            old_schema_version,
            new_schema_version,
        })
    }

    fn remove_state_trees(&mut self) {
        self.files
            .retain(|k, _| !k.starts_with("data/") && !k.starts_with("slots/"));
    }
}

/// What an "update program, keep data" (SPEC §12) changed that the reader
/// must react to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateReport {
    /// The new program requests a permission the old one did not; the reader
    /// MUST re-request consent before running it (SPEC §12 step 4).
    pub permissions_widened: bool,
    /// The data's schema version before the update.
    pub old_schema_version: u32,
    /// The program's schema version after the update. When greater than
    /// `old_schema_version`, the reader applies `migrations/` on next open
    /// (SPEC §12 step 3).
    pub new_schema_version: u32,
}

impl UpdateReport {
    /// Whether the schema version advanced (migrations will run on next open).
    pub fn schema_advanced(&self) -> bool {
        self.new_schema_version > self.old_schema_version
    }
}

fn schema_version_of(manifest: &Manifest) -> u32 {
    manifest
        .storage
        .as_ref()
        .and_then(|s| s.schema_version)
        .unwrap_or(0)
}

/// True if `newer` requests any capability `old` did not — a superset check
/// across every permission axis (SPEC §12 step 4). Absent = the empty/most-
/// restrictive value, so adding an axis counts as widening.
fn permissions_widened(old: Option<&Permissions>, newer: Option<&Permissions>) -> bool {
    let Some(newer) = newer else {
        return false;
    };
    let empty = Permissions::default();
    let old = old.unwrap_or(&empty);

    let old_net = old.network.as_deref().unwrap_or(&[]);
    let new_net = newer.network.as_deref().unwrap_or(&[]);
    if new_net.iter().any(|o| !old_net.contains(o)) {
        return true;
    }
    let old_mcp = old.mcp.as_deref().unwrap_or(&[]);
    let new_mcp = newer.mcp.as_deref().unwrap_or(&[]);
    if new_mcp.iter().any(|m| !old_mcp.contains(m)) {
        return true;
    }
    // Filesystem widens only none -> user-initiated.
    let fs_widened = matches!(newer.filesystem, Some(FilesystemAccess::UserInitiated))
        && !matches!(old.filesystem, Some(FilesystemAccess::UserInitiated));
    let flag_widened = |old_flag: Option<bool>, new_flag: Option<bool>| {
        new_flag == Some(true) && old_flag != Some(true)
    };
    fs_widened
        || flag_widened(old.clipboard, newer.clipboard)
        || flag_widened(old.print, newer.print)
        || flag_widened(old.notifications, newer.notifications)
}

#[cfg(test)]
mod update_tests {
    use super::*;
    use crate::manifest::{Permissions, StorageMode};

    /// Builds an opened app POID: given version, schema_version, one app file,
    /// and permissions. Packed and reopened so it is a realistic container.
    fn app(
        version: &str,
        schema_version: Option<u32>,
        index_html: &str,
        permissions: Permissions,
    ) -> Poid {
        let mut manifest =
            Manifest::new_app("com.example.kanban", "Kanban", version, "app/index.html");
        manifest.permissions = Some(permissions);
        if let Some(storage) = &mut manifest.storage {
            storage.schema_version = schema_version;
        }
        let builder = PoidBuilder::new(manifest)
            .file("app/index.html", index_html.as_bytes().to_vec())
            .unwrap();
        crate::open(&pack(builder).unwrap()).unwrap()
    }

    #[test]
    fn swaps_program_keeps_data_and_identity() {
        let id = Uuid::new_v4();
        let mut v1 = app("1.0.0", Some(1), "<h1>v1</h1>", Permissions::default());
        v1.set_instance_id(id);
        v1.set_data(br#"{"kept":true}"#);

        let v2 = app("2.0.0", Some(2), "<h1>v2</h1>", Permissions::default());
        let report = v1.update_program(&v2).unwrap();

        // Program swapped.
        assert_eq!(v1.file("app/index.html"), Some(&b"<h1>v2</h1>"[..]));
        assert_eq!(v1.manifest().app.as_ref().unwrap().version, "2.0.0");
        // Data and identity kept.
        assert_eq!(v1.data(), Some(&br#"{"kept":true}"#[..]));
        assert_eq!(v1.manifest().instance.as_ref().unwrap().id, Some(id));
        // Report signals a migration is due.
        assert!(report.schema_advanced());
        assert_eq!(
            (report.old_schema_version, report.new_schema_version),
            (1, 2)
        );
        assert!(!report.permissions_widened);
        // The result is still a valid container.
        let bytes = v1.to_bytes().unwrap();
        crate::open(&bytes).unwrap();
    }

    #[test]
    fn preserves_the_users_storage_mode() {
        let mut v1 = app("1.0.0", Some(1), "<h1>v1</h1>", Permissions::default());
        // The user converted this instance to vault (M08).
        v1.convert_storage_mode(StorageMode::Vault);
        let v2 = app("2.0.0", Some(2), "<h1>v2</h1>", Permissions::default());
        v1.update_program(&v2).unwrap();
        assert_eq!(
            v1.manifest().storage.as_ref().unwrap().mode,
            StorageMode::Vault
        );
    }

    #[test]
    fn detects_widened_permissions() {
        let mut v1 = app("1.0.0", Some(1), "x", Permissions::default());
        let widened = Permissions {
            network: Some(vec!["https://api.example.com".to_owned()]),
            ..Permissions::default()
        };
        let v2 = app("2.0.0", Some(1), "y", widened);
        let report = v1.update_program(&v2).unwrap();
        assert!(report.permissions_widened);
        assert!(!report.schema_advanced());
    }

    #[test]
    fn narrowing_permissions_is_not_widening() {
        let broad = Permissions {
            network: Some(vec!["https://a.example".to_owned()]),
            clipboard: Some(true),
            ..Permissions::default()
        };
        let mut v1 = app("1.0.0", Some(1), "x", broad);
        let v2 = app("2.0.0", Some(1), "y", Permissions::default());
        let report = v1.update_program(&v2).unwrap();
        assert!(!report.permissions_widened);
    }

    #[test]
    fn refuses_a_different_app_id() {
        let mut v1 = app("1.0.0", Some(1), "x", Permissions::default());
        let mut other = Manifest::new_app("com.example.other", "Other", "1.0.0", "app/index.html");
        other.permissions = Some(Permissions::default());
        let other = crate::open(
            &pack(
                PoidBuilder::new(other)
                    .file("app/index.html", b"z".to_vec())
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let err = v1.update_program(&other).unwrap_err();
        assert_eq!(err.code(), "update-mismatch");
    }

    #[test]
    fn a_signed_update_stays_valid() {
        // The signature payload excludes instance and storage (SPEC 9.3.2),
        // so preserving them across the swap keeps the new program's
        // signature valid over the updated container.
        let seed = [7u8; 32];
        let id = Uuid::new_v4();
        let mut v1 = app("1.0.0", Some(1), "<h1>v1</h1>", Permissions::default());
        v1.set_instance_id(id);
        v1.set_data(br#"{"kept":1}"#);

        let mut v2 = app("2.0.0", Some(2), "<h1>v2</h1>", Permissions::default());
        v2.sign(&seed).unwrap();
        assert!(matches!(
            v2.signature_status().unwrap(),
            crate::SignatureStatus::Valid { .. }
        ));

        v1.update_program(&v2).unwrap();
        // The copied signature still verifies over the updated container.
        assert!(matches!(
            v1.signature_status().unwrap(),
            crate::SignatureStatus::Valid { .. }
        ));
    }
}
