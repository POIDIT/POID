//! The IPC surface of the host UI (hub + reader chrome).
//!
//! These commands are reachable only from Studio's own windows: the
//! application inside the sandboxed iframe has an opaque origin with no
//! usable Tauri IPC (verified in M07), and its one channel is the
//! `postMessage` bridge (`@poid/host`), which brokers a different,
//! capability-checked protocol.
//!
//! Scope discipline (security rule 3): every vault command resolves the
//! instance through the **calling window's label**. There is deliberately no
//! parameter that names an instance, a path, or another window.

use std::collections::HashMap;
use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use tauri::{AppHandle, Manager, State, WebviewWindow};
use uuid::Uuid;

use crate::asset_registry::{Asset, SessionAssets};
use crate::document::{self, DocumentDto};
use crate::state::Documents;
use crate::vault_state::VaultState;
use crate::windows;

/// One asset the reader hands the synthetic origin (SPEC §5.2.1).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetDto {
    /// Container path, e.g. `app/main.js` (empty = the entry, at the root).
    pub path: String,
    /// MIME type.
    pub content_type: String,
    /// Bytes, base64.
    pub data_b64: String,
}

/// The synthetic origin's base URL and its CSP source (SPEC §5.2.1). The
/// custom `poid` scheme is served at `poid://localhost` everywhere except
/// Windows/WebView2, which maps custom schemes to `http://<scheme>.localhost`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticOriginInfo {
    /// URL prefix; the reader appends `<session>/<entry>`.
    pub base: String,
    /// CSP `script-src`/`style-src` source for this origin.
    pub csp_source: String,
}

/// Reports the synthetic origin's URL base + CSP source to the reader.
#[tauri::command]
pub fn synthetic_origin() -> SyntheticOriginInfo {
    let origin = if cfg!(windows) {
        "http://poid.localhost"
    } else {
        "poid://localhost"
    };
    SyntheticOriginInfo {
        base: origin.to_owned(),
        csp_source: origin.to_owned(),
    }
}

/// Registers a Reader session's served assets for the `poid://` origin.
#[tauri::command]
pub fn register_session_assets(
    assets_state: State<'_, SessionAssets>,
    session: String,
    assets: Vec<AssetDto>,
) -> Result<(), String> {
    let mut map = HashMap::with_capacity(assets.len());
    for a in assets {
        let bytes = BASE64.decode(&a.data_b64).map_err(|e| e.to_string())?;
        map.insert(
            a.path,
            Asset {
                content_type: a.content_type,
                bytes,
            },
        );
    }
    assets_state.set(session, map);
    Ok(())
}

/// Releases a Reader session's served assets (reader teardown).
#[tauri::command]
pub fn revoke_session_assets(assets_state: State<'_, SessionAssets>, session: String) {
    assets_state.remove(&session);
}

/// Returns the document this Reader window was opened for.
#[tauri::command]
pub fn reader_bootstrap(
    window: WebviewWindow,
    documents: State<'_, Documents>,
) -> Result<DocumentDto, String> {
    documents
        .get(window.label())
        .ok_or_else(|| "no document is registered for this window".to_owned())
}

/// Opens `path` in a new Reader window (the hub's Open action).
#[tauri::command]
pub fn open_document(app: AppHandle, path: String) {
    windows::open_reader(&app, Path::new(&path));
}

/// The user's Fork / Move / Share decision for a copy conflict (SPEC §6.3).
/// Applies the choice, re-registers, binds the window, and returns the
/// refreshed document.
#[tauri::command]
pub fn resolve_copy_conflict(
    window: WebviewWindow,
    documents: State<'_, Documents>,
    vault: State<'_, VaultState>,
    choice: String,
) -> Result<DocumentDto, String> {
    let label = window.label().to_owned();
    let dto = documents
        .get(&label)
        .ok_or_else(|| "no document is registered for this window".to_owned())?;
    let DocumentDto::Loaded { path, .. } = &dto else {
        return Err("a rejected document has no copy conflict".to_owned());
    };
    let path = std::path::PathBuf::from(path);
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let mut poid = poid_core::open(&bytes).map_err(|e| e.to_string())?;
    let id = document::instance_id(&poid).ok_or("the document lost its instance id")?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let bound: Uuid = match choice.as_str() {
        "fork" => {
            // Fork: a new identity whose memory is a snapshot of the
            // original's — the two files diverge from here. (The from-scratch
            // variant is "Duplicate as empty", a separate operation.)
            let new_id = Uuid::new_v4();
            let src = vault.vault().instance_path(id);
            if src.exists() {
                std::fs::copy(&src, vault.vault().instance_path(new_id))
                    .map_err(|e| e.to_string())?;
            }
            poid.set_instance_id(new_id);
            poid.save_path(&path).map_err(|e| e.to_string())?;
            let hash = poid
                .to_bytes()
                .map(|b| poid_vault::hash_bytes(&b))
                .unwrap_or_default();
            vault
                .with_index(|index| index.register(new_id, path.clone(), hash))
                .map_err(|e| e.to_string())?;
            new_id
        }
        "move" => {
            vault
                .with_index(|index| index.record_move(id, path.clone()))
                .map_err(|e| e.to_string())?;
            id
        }
        "share" => {
            vault
                .with_index(|index| index.add_shared_path(id, path.clone()))
                .map_err(|e| e.to_string())?;
            id
        }
        other => return Err(format!("unknown copy resolution: {other}")),
    };

    vault.bind_window(&label, bound);
    let refreshed = document::loaded_dto(&poid, &path, file_name, None);
    documents.insert(label, refreshed.clone());
    Ok(refreshed)
}

