//! The vault document: one Automerge document per `instance.id`.
//!
//! Schema (SPEC §6.4, §6.5):
//!
//! ```text
//! ROOT ─ "slots"   ─ <slot name> ─ "kv" ─ <key> → JSON-serialized value
//!      └ "current" → active slot name (the §6.4 pointer)
//! ```
//!
//! **Values are atomic.** Each kv value is stored as one serialized JSON
//! string, not as a nested Automerge structure. `poid.db.kv.set` promises
//! atomic replacement; field-level CRDT merging of two concurrent `set`s of
//! the *same* key would invent objects the application never wrote. With
//! atomic values a same-key conflict resolves to one deterministic winner,
//! while writes to *different* keys — the common case, and the two-device
//! DoD — merge losslessly at the map level.
//!
//! This module is platform-pure (no filesystem): the desktop store wraps it
//! with files, the WASM binding hands its bytes to IndexedDB.

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjId, ObjType, ReadDoc, Value, ROOT};

use crate::error::{Result, VaultError};

/// The slot the reader uses when the manifest does not enable slots. Matches
/// the empty scope slot the boundary broker has used since M04.
pub const DEFAULT_SLOT: &str = "";

/// One instance's memory: a CRDT document with slots of atomic kv values.
pub struct InstanceDoc {
    doc: AutoCommit,
}

impl InstanceDoc {
    /// A fresh, empty document.
    pub fn new() -> Self {
        Self {
            doc: AutoCommit::new(),
        }
    }

    /// Loads a document from its serialized bytes.
    pub fn load(bytes: &[u8]) -> Result<Self> {
        Ok(Self {
            doc: AutoCommit::load(bytes)?,
        })
    }

    /// Serializes the full document (the unit the store persists atomically).
    pub fn save(&mut self) -> Vec<u8> {
        self.doc.save()
    }

    /// Merges another replica of this instance's document into this one.
    /// This is the two-device story: both sides' writes survive (SPEC §6.5).
    pub fn merge_bytes(&mut self, other: &[u8]) -> Result<()> {
        let mut other = AutoCommit::load(other)?;
        self.doc.merge(&mut other)?;
        Ok(())
    }

    /// The document's current heads — the sync cursor. Together with
    /// [`Self::changes_since`] this is the operation log M12 will transport.
    pub fn heads(&mut self) -> Vec<automerge::ChangeHash> {
        self.doc.get_heads()
    }

    /// Serialized changes made after `heads` (empty input = full history).
    /// A remote replica applies them with [`Self::apply_changes`].
    pub fn changes_since(&mut self, heads: &[automerge::ChangeHash]) -> Vec<u8> {
        self.doc.save_after(heads)
    }

    /// Applies serialized changes produced by [`Self::changes_since`].
    pub fn apply_changes(&mut self, changes: &[u8]) -> Result<()> {
        self.doc.load_incremental(changes)?;
        Ok(())
    }

    // ---- slots (SPEC §6.4) ----

    /// Names of every slot that has ever stored data, sorted.
    pub fn slots(&self) -> Vec<String> {
        match self.get_map(&ROOT, "slots") {
            Some(slots) => {
                let mut names: Vec<String> = self.doc.keys(&slots).collect();
                names.sort();
                names
            }
            None => Vec::new(),
        }
    }

    /// The active slot per the `current` pointer; the default slot if unset.
    pub fn current_slot(&self) -> String {
        match self.doc.get(&ROOT, "current") {
            Ok(Some((Value::Scalar(s), _))) => s
                .to_str()
                .map(str::to_owned)
                .unwrap_or_else(|| DEFAULT_SLOT.to_owned()),
            _ => DEFAULT_SLOT.to_owned(),
        }
    }

    /// Moves the `current` pointer. Purely a reader operation — the
    /// application can neither call nor observe this (SPEC §6.4).
    pub fn set_current_slot(&mut self, slot: &str) -> Result<()> {
        self.doc.put(&ROOT, "current", slot)?;
        Ok(())
    }

    // ---- kv (the surface the Data Engine substitutes per scope) ----

    /// The value under `key` in `slot`, if any.
    pub fn kv_get(&self, slot: &str, key: &str) -> Result<Option<serde_json::Value>> {
        let Some(kv) = self.kv_map(slot) else {
            return Ok(None);
        };
        match self.doc.get(&kv, key) {
            Ok(Some((Value::Scalar(s), _))) => match s.to_str() {
                Some(raw) => Ok(Some(serde_json::from_str(raw).map_err(|e| {
                    VaultError::Corrupt {
                        message: format!("stored value is not JSON: {e}"),
                    }
                })?)),
                None => Ok(None),
            },
            _ => Ok(None),
        }
    }

