//! Storage mode conversion (SPEC §6.1): `embedded ↔ vault`, on any machine.
//!
//! The container side is `poid-core` (manifest flip, state trees, atomic
//! save); this module moves the *data* between `data/`+`slots/` in the file
//! and the CRDT document in the vault.
//!
//! Export is canonical — top-level keys sorted, two-space pretty JSON, one
//! trailing newline — so `embedded → vault → embedded` reproduces the store
//! byte for byte (the M08 DoD), and two machines exporting the same state
//! produce the same file.
//!
//! `connection` transitions are a manifest flip plus dropped local state
//! (data never travels with the file, SPEC §6.1); wiring an actual backend
//! lands with the Connections milestone.

use std::collections::BTreeMap;

use poid_core::{Poid, StorageMode};
use uuid::Uuid;

use crate::doc::DEFAULT_SLOT;
use crate::error::{Result, VaultError};
use crate::store::Vault;

/// Moves embedded state into the vault and flips the mode (SPEC §6.1).
///
/// Seeds the default slot from `data/store.json` and every named slot from
/// `slots/<name>/store.json`, honours the `slots/current` pointer, assigns
/// `instance.id` if the file never had one, and drops the state trees from
/// the container. The caller persists the file (`save_path`) afterwards.
///
/// Returns the instance id the vault entry is keyed by.
pub fn embedded_to_vault(poid: &mut Poid, vault: &Vault, quota_bytes: u64) -> Result<Uuid> {
    let id = poid
        .manifest()
        .instance
        .as_ref()
        .and_then(|i| i.id)
        .unwrap_or_else(Uuid::new_v4);

    let mut instance = vault.instance(id, quota_bytes)?;

    if let Some(bytes) = poid.data() {
        seed_slot(&mut instance, DEFAULT_SLOT, bytes)?;
    }
    let named: Vec<(String, Vec<u8>)> = poid
        .files()
        .filter_map(|(path, bytes)| {
            let name = path.strip_prefix("slots/")?.strip_suffix("/store.json")?;
            Some((name.to_owned(), bytes.to_vec()))
        })
        .collect();
    for (name, bytes) in &named {
        seed_slot(&mut instance, name, bytes)?;
    }
    if let Some(current) = poid.file("slots/current") {
        let name = String::from_utf8_lossy(current).trim().to_owned();
        instance.doc_mut().set_current_slot(&name)?;
    }
    instance.flush()?;

    poid.set_instance_id(id);
    poid.convert_storage_mode(StorageMode::Vault);
    Ok(id)
}

/// Pulls vault state back into the container and flips the mode (SPEC §6.1).
///
/// The default slot becomes `data/store.json`; named slots become
/// `slots/<name>/store.json`; a non-default `current` pointer is written to
/// `slots/current`. The vault entry is left in place — converting a file
/// must never destroy the memory other Share-memory files may still address.
pub fn vault_to_embedded(poid: &mut Poid, vault: &Vault, quota_bytes: u64) -> Result<()> {
    let id = poid
        .manifest()
        .instance
        .as_ref()
        .and_then(|i| i.id)
        .ok_or_else(|| VaultError::Corrupt {
            message: "a vault-mode container without instance.id has no vault entry".to_owned(),
        })?;
    let instance = vault.instance(id, quota_bytes)?;
    let doc = instance.doc();

    poid.convert_storage_mode(StorageMode::Embedded);
    if let Some(json) = export_slot(doc, DEFAULT_SLOT)? {
        poid.set_data(&json);
    }
    for slot in doc.slots() {
        if slot == DEFAULT_SLOT {
            continue;
        }
        if let Some(json) = export_slot(doc, &slot)? {
            poid.set_slot_data(&slot, &json)
                .map_err(|e| VaultError::Corrupt {
                    message: e.to_string(),
                })?;
        }
    }
    let current = doc.current_slot();
    if current != DEFAULT_SLOT {
        poid.set_current_slot_pointer(&current);
    }
    Ok(())
}

/// Seeds one vault slot from a `store.json` payload (a JSON object; anything
/// else is refused — the store is a map of keys, SPEC §6.2).
fn seed_slot(instance: &mut crate::store::VaultInstance, slot: &str, bytes: &[u8]) -> Result<()> {
    let parsed: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| VaultError::InvalidValue {
            message: format!("store.json is not valid JSON: {e}"),
        })?;
    let serde_json::Value::Object(map) = parsed else {
        return Err(VaultError::InvalidValue {
            message: "store.json must be a JSON object".to_owned(),
        });
    };
    for (key, value) in map {
        instance.kv_set(slot, &key, &value)?;
    }
    Ok(())
}

