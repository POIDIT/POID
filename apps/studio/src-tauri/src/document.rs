//! Opening a container for a Reader window, including instance identity
//! (SPEC §3.2, §6.3).
//!
//! `poid-core` does all validation; this module shapes the result for the
//! window's TypeScript and runs the identity algorithm:
//! - a `null` `instance.id` is assigned, **written back into the file**
//!   atomically, and (for vault-mode files) registered in the vault index;
//! - a known id is classified Register / Open / Moved / Copy by the index —
//!   a genuine copy surfaces as `copyConflict` and the window prompts
//!   Fork / Move / Share before anything runs.
//!
//! Copy detection applies to vault-keyed memory only: an embedded file's
//! data travels inside it, so a copy is naturally independent and there is
//! nothing to contend for (interpretation noted in the M08 PR).
//!
//! A rejected container is a *result*, not an error: the Reader window opens
//! either way and shows an honest explanation panel (UX rule 4).

use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use poid_core::{Poid, SignatureStatus, StorageMode};
use serde::Serialize;
use uuid::Uuid;

use crate::vault_state::VaultState;

/// One container file for the frontend, base64-encoded for the JSON IPC hop.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    /// Container-relative path, e.g. `app/index.html`.
    pub path: String,
    /// File bytes, base64 (standard alphabet, padded).
    pub data_b64: String,
}

/// A genuine copy awaiting the user's Fork / Move / Share decision.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyConflict {
    /// A registered path of the same instance that still exists.
    pub existing_path: String,
}

/// What a Reader window receives when it asks for its document.
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DocumentDto {
    /// The container was refused by the validation core.
    #[serde(rename_all = "camelCase")]
    Rejected {
        /// Normative registry code (`POID-xxx`), when the rejection has one.
        registry: Option<String>,
        /// Stable diagnostic code, e.g. `native-code`.
        code: String,
        /// Technical detail from the core.
        message: String,
        /// The file's display name.
        file_name: String,
    },
    /// The container is valid; the frontend routes it (app / data / notice).
    #[serde(rename_all = "camelCase")]
    Loaded {
        /// The file's display name.
        file_name: String,
        /// Absolute path of the opened file.
        path: String,
        /// The validated manifest, serialized exactly as `poid-wasm` does —
        /// both readers feed the same `extractFacts`.
        manifest_json: String,
        /// `"valid"` or `"none"` (unsigned and unverifiable collapse to
        /// `"none"`, mirroring the Web Reader).
        signature: String,
        /// Every file in the container.
        files: Vec<FileEntry>,
        /// Set when this open is a genuine copy (SPEC §6.3 case 3); the
        /// window must prompt before consent.
        copy_conflict: Option<CopyConflict>,
    },
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn rejected(e: &poid_core::PoidError, file_name: String) -> DocumentDto {
    DocumentDto::Rejected {
        registry: e.conformance_code().map(str::to_owned),
        code: e.code().to_owned(),
        message: e.to_string(),
        file_name,
    }
}

/// The storage mode a manifest declares (default: embedded).
pub fn storage_mode(poid: &Poid) -> StorageMode {
    poid.manifest()
        .storage
        .as_ref()
        .map(|s| s.mode)
        .unwrap_or(StorageMode::Embedded)
}

/// The manifest's declared instance id, if any.
pub fn instance_id(poid: &Poid) -> Option<Uuid> {
    poid.manifest().instance.as_ref().and_then(|i| i.id)
}

/// Opens and validates `path`, runs the identity algorithm, and shapes the
/// outcome. Returns the DTO plus the instance id a vault session should bind
/// to (`None` for rejected documents and unresolved copy conflicts).
pub fn load(path: &Path, vault: &VaultState) -> (DocumentDto, Option<Uuid>) {
    let file_name = display_name(path);
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => return (rejected(&poid_core::PoidError::Io(e), file_name), None),
    };
    let mut poid = match poid_core::open(&bytes) {
        Ok(poid) => poid,
        Err(e) => return (rejected(&e, file_name), None),
    };

    let mode = storage_mode(&poid);
    let mut copy_conflict = None;

    let id = match instance_id(&poid) {
        None => {
            // First open anywhere: assign, write back, register (SPEC §3.2).
            let id = Uuid::new_v4();
            poid.set_instance_id(id);
            match poid.save_path(path) {
                Ok(()) => {
                    if mode == StorageMode::Vault {
                        register(vault, id, path, &poid);
                    }
                }
                // Read-only media: keep the id for this session, like the
                // Web Reader — it will travel in a saved copy, not this file.
                Err(e) => eprintln!("poid-studio: could not write instance.id back: {e}"),
            }
            Some(id)
        }
        Some(id) => {
            if mode == StorageMode::Vault {
                let disposition =
                    match vault.with_index(|index| index.resolve(id, path, |p| p.exists())) {
                        Ok(d) => d,
                        Err(e) => {
                            eprintln!("poid-studio: instance index unavailable: {e}");
                            poid_vault::Disposition::Open
                        }
                    };
                match disposition {
                    poid_vault::Disposition::Register => register(vault, id, path, &poid),
                    poid_vault::Disposition::Open => {}
                    poid_vault::Disposition::Moved => {
                        let _ = vault.with_index(|index| index.record_move(id, path.to_path_buf()));
                    }
                    poid_vault::Disposition::Copy { existing } => {
                        copy_conflict = Some(CopyConflict {
                            existing_path: existing.to_string_lossy().into_owned(),
                        });
                    }
                }
            }
            Some(id)
        }
    };

    let dto = loaded_dto(&poid, path, file_name, copy_conflict.clone());
    // A window binds to its instance only once the conflict (if any) is
    // resolved — the resolve command re-binds with the user's choice.
    let bind = if copy_conflict.is_none() { id } else { None };
    (dto, bind)
}

