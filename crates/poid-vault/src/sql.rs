//! SQL database blobs (M10): the desktop persistence for `poid.db.sql` /
//! `poid.db.docs`.
//!
//! The reader-side SQL engine (wa-sqlite in the webview) owns SQL semantics
//! and enforces the quota with `PRAGMA max_page_count`; the vault stores the
//! resulting database bytes as opaque blobs — one file per (instance, slot),
//! written with the same temp → fsync → rename discipline as the CRDT
//! documents:
//!
//! ```text
//! <root>/instances/<uuid>.sql/s<hex(slot)>.sqlite
//! ```
//!
//! Slot names are hex-encoded in the file name: a slot name is user-ish data
//! and must never be interpreted by the filesystem. The size check here is a
//! backstop against a compromised webview, not the primary quota.

use std::fs;
use std::path::PathBuf;

use uuid::Uuid;

use crate::error::{Result, VaultError};
use crate::store::{atomic_write, Vault};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The blob file name for a slot (`s<hex>.sqlite`; the empty slot is `s.sqlite`).
fn slot_file(slot: &str) -> String {
    format!("s{}.sqlite", hex(slot.as_bytes()))
}

impl Vault {
    /// The directory holding one instance's SQL blobs.
    pub fn sql_dir(&self, id: Uuid) -> PathBuf {
        self.root().join("instances").join(format!("{id}.sql"))
    }

    /// Loads the SQL database bytes for `(id, slot)`, if any were saved.
    pub fn sql_load(&self, id: Uuid, slot: &str) -> Result<Option<Vec<u8>>> {
        let path = self.sql_dir(id).join(slot_file(slot));
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(fs::read(path)?))
    }

    /// Saves the SQL database bytes for `(id, slot)` atomically.
    ///
    /// `quota_bytes` is the per-POID quota (SPEC `storage.quota_mb`); the
    /// engine enforces it first via `max_page_count`, this check is the
    /// vault's own backstop.
    pub fn sql_save(&self, id: Uuid, slot: &str, bytes: &[u8], quota_bytes: u64) -> Result<()> {
        if bytes.len() as u64 > quota_bytes {
            return Err(VaultError::QuotaExceeded {
                projected: bytes.len() as u64,
                limit: quota_bytes,
            });
        }
        atomic_write(&self.sql_dir(id).join(slot_file(slot)), bytes)
    }

    /// Removes every SQL blob of an instance (Fork rollback,
    /// Duplicate-as-empty, uninstall cleanup). Missing directory is a no-op.
    pub fn sql_remove_instance(&self, id: Uuid) -> Result<()> {
        let dir = self.sql_dir(id);
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::store::{Vault, DEFAULT_QUOTA_BYTES};

    #[test]
    fn round_trips_blobs_per_slot() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let id = Uuid::new_v4();

        assert_eq!(vault.sql_load(id, "").unwrap(), None);
        vault
            .sql_save(id, "", b"default-slot", DEFAULT_QUOTA_BYTES)
            .unwrap();
        vault
            .sql_save(id, "work", b"work-slot", DEFAULT_QUOTA_BYTES)
            .unwrap();

        assert_eq!(
            vault.sql_load(id, "").unwrap(),
            Some(b"default-slot".to_vec())
        );
        assert_eq!(
            vault.sql_load(id, "work").unwrap(),
            Some(b"work-slot".to_vec())
        );
        // Slots are independent files; an unknown slot is empty.
        assert_eq!(vault.sql_load(id, "other").unwrap(), None);
    }

    #[test]
    fn hostile_slot_names_stay_inside_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let id = Uuid::new_v4();

        // Path-traversal-shaped and reserved names are data, not paths.
        for slot in ["../escape", "..", "con", "a/b\\c", "nul.txt"] {
            vault.sql_save(id, slot, b"x", DEFAULT_QUOTA_BYTES).unwrap();
            assert_eq!(vault.sql_load(id, slot).unwrap(), Some(b"x".to_vec()));
        }
        // Everything landed under the instance's sql dir, nothing escaped.
        let entries: Vec<_> = std::fs::read_dir(vault.sql_dir(id))
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 5);
    }

    #[test]
    fn save_replaces_atomically_and_respects_the_backstop_quota() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let id = Uuid::new_v4();

        vault.sql_save(id, "", b"one", DEFAULT_QUOTA_BYTES).unwrap();
        vault.sql_save(id, "", b"two", DEFAULT_QUOTA_BYTES).unwrap();
        assert_eq!(vault.sql_load(id, "").unwrap(), Some(b"two".to_vec()));

        let err = vault.sql_save(id, "", &[0u8; 32], 16).unwrap_err();
        assert!(matches!(err, crate::VaultError::QuotaExceeded { .. }));
        // The rejected write left the previous bytes intact.
        assert_eq!(vault.sql_load(id, "").unwrap(), Some(b"two".to_vec()));
    }

    #[test]
    fn remove_instance_clears_all_slots() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let id = Uuid::new_v4();
        vault.sql_save(id, "", b"a", DEFAULT_QUOTA_BYTES).unwrap();
        vault.sql_save(id, "b", b"b", DEFAULT_QUOTA_BYTES).unwrap();
        vault.sql_remove_instance(id).unwrap();
        assert_eq!(vault.sql_load(id, "").unwrap(), None);
        assert!(!vault.sql_dir(id).exists());
        // Removing again is a no-op.
        vault.sql_remove_instance(id).unwrap();
    }
}