    /// Sets `key` to `value` (atomic replacement).
    pub fn kv_set(&mut self, slot: &str, key: &str, value: &serde_json::Value) -> Result<()> {
        let raw = serde_json::to_string(value).map_err(|e| VaultError::InvalidValue {
            message: e.to_string(),
        })?;
        let kv = self.kv_map_mut(slot)?;
        self.doc.put(&kv, key, raw)?;
        Ok(())
    }

    /// Deletes `key` from `slot`. Deleting a missing key is a no-op.
    pub fn kv_delete(&mut self, slot: &str, key: &str) -> Result<()> {
        if let Some(kv) = self.kv_map(slot) {
            if self.doc.get(&kv, key)?.is_some() {
                self.doc.delete(&kv, key)?;
            }
        }
        Ok(())
    }

    /// Sorted keys in `slot`, optionally filtered by prefix.
    pub fn kv_list(&self, slot: &str, prefix: Option<&str>) -> Vec<String> {
        let Some(kv) = self.kv_map(slot) else {
            return Vec::new();
        };
        let mut keys: Vec<String> = self
            .doc
            .keys(&kv)
            .filter(|k| prefix.is_none_or(|p| k.starts_with(p)))
            .collect();
        keys.sort();
        keys
    }

    /// Removes every key in `slot`.
    pub fn kv_clear(&mut self, slot: &str) -> Result<()> {
        if let Some(kv) = self.kv_map(slot) {
            let keys: Vec<String> = self.doc.keys(&kv).collect();
            for key in keys {
                self.doc.delete(&kv, key.as_str())?;
            }
        }
        Ok(())
    }

    /// Bytes stored in `slot` (keys + serialized values), for quota
    /// accounting. Recomputed by walking — the map holds only scalars, so
    /// this is linear in entry count, and correctness beats a cache that can
    /// drift after merges.
    pub fn usage(&self, slot: &str) -> u64 {
        let Some(kv) = self.kv_map(slot) else {
            return 0;
        };
        self.doc
            .keys(&kv)
            .map(|k| {
                let value_len = match self.doc.get(&kv, k.as_str()) {
                    Ok(Some((Value::Scalar(s), _))) => {
                        s.to_str().map(|v| v.len() as u64).unwrap_or(0)
                    }
                    _ => 0,
                };
                k.len() as u64 + value_len
            })
            .sum()
    }

    /// Bytes stored across every slot — the per-POID quota denominator.
    pub fn total_usage(&self) -> u64 {
        self.slots().iter().map(|s| self.usage(s)).sum()
    }

    // ---- schema plumbing ----

    fn get_map(&self, parent: &ObjId, prop: &str) -> Option<ObjId> {
        match self.doc.get(parent, prop) {
            Ok(Some((Value::Object(ObjType::Map), id))) => Some(id),
            _ => None,
        }
    }

    fn kv_map(&self, slot: &str) -> Option<ObjId> {
        let slots = self.get_map(&ROOT, "slots")?;
        let slot = self.get_map(&slots, slot)?;
        self.get_map(&slot, "kv")
    }

    fn kv_map_mut(&mut self, slot: &str) -> Result<ObjId> {
        let slots = match self.get_map(&ROOT, "slots") {
            Some(id) => id,
            None => self.doc.put_object(&ROOT, "slots", ObjType::Map)?,
        };
        let slot_obj = match self.get_map(&slots, slot) {
            Some(id) => id,
            None => self.doc.put_object(&slots, slot, ObjType::Map)?,
        };
        Ok(match self.get_map(&slot_obj, "kv") {
            Some(id) => id,
            None => self.doc.put_object(&slot_obj, "kv", ObjType::Map)?,
        })
    }
}

impl Default for InstanceDoc {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{InstanceDoc, DEFAULT_SLOT};
    use serde_json::json;

    #[test]
    fn set_get_roundtrip_preserves_json() {
        let mut doc = InstanceDoc::new();
        doc.kv_set(
            DEFAULT_SLOT,
            "board",
            &json!({"cols": ["todo", "done"], "n": 3}),
        )
        .unwrap();
        assert_eq!(
            doc.kv_get(DEFAULT_SLOT, "board").unwrap(),
            Some(json!({"cols": ["todo", "done"], "n": 3}))
        );
    }

    #[test]
    fn slots_isolate_data() {
        let mut doc = InstanceDoc::new();
        doc.kv_set("work", "k", &json!(1)).unwrap();
        doc.kv_set("home", "k", &json!(2)).unwrap();
        assert_eq!(doc.kv_get("work", "k").unwrap(), Some(json!(1)));
        assert_eq!(doc.kv_get("home", "k").unwrap(), Some(json!(2)));
        assert_eq!(doc.slots(), vec!["home".to_owned(), "work".to_owned()]);
    }

    #[test]
    fn current_pointer_defaults_and_moves() {
        let mut doc = InstanceDoc::new();
        assert_eq!(doc.current_slot(), DEFAULT_SLOT);
        doc.set_current_slot("save-2").unwrap();
        assert_eq!(doc.current_slot(), "save-2");
    }

