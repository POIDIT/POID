//! The esbuild-wasm build engine as a managed runtime (M12.2b).
//!
//! esbuild is 11.76 MiB. Baking it into the Studio binary would dent the
//! product's "~10 MB, not 120 MB" claim, so — like Pyodide — it is downloaded
//! once, verified against a committed checksum, and cached. A running POID can
//! never trigger this: only the converter, a Studio-side tool, asks for it
//! (SPEC §5.4, the same rule engines follow).
//!
//! The wasm the webview compiles is the same bytes this verifies; the pin in
//! `engines/esbuild.json` is the single source of truth, embedded here at
//! compile time so the binary carries the checksum it trusts.
//!
//! Sources, in order: the `POID_ESBUILD_WASM` override (a local file, for
//! development and CI, which stage the wasm from node_modules), then the disk
//! cache, then the network download from the pinned source.

use std::path::PathBuf;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

/// The committed engine pin, embedded so the binary carries the checksum it
/// trusts rather than reading one off disk.
const ENGINE_JSON: &str = include_str!("../../../../engines/esbuild.json");

#[derive(serde::Deserialize)]
struct EnginePin {
    version: String,
    source: String,
    files: std::collections::BTreeMap<String, String>,
}

fn pin() -> Result<EnginePin, String> {
    serde_json::from_str(ENGINE_JSON)
        .map_err(|e| format!("the embedded esbuild engine pin is invalid: {e}"))
}

/// What the runtime manager (M12.3) and the converter show about the engine.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    /// The pinned version.
    pub version: String,
    /// Whether a verified copy is available (cached, or via the override).
    pub installed: bool,
    /// Size in bytes when installed.
    pub byte_len: Option<usize>,
}

fn expected_sha(pin: &EnginePin) -> Result<&String, String> {
    pin.files
        .get("esbuild.wasm")
        .ok_or_else(|| "the engine pin names no esbuild.wasm".to_owned())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Where the verified engine is cached.
fn cache_path(app: &AppHandle, version: &str) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("no app data dir: {e}"))?
        .join("engines")
        .join("esbuild");
    Ok(dir.join(format!("esbuild-{version}.wasm")))
}

/// Reads a candidate file and returns its bytes only if the checksum matches.
fn read_if_valid(path: &std::path::Path, expected: &str) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    (sha256_hex(&bytes) == expected).then_some(bytes)
}

/// Returns the verified engine bytes, fetching and caching them if needed.
///
/// A `POID_ESBUILD_WASM` override or the disk cache short-circuits the network;
/// both are verified before use, so a stale or tampered file is treated as
/// absent rather than trusted.
async fn ensure_bytes(app: &AppHandle) -> Result<Vec<u8>, String> {
    let pin = pin()?;
    let expected = expected_sha(&pin)?.clone();

    // 1. The development / CI override: a local file staged from node_modules.
    if let Some(override_path) = std::env::var_os("POID_ESBUILD_WASM") {
        let path = PathBuf::from(override_path);
        return read_if_valid(&path, &expected).ok_or_else(|| {
            format!(
                "POID_ESBUILD_WASM points at `{}`, which is missing or does not match the \
                 pinned checksum",
                path.display()
            )
        });
    }

    // 2. The verified disk cache.
    let cached = cache_path(app, &pin.version)?;
    if let Some(bytes) = read_if_valid(&cached, &expected) {
        return Ok(bytes);
    }

    // 3. Download from the pinned source, verify, then cache. rustls uses a
    // process-global crypto provider; install ring's if nothing set one yet
    // (idempotent — a second install is ignored).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let response = reqwest::get(&pin.source)
        .await
        .map_err(|e| format!("could not reach the build engine at {}: {e}", pin.source))?;
    if !response.status().is_success() {
        return Err(format!(
            "the build engine download returned HTTP {} from {}",
            response.status(),
            pin.source
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("the build engine download failed: {e}"))?
        .to_vec();
    if sha256_hex(&bytes) != expected {
        return Err(
            "the downloaded build engine does not match its pinned checksum; refusing to use an \
             unverified engine"
                .to_owned(),
        );
    }
    if let Some(parent) = cached.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&cached, &bytes);
    Ok(bytes)
}

/// Reports whether a verified engine is available without downloading one.
#[tauri::command]
pub fn esbuild_engine_status(app: AppHandle) -> Result<EngineStatus, String> {
    let pin = pin()?;
    let expected = expected_sha(&pin)?.clone();

    let bytes = std::env::var_os("POID_ESBUILD_WASM")
        .map(PathBuf::from)
        .and_then(|p| read_if_valid(&p, &expected))
        .or_else(|| {
            cache_path(&app, &pin.version)
                .ok()
                .and_then(|p| read_if_valid(&p, &expected))
        });

    Ok(EngineStatus {
        version: pin.version,
        installed: bytes.is_some(),
        byte_len: bytes.map(|b| b.len()),
    })
}

/// Returns the verified engine wasm as base64 for the webview to compile into
/// a `WebAssembly.Module`. Downloads it first if necessary — this is the call
/// the converter makes when a project needs building.
#[tauri::command]
pub async fn esbuild_engine_wasm(app: AppHandle) -> Result<String, String> {
    let bytes = ensure_bytes(&app).await?;
    Ok(BASE64.encode(&bytes))
}

/// Removes the cached engine (the runtime manager's "remove Pyodide" for
/// esbuild). The override, if set, is untouched — it is not ours to delete.
#[tauri::command]
pub fn esbuild_engine_remove(app: AppHandle) -> Result<(), String> {
    let pin = pin()?;
    let cached = cache_path(&app, &pin.version)?;
    match std::fs::remove_file(&cached) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not remove the cached engine: {e}")),
    }
}

/// The pinned esbuild version, for stamping `runtime.toolchain.esbuild`.
pub fn pinned_version() -> Result<String, String> {
    Ok(pin()?.version)
}
