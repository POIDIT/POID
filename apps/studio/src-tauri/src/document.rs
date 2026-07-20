//! Opening a container for a Reader window.
//!
//! `poid-core` does all validation; this module only shapes the result for
//! the window's TypeScript. A rejected container is a *result*, not an error:
//! the Reader window opens either way and shows an honest explanation panel
//! (UX rule 4) — a double-click must never silently do nothing.

use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use poid_core::SignatureStatus;
use serde::Serialize;

/// One container file for the frontend, base64-encoded for the JSON IPC hop.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    /// Container-relative path, e.g. `app/index.html`.
    pub path: String,
    /// File bytes, base64 (standard alphabet, padded).
    pub data_b64: String,
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
    },
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Opens and validates `path`, shaping the outcome for the Reader window.
pub fn load(path: &Path) -> DocumentDto {
    let file_name = display_name(path);
    let poid = match poid_core::open_path(path) {
        Ok(poid) => poid,
        Err(e) => {
            return DocumentDto::Rejected {
                registry: e.conformance_code().map(str::to_owned),
                code: e.code().to_owned(),
                message: e.to_string(),
                file_name,
            }
        }
    };

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
    }
}

#[cfg(test)]
mod tests {
    use super::{load, DocumentDto};

    #[test]
    fn a_missing_file_is_a_rejection_not_a_panic() {
        let dto = load(std::path::Path::new("Z:\\does\\not\\exist.poid"));
        match dto {
            DocumentDto::Rejected { registry, code, .. } => {
                assert_eq!(registry, None, "I/O failures have no registry code");
                assert_eq!(code, "io");
            }
            DocumentDto::Loaded { .. } => panic!("a missing file cannot load"),
        }
    }

    #[test]
    fn a_packed_container_loads_with_manifest_and_files() {
        let manifest =
            poid_core::Manifest::new_app("dev.poid.test", "Test", "1.0.0", "app/index.html");
        let builder = match poid_core::PoidBuilder::new(manifest).file(
            "app/index.html",
            b"<!doctype html><title>t</title>".to_vec(),
        ) {
            Ok(builder) => builder,
            Err(e) => panic!("builder rejected the file: {e}"),
        };
        let bytes = match poid_core::pack(builder) {
            Ok(bytes) => bytes,
            Err(e) => panic!("pack failed: {e}"),
        };
        let dir = std::env::temp_dir().join("poid-studio-doc-test");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("min.poid");
        std::fs::write(&path, bytes).ok();

        match load(&path) {
            DocumentDto::Loaded {
                manifest_json,
                signature,
                files,
                file_name,
                ..
            } => {
                assert_eq!(file_name, "min.poid");
                assert_eq!(signature, "none");
                assert!(manifest_json.contains("dev.poid.test"));
                assert!(files.iter().any(|f| f.path == "app/index.html"));
            }
            DocumentDto::Rejected { message, .. } => panic!("expected a load: {message}"),
        }
    }
}