    #[test]
    fn save_load_roundtrip() {
        let mut doc = InstanceDoc::new();
        doc.kv_set(DEFAULT_SLOT, "x", &json!("hello")).unwrap();
        let bytes = doc.save();
        let restored = InstanceDoc::load(&bytes).unwrap();
        assert_eq!(
            restored.kv_get(DEFAULT_SLOT, "x").unwrap(),
            Some(json!("hello"))
        );
    }

    #[test]
    fn two_devices_merge_without_losing_either_write() {
        // The load-bearing DoD: append on A, append on B, merge → both survive.
        let mut a = InstanceDoc::new();
        a.kv_set(DEFAULT_SLOT, "seed", &json!(true)).unwrap();
        let base = a.save();

        let mut b = InstanceDoc::load(&base).unwrap();
        a.kv_set(DEFAULT_SLOT, "from-a", &json!("alpha")).unwrap();
        b.kv_set(DEFAULT_SLOT, "from-b", &json!("beta")).unwrap();

        let b_bytes = b.save();
        a.merge_bytes(&b_bytes).unwrap();

        assert_eq!(
            a.kv_get(DEFAULT_SLOT, "from-a").unwrap(),
            Some(json!("alpha"))
        );
        assert_eq!(
            a.kv_get(DEFAULT_SLOT, "from-b").unwrap(),
            Some(json!("beta"))
        );
        assert_eq!(a.kv_get(DEFAULT_SLOT, "seed").unwrap(), Some(json!(true)));
    }

    #[test]
    fn same_key_conflict_resolves_to_one_deterministic_winner() {
        // Atomic values: a concurrent same-key set never invents a merged
        // object — one side wins, and both replicas agree on which.
        let mut a = InstanceDoc::new();
        a.kv_set(DEFAULT_SLOT, "seed", &json!(0)).unwrap();
        let base = a.save();
        let mut b = InstanceDoc::load(&base).unwrap();

        a.kv_set(DEFAULT_SLOT, "k", &json!({"from": "a"})).unwrap();
        b.kv_set(DEFAULT_SLOT, "k", &json!({"from": "b"})).unwrap();

        let (a_bytes, b_bytes) = (a.save(), b.save());
        a.merge_bytes(&b_bytes).unwrap();
        b.merge_bytes(&a_bytes).unwrap();

        let winner_a = a.kv_get(DEFAULT_SLOT, "k").unwrap().unwrap();
        let winner_b = b.kv_get(DEFAULT_SLOT, "k").unwrap().unwrap();
        assert_eq!(winner_a, winner_b, "replicas must converge");
        assert!(
            winner_a == json!({"from": "a"}) || winner_a == json!({"from": "b"}),
            "the winner is one of the written values, never an invented merge"
        );
    }

    #[test]
    fn change_log_transports_edits() {
        // The operation-log contract M12's sync will ride on.
        let mut a = InstanceDoc::new();
        a.kv_set(DEFAULT_SLOT, "seed", &json!(1)).unwrap();
        let base = a.save();
        let mut b = InstanceDoc::load(&base).unwrap();

        let cursor = b.heads();
        a.kv_set(DEFAULT_SLOT, "late", &json!("news")).unwrap();
        let delta = a.changes_since(&cursor);
        b.apply_changes(&delta).unwrap();
        assert_eq!(b.kv_get(DEFAULT_SLOT, "late").unwrap(), Some(json!("news")));
    }

    #[test]
    fn usage_counts_keys_and_values() {
        let mut doc = InstanceDoc::new();
        assert_eq!(doc.total_usage(), 0);
        doc.kv_set(DEFAULT_SLOT, "ab", &json!("xy")).unwrap();
        // key "ab" (2) + serialized value "\"xy\"" (4)
        assert_eq!(doc.usage(DEFAULT_SLOT), 6);
        doc.kv_delete(DEFAULT_SLOT, "ab").unwrap();
        assert_eq!(doc.usage(DEFAULT_SLOT), 0);
    }

    #[test]
    fn list_and_clear_scope_to_one_slot() {
        let mut doc = InstanceDoc::new();
        doc.kv_set("a", "k1", &json!(1)).unwrap();
        doc.kv_set("a", "k2", &json!(2)).unwrap();
        doc.kv_set("b", "other", &json!(3)).unwrap();
        assert_eq!(doc.kv_list("a", None), vec!["k1", "k2"]);
        assert_eq!(doc.kv_list("a", Some("k1")), vec!["k1"]);
        doc.kv_clear("a").unwrap();
        assert!(doc.kv_list("a", None).is_empty());
        assert_eq!(doc.kv_get("b", "other").unwrap(), Some(json!(3)));
    }
}
