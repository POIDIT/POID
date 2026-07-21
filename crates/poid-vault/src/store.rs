//! The on-disk vault store (desktop): one file per instance, written
//! atomically.
//!
//! Layout under the store root:
//!
//! ```text
//! <root>/instances/<uuid>.automerge   one CRDT document per instance
//! <root>/index.json                   the instance index (see `index`)
//! ```
//!
//! Every persist is temp → fsync → rename: a crash mid-write leaves the
//! previous document intact, never a torn file. The full document is written
//! each flush — vault entries are bounded by the quota (default 64 MB) and
//! flushes are debounced by the caller; an append-only incremental log is a
//! later optimization, not a correctness need.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::doc::InstanceDoc;
use crate::error::{Result, VaultError};

/// The default per-POID quota when the manifest does not set
/// `storage.quota_mb` (SPEC §3.1): 64 MB.
pub const DEFAULT_QUOTA_BYTES: u64 = 64 * 1024 * 1024;

/// A vault store rooted at a directory.
pub struct Vault {
    root: PathBuf,
}

impl Vault {
    /// Opens (creating if needed) a vault store at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join("instances"))?;
        Ok(Self { root })
    }

    /// The store's root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The path of one instance's document file.
    pub fn instance_path(&self, id: Uuid) -> PathBuf {
        self.root.join("instances").join(format!("{id}.automerge"))
    }

    /// Loads an instance's document, or starts an empty one if it has never
    /// stored anything.
    pub fn instance(&self, id: Uuid, quota_bytes: u64) -> Result<VaultInstance> {
        let path = self.instance_path(id);
        let doc = if path.exists() {
            InstanceDoc::load(&fs::read(&path)?)?
        } else {
            InstanceDoc::new()
        };
        Ok(VaultInstance {
            id,
            doc,
            path,
            quota_bytes,
        })
    }

    /// Removes an instance's document and its SQL blobs (Fork rollback,
    /// Duplicate-as-empty of the source, uninstall cleanup). Removing a
    /// missing entry is a no-op.
    pub fn remove_instance(&self, id: Uuid) -> Result<()> {
        let path = self.instance_path(id);
        if path.exists() {
            fs::remove_file(path)?;
        }
        self.sql_remove_instance(id)
    }
}

/// One instance's open document plus its persistence location and quota.
pub struct VaultInstance {
    id: Uuid,
    doc: InstanceDoc,
    path: PathBuf,
    quota_bytes: u64,
}

impl VaultInstance {
    /// The instance id this document belongs to.
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// The underlying CRDT document (all read operations).
    pub fn doc(&self) -> &InstanceDoc {
        &self.doc
    }

    /// The underlying CRDT document, mutably — for slot switching and other
    /// reader-side operations that the quota does not apply to.
    pub fn doc_mut(&mut self) -> &mut InstanceDoc {
        &mut self.doc
    }

    /// Sets a value, enforcing the per-POID quota across all slots. The
    /// rejected write leaves the document untouched.
    pub fn kv_set(&mut self, slot: &str, key: &str, value: &serde_json::Value) -> Result<()> {
        let raw_len = serde_json::to_string(value)
            .map_err(|e| VaultError::InvalidValue {
                message: e.to_string(),
            })?
            .len() as u64;
        let current_entry = self
            .doc
            .kv_get(slot, key)
            .ok()
            .flatten()
            .and_then(|v| serde_json::to_string(&v).ok())
            .map(|s| key.len() as u64 + s.len() as u64)
            .unwrap_or(0);
        let projected = self.doc.total_usage() - current_entry + key.len() as u64 + raw_len;
        if projected > self.quota_bytes {
            return Err(VaultError::QuotaExceeded {
                projected,
                limit: self.quota_bytes,
            });
        }
        self.doc.kv_set(slot, key, value)
    }

    /// Merges another replica's bytes (two-device reconciliation). Merges are
    /// exempt from the quota check: refusing a merge would drop the other
    /// device's writes, which is exactly what a CRDT must never do.
    pub fn merge_bytes(&mut self, other: &[u8]) -> Result<()> {
        self.doc.merge_bytes(other)
    }