fn register(vault: &VaultState, id: Uuid, path: &Path, poid: &Poid) {
    let hash = poid
        .to_bytes()
        .map(|b| poid_vault::hash_bytes(&b))
        .unwrap_or_default();
    let _ = vault.with_index(|index| index.register(id, path.to_path_buf(), hash));
}

/// Shapes a validated container as the Loaded DTO.
pub fn loaded_dto(
    poid: &Poid,
    path: &Path,
    file_name: String,
    copy_conflict: Option<CopyConflict>,
) -> DocumentDto {
    let manifest_json = match serde_json::to_string(poid.manifest()) {
        Ok(json) => json,
        Err(e) => {
            // Unreachable in practice (the manifest just deserialized), but
            // an honest rejection beats a panic.
            return DocumentDto::Rejected {
                registry: None,
                code: "manifest-serialize".to_owned(),
                message: e.to_string(),
                file_name,
            };
        }
    };
    let signature = match poid.signature_status() {
        Ok(SignatureStatus::Valid { .. }) => "valid",
        _ => "none",
    };
    let files = poid
        .files()
        .map(|(p, bytes)| FileEntry {
            path: p.to_owned(),
            data_b64: BASE64.encode(bytes),
        })
        .collect();

    DocumentDto::Loaded {
        file_name,
        path: path.to_string_lossy().into_owned(),
        manifest_json,
        signature: signature.to_owned(),
        files,
        copy_conflict,
    }
}

#[cfg(test)]
mod tests {
    use super::{load, DocumentDto};
    use crate::vault_state::VaultState;
    use poid_core::StorageMode;

    fn vault_state() -> (tempfile::TempDir, VaultState) {
        let dir = tempfile::tempdir().unwrap();
        let state = VaultState::open(dir.path().to_path_buf()).unwrap();
        (dir, state)
    }

    fn vault_mode_app() -> Vec<u8> {
        let mut manifest =
            poid_core::Manifest::new_app("dev.poid.test", "Test", "1.0.0", "app/index.html");
        manifest.storage = Some(poid_core::Storage {
            mode: StorageMode::Vault,
            slots: None,
            protected: None,
            quota_mb: None,
            schema_version: None,
            requires: None,
            extra: poid_core::ExtraFields::new(),
        });
        let builder = poid_core::PoidBuilder::new(manifest)
            .file(
                "app/index.html",
                b"<!doctype html><title>t</title>".to_vec(),
            )
            .unwrap();
        poid_core::pack(builder).unwrap()
    }

    #[test]
    fn a_missing_file_is_a_rejection_not_a_panic() {
        let (_dir, vault) = vault_state();
        let (dto, bind) = load(std::path::Path::new("Z:\\does\\not\\exist.poid"), &vault);
        assert!(bind.is_none());
        match dto {
            DocumentDto::Rejected { registry, code, .. } => {
                assert_eq!(registry, None, "I/O failures have no registry code");
                assert_eq!(code, "io");
            }
            DocumentDto::Loaded { .. } => panic!("a missing file cannot load"),
        }
    }

    #[test]
    fn first_open_writes_the_id_back_and_registers() {
        let (_dir, vault) = vault_state();
        let docs = tempfile::tempdir().unwrap();
        let path = docs.path().join("v.poid");
        std::fs::write(&path, vault_mode_app()).unwrap();

        let (dto, bind) = load(&path, &vault);
        let id = bind.expect("vault doc binds");
        match dto {
            DocumentDto::Loaded {
                manifest_json,
                copy_conflict,
                ..
            } => {
                assert!(copy_conflict.is_none());
                assert!(manifest_json.contains(&id.to_string()), "id in the DTO");
            }
            DocumentDto::Rejected { message, .. } => panic!("expected a load: {message}"),
        }
        // The id was written into the file on disk (SPEC §3.2).
        let reread = poid_core::open(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(super::instance_id(&reread), Some(id));
    }

    #[test]
    fn a_byte_identical_copy_raises_the_conflict_prompt() {
        let (_dir, vault) = vault_state();
        let docs = tempfile::tempdir().unwrap();
        let original = docs.path().join("orig.poid");
        std::fs::write(&original, vault_mode_app()).unwrap();

        // First open assigns + registers.
        let (_, bind) = load(&original, &vault);
        assert!(bind.is_some());

        // Ctrl+C: the copy is byte-identical, both files exist.
        let copy = docs.path().join("copy.poid");
        std::fs::copy(&original, &copy).unwrap();

        let (dto, bind) = load(&copy, &vault);
        assert!(bind.is_none(), "an unresolved conflict must not bind");
        match dto {
            DocumentDto::Loaded { copy_conflict, .. } => {
                let conflict = copy_conflict.expect("the copy must prompt");
                assert!(conflict.existing_path.ends_with("orig.poid"));
            }
            DocumentDto::Rejected { message, .. } => panic!("expected a load: {message}"),
        }
    }

    #[test]
    fn a_move_is_silent() {
        let (_dir, vault) = vault_state();
        let docs = tempfile::tempdir().unwrap();
        let original = docs.path().join("here.poid");
        std::fs::write(&original, vault_mode_app()).unwrap();
        let (_, first_bind) = load(&original, &vault);

        let moved = docs.path().join("there.poid");
        std::fs::rename(&original, &moved).unwrap();

        let (dto, bind) = load(&moved, &vault);
        assert_eq!(bind, first_bind, "memory follows the move");
        match dto {
            DocumentDto::Loaded { copy_conflict, .. } => assert!(copy_conflict.is_none()),
            DocumentDto::Rejected { message, .. } => panic!("expected a load: {message}"),
        }
    }
}
