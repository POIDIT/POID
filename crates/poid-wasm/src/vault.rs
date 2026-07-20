//! wasm-bindgen bindings over `poid-vault` for the Web Reader.
//!
//! The same CRDT engine as the desktop — one storage engine, all platforms
//! (SPEC §6.5). The binding is byte-oriented: the Web Reader persists
//! [`VaultDoc::save`]'s bytes in IndexedDB and hands them back to
//! [`VaultDoc::load`]; merging two replicas is [`VaultDoc::merge`]. Values
//! cross the boundary as JSON strings — the TypeScript side parses them, so
//! no structured-clone layer can drift from the desktop's semantics.

use wasm_bindgen::prelude::*;

use crate::js_error;
use poid_vault::InstanceDoc;

/// One instance's CRDT memory (an Automerge document), scriptable from JS.
#[wasm_bindgen]
pub struct VaultDoc {
    doc: InstanceDoc,
}

#[wasm_bindgen]
impl VaultDoc {
    /// A fresh, empty document.
    #[wasm_bindgen(constructor)]
    pub fn new() -> VaultDoc {
        VaultDoc {
            doc: InstanceDoc::new(),
        }
    }

    /// Loads a document from previously saved bytes.
    pub fn load(bytes: &[u8]) -> Result<VaultDoc, JsValue> {
        Ok(VaultDoc {
            doc: InstanceDoc::load(bytes).map_err(js_error)?,
        })
    }

    /// Serializes the document — the unit IndexedDB persists.
    pub fn save(&mut self) -> Vec<u8> {
        self.doc.save()
    }

    /// Merges another replica's bytes into this document (SPEC §6.5).
    pub fn merge(&mut self, other: &[u8]) -> Result<(), JsValue> {
        self.doc.merge_bytes(other).map_err(js_error)
    }

    /// The value under `key` in `slot` as a JSON string, or `undefined`.
    #[wasm_bindgen(js_name = kvGet)]
    pub fn kv_get(&self, slot: &str, key: &str) -> Result<Option<String>, JsValue> {
        let value = self.doc.kv_get(slot, key).map_err(js_error)?;
        Ok(value.map(|v| v.to_string()))
    }

    /// Sets `key` in `slot` to a JSON-encoded value (atomic replacement).
    #[wasm_bindgen(js_name = kvSet)]
    pub fn kv_set(&mut self, slot: &str, key: &str, json: &str) -> Result<(), JsValue> {
        let value: serde_json::Value = serde_json::from_str(json).map_err(|e| {
            js_error(poid_vault::VaultError::InvalidValue {
                message: e.to_string(),
            })
        })?;
        self.doc.kv_set(slot, key, &value).map_err(js_error)
    }

    /// Deletes `key` from `slot`.
    #[wasm_bindgen(js_name = kvDelete)]
    pub fn kv_delete(&mut self, slot: &str, key: &str) -> Result<(), JsValue> {
        self.doc.kv_delete(slot, key).map_err(js_error)
    }

    /// Sorted keys in `slot`, optionally filtered by prefix.
    #[wasm_bindgen(js_name = kvList)]
    pub fn kv_list(&self, slot: &str, prefix: Option<String>) -> Vec<String> {
        self.doc.kv_list(slot, prefix.as_deref())
    }

    /// Removes every key in `slot`.
    #[wasm_bindgen(js_name = kvClear)]
    pub fn kv_clear(&mut self, slot: &str) -> Result<(), JsValue> {
        self.doc.kv_clear(slot).map_err(js_error)
    }

    /// Bytes stored in `slot` (quota accounting).
    pub fn usage(&self, slot: &str) -> f64 {
        self.doc.usage(slot) as f64
    }

    /// Bytes stored across every slot.
    #[wasm_bindgen(js_name = totalUsage)]
    pub fn total_usage(&self) -> f64 {
        self.doc.total_usage() as f64
    }

    /// Every slot that has stored data, sorted.
    pub fn slots(&self) -> Vec<String> {
        self.doc.slots()
    }

    /// The active slot per the `current` pointer (SPEC §6.4).
    #[wasm_bindgen(js_name = currentSlot)]
    pub fn current_slot(&self) -> String {
        self.doc.current_slot()
    }

    /// Moves the `current` pointer — a reader operation, never the app's.
    #[wasm_bindgen(js_name = setCurrentSlot)]
    pub fn set_current_slot(&mut self, slot: &str) -> Result<(), JsValue> {
        self.doc.set_current_slot(slot).map_err(js_error)
    }
}

impl Default for VaultDoc {
    fn default() -> Self {
        Self::new()
    }
}