/// One kv entry in a hydration snapshot.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultEntry {
    /// The key.
    pub key: String,
    /// The value, JSON-encoded.
    pub value_json: String,
}

/// What a Reader window's engine hydrates from.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultSnapshot {
    /// The active slot (SPEC §6.4 `current`).
    pub slot: String,
    /// Every slot that has stored data.
    pub slots: Vec<String>,
    /// The active slot's entries.
    pub entries: Vec<VaultEntry>,
}

fn window_instance(window: &WebviewWindow, vault: &VaultState) -> Result<(Uuid, u64), String> {
    let id = vault
        .window_instance(window.label())
        .ok_or("this window has no vault session")?;
    let quota = window
        .app_handle()
        .state::<Documents>()
        .get(window.label())
        .and_then(|dto| match dto {
            DocumentDto::Loaded { manifest_json, .. } => {
                serde_json::from_str::<serde_json::Value>(&manifest_json)
                    .ok()
                    .and_then(|m| m.pointer("/storage/quota_mb").and_then(|q| q.as_u64()))
                    .map(|mb| mb * 1024 * 1024)
            }
            DocumentDto::Rejected { .. } => None,
        })
        .unwrap_or(poid_vault::DEFAULT_QUOTA_BYTES);
    Ok((id, quota))
}

/// Full snapshot of the window's active slot (engine hydration).
#[tauri::command]
pub fn vault_hydrate(
    window: WebviewWindow,
    vault: State<'_, VaultState>,
) -> Result<VaultSnapshot, String> {
    let (id, quota) = window_instance(&window, &vault)?;
    vault
        .with_instance(id, quota, |inst| {
            let slot = inst.doc().current_slot();
            let mut entries = Vec::new();
            for key in inst.doc().kv_list(&slot, None) {
                if let Some(value) = inst.doc().kv_get(&slot, &key)? {
                    entries.push(VaultEntry {
                        key,
                        value_json: value.to_string(),
                    });
                }
            }
            Ok(VaultSnapshot {
                slot,
                slots: inst.doc().slots(),
                entries,
            })
        })
        .map_err(|e| e.to_string())
}

/// Sets a key in the window's active slot. `QUOTA_EXCEEDED` maps to the
/// broker error of the same name.
#[tauri::command]
pub fn vault_kv_set(
    window: WebviewWindow,
    vault: State<'_, VaultState>,
    key: String,
    value_json: String,
) -> Result<(), String> {
    let (id, quota) = window_instance(&window, &vault)?;
    let value: serde_json::Value = serde_json::from_str(&value_json).map_err(|e| e.to_string())?;
    vault
        .with_instance(id, quota, |inst| {
            let slot = inst.doc().current_slot();
            inst.kv_set(&slot, &key, &value)?;
            inst.flush()
        })
        .map_err(|e| match e {
            poid_vault::VaultError::QuotaExceeded { .. } => "QUOTA_EXCEEDED".to_owned(),
            other => other.to_string(),
        })
}

/// Deletes a key in the window's active slot.
#[tauri::command]
pub fn vault_kv_delete(
    window: WebviewWindow,
    vault: State<'_, VaultState>,
    key: String,
) -> Result<(), String> {
    let (id, quota) = window_instance(&window, &vault)?;
    vault
        .with_instance(id, quota, |inst| {
            let slot = inst.doc().current_slot();
            inst.doc_mut().kv_delete(&slot, &key)?;
            inst.flush()
        })
        .map_err(|e| e.to_string())
}

/// Clears the window's active slot.
#[tauri::command]
pub fn vault_kv_clear(window: WebviewWindow, vault: State<'_, VaultState>) -> Result<(), String> {
    let (id, quota) = window_instance(&window, &vault)?;
    vault
        .with_instance(id, quota, |inst| {
            let slot = inst.doc().current_slot();
            inst.doc_mut().kv_clear(&slot)?;
            inst.flush()
        })
        .map_err(|e| e.to_string())
}

/// Switches the active slot (a reader operation — the app never calls this;
/// the Reader window remounts afterwards so the app starts in the new
/// scope). Returns the fresh snapshot.
#[tauri::command]
pub fn vault_switch_slot(
    window: WebviewWindow,
    vault: State<'_, VaultState>,
    slot: String,
) -> Result<VaultSnapshot, String> {
    let (id, quota) = window_instance(&window, &vault)?;
    vault
        .with_instance(id, quota, |inst| {
            inst.doc_mut().set_current_slot(&slot)?;
            inst.flush()
        })
        .map_err(|e| e.to_string())?;
    vault_hydrate(window, vault)
}