/// Canonically serializes one slot: sorted keys, two-space indent, trailing
/// newline. `None` when the slot holds nothing (no file is written).
fn export_slot(doc: &crate::doc::InstanceDoc, slot: &str) -> Result<Option<Vec<u8>>> {
    let keys = doc.kv_list(slot, None);
    if keys.is_empty() {
        return Ok(None);
    }
    let mut map = BTreeMap::new();
    for key in keys {
        if let Some(value) = doc.kv_get(slot, &key)? {
            map.insert(key, value);
        }
    }
    let mut out = serde_json::to_vec_pretty(&map).map_err(|e| VaultError::InvalidValue {
        message: e.to_string(),
    })?;
    out.push(b'\n');
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::{embedded_to_vault, vault_to_embedded};
    use crate::store::{Vault, DEFAULT_QUOTA_BYTES};
    use poid_core::{pack, Manifest, PoidBuilder, StorageMode};

    fn app_with_data(store_json: &[u8]) -> poid_core::Poid {
        let manifest = Manifest::new_app("dev.poid.test", "T", "1.0.0", "app/index.html");
        let builder = PoidBuilder::new(manifest)
            .file("app/index.html", b"<!doctype html>".to_vec())
            .unwrap()
            .file("data/store.json", store_json.to_vec())
            .unwrap();
        let bytes = pack(builder).unwrap();
        poid_core::open(&bytes).unwrap()
    }

    #[test]
    fn embedded_to_vault_to_embedded_is_byte_identical() {
        // The DoD: data is intact and byte-identical after the round trip.
        // The input is in the canonical form the exporter itself writes.
        let canonical = b"{\n  \"count\": 3,\n  \"title\": \"notes\"\n}\n";
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();

        let mut poid = app_with_data(canonical);
        let id = embedded_to_vault(&mut poid, &vault, DEFAULT_QUOTA_BYTES).unwrap();

        // Leaving embedded dropped the tree and keyed the vault entry.
        assert!(poid.data().is_none());
        assert_eq!(
            poid.manifest().storage.as_ref().unwrap().mode,
            StorageMode::Vault
        );
        assert_eq!(poid.manifest().instance.as_ref().unwrap().id, Some(id));

        vault_to_embedded(&mut poid, &vault, DEFAULT_QUOTA_BYTES).unwrap();
        assert_eq!(
            poid.manifest().storage.as_ref().unwrap().mode,
            StorageMode::Embedded
        );
        assert_eq!(poid.data().unwrap(), canonical);
    }

    #[test]
    fn conversion_survives_a_container_repack() {
        // The same round trip, but through actual container bytes both ways —
        // what the desktop flow does between the two conversions.
        let canonical = b"{\n  \"k\": true\n}\n";
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();

        let mut poid = app_with_data(canonical);
        embedded_to_vault(&mut poid, &vault, DEFAULT_QUOTA_BYTES).unwrap();
        let vault_mode_bytes = poid.to_bytes().unwrap();

        let mut reopened = poid_core::open(&vault_mode_bytes).unwrap();
        vault_to_embedded(&mut reopened, &vault, DEFAULT_QUOTA_BYTES).unwrap();
        assert_eq!(reopened.data().unwrap(), canonical);
    }

    #[test]
    fn named_slots_round_trip_with_the_current_pointer() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();

        let manifest = Manifest::new_app("dev.poid.slots", "S", "1.0.0", "app/index.html");
        let builder = PoidBuilder::new(manifest)
            .file("app/index.html", b"<!doctype html>".to_vec())
            .unwrap()
            .file("slots/work/store.json", b"{\n  \"a\": 1\n}\n".to_vec())
            .unwrap()
            .file("slots/current", b"work".to_vec())
            .unwrap();
        let mut poid = poid_core::open(&pack(builder).unwrap()).unwrap();

        embedded_to_vault(&mut poid, &vault, DEFAULT_QUOTA_BYTES).unwrap();
        assert!(poid.file("slots/work/store.json").is_none());

        vault_to_embedded(&mut poid, &vault, DEFAULT_QUOTA_BYTES).unwrap();
        assert_eq!(
            poid.file("slots/work/store.json").unwrap(),
            b"{\n  \"a\": 1\n}\n"
        );
        assert_eq!(poid.file("slots/current").unwrap(), b"work");
    }

    #[test]
    fn vault_to_embedded_without_an_id_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let manifest = Manifest::new_app("dev.poid.noid", "N", "1.0.0", "app/index.html");
        let builder = PoidBuilder::new(manifest)
            .file("app/index.html", b"x".to_vec())
            .unwrap();
        let mut poid = poid_core::open(&pack(builder).unwrap()).unwrap();
        poid.convert_storage_mode(StorageMode::Vault);
        assert!(vault_to_embedded(&mut poid, &vault, DEFAULT_QUOTA_BYTES).is_err());
    }
}