    /// Persists the document atomically (temp → fsync → rename).
    pub fn flush(&mut self) -> Result<()> {
        atomic_write(&self.path, &self.doc.save())
    }
}

/// Writes `bytes` to `path` atomically: a temp file in the same directory is
/// fsynced and then renamed over the target, so the target is always either
/// the old content or the new — never a prefix of the new.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or_else(|| {
        VaultError::Io(std::io::Error::other("target path has no parent directory"))
    })?;
    fs::create_dir_all(dir)?;
    let tmp = temp_path_in(dir);
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        // The handle must be closed before the rename: Windows refuses to
        // rename an open file. `fs::rename` then replaces the target on both
        // Unix and Windows (MOVEFILE_REPLACE_EXISTING).
        drop(file);
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn temp_path_in(dir: &Path) -> PathBuf {
    // A private counter is not enough across processes; mix in the pid.
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    dir.join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg(test)]
mod tests {
    use super::{Vault, DEFAULT_QUOTA_BYTES};
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn persists_and_reloads_an_instance() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let id = Uuid::new_v4();

        let mut inst = vault.instance(id, DEFAULT_QUOTA_BYTES).unwrap();
        inst.kv_set("", "note", &json!("remember me")).unwrap();
        inst.flush().unwrap();

        let reloaded = vault.instance(id, DEFAULT_QUOTA_BYTES).unwrap();
        assert_eq!(
            reloaded.doc().kv_get("", "note").unwrap(),
            Some(json!("remember me"))
        );
    }

    #[test]
    fn quota_rejects_the_write_that_crosses_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let mut inst = vault.instance(Uuid::new_v4(), 100).unwrap();

        inst.kv_set("", "a", &json!("0123456789012345678901234567890123456789"))
            .unwrap();
        let err = inst
            .kv_set(
                "",
                "b",
                &json!("0123456789012345678901234567890123456789012345678901234567890"),
            )
            .unwrap_err();
        assert!(matches!(
            err,
            crate::error::VaultError::QuotaExceeded { .. }
        ));
        // The rejected write left the store untouched.
        assert_eq!(inst.doc().kv_get("", "b").unwrap(), None);
    }

    #[test]
    fn overwriting_a_key_counts_the_replacement_not_the_sum() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let mut inst = vault.instance(Uuid::new_v4(), 60).unwrap();
        let big = json!("0123456789012345678901234567890123456789");
        inst.kv_set("", "k", &big).unwrap();
        // Overwriting the same key with the same-size value fits: the old
        // entry's bytes are reclaimed in the projection.
        inst.kv_set("", "k", &big).unwrap();
    }

    #[test]
    fn two_stores_merge_like_two_devices() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();

        let vault_a = Vault::open(dir_a.path()).unwrap();
        let vault_b = Vault::open(dir_b.path()).unwrap();

        let mut a = vault_a.instance(id, DEFAULT_QUOTA_BYTES).unwrap();
        a.kv_set("", "seed", &json!(1)).unwrap();
        a.flush().unwrap();

        // "Device B" starts from A's synced state.
        std::fs::copy(vault_a.instance_path(id), vault_b.instance_path(id)).unwrap();
        let mut b = vault_b.instance(id, DEFAULT_QUOTA_BYTES).unwrap();

        a.kv_set("", "from-a", &json!("alpha")).unwrap();
        b.kv_set("", "from-b", &json!("beta")).unwrap();
        a.flush().unwrap();
        b.flush().unwrap();

        // Merge B's file into A — offline reconciliation, no server.
        let b_bytes = std::fs::read(vault_b.instance_path(id)).unwrap();
        a.merge_bytes(&b_bytes).unwrap();
        assert_eq!(a.doc().kv_get("", "from-a").unwrap(), Some(json!("alpha")));
        assert_eq!(a.doc().kv_get("", "from-b").unwrap(), Some(json!("beta")));
    }

    #[test]
    fn atomic_write_replaces_existing_content() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("f.bin");
        super::atomic_write(&target, b"one").unwrap();
        super::atomic_write(&target, b"two").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"two");
        // No temp litter left behind.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-"))
            .collect();
        assert!(leftovers.is_empty());
    }
}
