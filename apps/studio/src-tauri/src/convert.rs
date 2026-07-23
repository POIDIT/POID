//! The Converter's IPC surface (M12.2, M12.2b): a dropped folder or file
//! becomes a `.poid`.
//!
//! The conversion logic is `poid-convert`'s — the exact pipeline the CLI runs,
//! so a project converts identically in Studio and on the command line. This
//! module adapts it to the hub and straddles the one boundary the CLI and
//! Studio do not share: the build engine.
//!
//! - `convert_prepare` classifies and stages. A project that needs no build
//!   (static files, a lone HTML page) is finished and returned right here.
//!   One that needs building comes back with a token and the inputs the
//!   webview's esbuild-wasm needs: the staged files, the verified Standard
//!   Library bundles, the aliases, and the entry.
//! - the webview builds (see `esbuild_engine` for the engine bytes).
//! - `convert_finish` takes the bundle back, inlines it, and packs the `.poid`.
//!
//! Reachable only from the hub window: these take file bytes and an opaque
//! job token, never a scope identifier.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use poid_convert::{Bundled, SourceFile};
use tauri::{AppHandle, Manager, State};

use crate::esbuild_engine;

/// esbuild aliases resolve to files under this prefix in the build's file map,
/// so a Standard Library bundle can never collide with a project source.
const STDLIB_PREFIX: &str = "__stdlib__/";

/// Prepared build jobs awaiting their bundle, keyed by an opaque token. The
/// display name rides along because `Prepared`'s title is private.
#[derive(Default)]
pub struct ConvertJobs(Mutex<HashMap<String, (poid_convert::Prepared, String)>>);

/// One file the webview read from the drop or the picker.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputFile {
    /// Path relative to the dropped root, `/`-separated.
    pub rel: String,
    /// Base64 of the file's bytes.
    pub bytes_base64: String,
}

/// A `{rel, base64}` pair the webview feeds to esbuild-wasm.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildFile {
    pub rel: String,
    pub bytes_base64: String,
}

/// The result of `convert_prepare`: either a finished `.poid` (no build) or a
/// build request (the webview must build, then call `convert_finish`).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertPrep {
    /// True when the webview must run a build before a `.poid` exists.
    pub needs_build: bool,
    /// How the input was classified, for the hub to show.
    pub kind: String,
    /// The finished `.poid` bytes, base64 (no-build path only).
    pub poid_base64: Option<String>,
    /// Files in the finished container (no-build path only).
    pub file_count: usize,
    /// Size of the finished `.poid` (no-build path only).
    pub byte_len: usize,
    /// Opaque handle to the stored job (build path only).
    pub token: Option<String>,
    /// Bundle entry (build path only).
    pub entry: Option<String>,
    /// Files to feed esbuild-wasm: staged sources plus the verified Standard
    /// Library bundles (build path only).
    pub build_files: Vec<BuildFile>,
    /// Bare specifier → path in `build_files` (build path only).
    pub aliases: HashMap<String, String>,
}

/// The result of `convert_finish`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertOutcome {
    /// The `.poid` bytes, base64.
    pub poid_base64: String,
    /// Number of files in the container.
    pub file_count: usize,
    /// Size of the produced `.poid` in bytes.
    pub byte_len: usize,
}

fn kind_word(kind: poid_convert::InputKind) -> &'static str {
    match kind {
        poid_convert::InputKind::Static => "static",
        poid_convert::InputKind::SingleHtml => "html",
        poid_convert::InputKind::Artifact => "artifact",
        poid_convert::InputKind::Project => "project",
    }
}

fn builder_id() -> String {
    format!("poid-studio@{}", env!("CARGO_PKG_VERSION"))
}

/// Locates the Standard Library directory: `POID_STDLIB`, then `stdlib/` in the
/// bundled resources. Mirrors the CLI's policy (paths differ; the checksum both
/// enforce is `poid_convert::verify_bundle`).
fn stdlib_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("POID_STDLIB") {
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    let resource = app
        .path()
        .resource_dir()
        .map_err(|e| format!("no resource dir: {e}"))?
        .join("stdlib");
    Ok(resource)
}

/// Loads and verifies one Standard Library bundle from disk.
fn load_stdlib_bundle(dir: &Path, selection: &poid_convert::Selection) -> Result<Vec<u8>, String> {
    let path = dir.join(&selection.rel);
    let bytes = std::fs::read(&path).map_err(|_| {
        format!(
            "this project needs `{}` from the Standard Library, but it is not installed. \
             Build it with `pnpm --filter @poid/stdlib build:lib`, or set POID_STDLIB.",
            selection.specifier
        )
    })?;
    poid_convert::verify_bundle(selection, &bytes).map_err(|e| e.message)?;
    Ok(bytes)
}

fn decode_sources(files: Vec<InputFile>) -> Result<Vec<SourceFile>, String> {
    let mut sources = Vec::with_capacity(files.len());
    for file in files {
        let bytes = BASE64
            .decode(file.bytes_base64.as_bytes())
            .map_err(|_| format!("`{}` could not be read.", file.rel))?;
        sources.push(SourceFile::new(file.rel, bytes));
    }
    Ok(sources)
}

/// Classifies and stages a dropped folder or file. Finishes the no-build path
/// outright; returns a build request otherwise.
#[tauri::command]
pub fn convert_prepare(
    files: Vec<InputFile>,
    display_name: String,
    app: AppHandle,
    jobs: State<'_, ConvertJobs>,
) -> Result<ConvertPrep, String> {
    if files.is_empty() {
        return Err("There were no files to convert.".to_owned());
    }
    let sources = decode_sources(files)?;
    let prepared = poid_convert::prepare(sources, &display_name).map_err(|e| e.message)?;
    let kind = kind_word(prepared.kind).to_owned();

    // No build: finish and pack right here.
    if !prepared.needs_build {
        let built = poid_convert::finish(prepared, None, None).map_err(|e| e.message)?;
        let file_count = built.app_files.len() + built.data_files.len();
        let bytes =
            poid_convert::pack_converted(&built, &display_name, &display_name, &builder_id())
                .map_err(|e| e.message)?;
        return Ok(ConvertPrep {
            needs_build: false,
            kind,
            poid_base64: Some(BASE64.encode(&bytes)),
            file_count,
            byte_len: bytes.len(),
            token: None,
            entry: None,
            build_files: Vec::new(),
            aliases: HashMap::new(),
        });
    }

    // Build path: gather the inputs the webview's esbuild-wasm needs.
    let mut build_files: Vec<BuildFile> = prepared
        .staged
        .iter()
        .map(|f| BuildFile {
            rel: f.rel.clone(),
            bytes_base64: BASE64.encode(&f.content),
        })
        .collect();

    let mut aliases = HashMap::new();
    if !prepared.selections.is_empty() {
        let dir = stdlib_dir(&app)?;
        for selection in &prepared.selections {
            let bytes = load_stdlib_bundle(&dir, selection)?;
            let virtual_path = format!("{STDLIB_PREFIX}{}", selection.rel);
            build_files.push(BuildFile {
                rel: virtual_path.clone(),
                bytes_base64: BASE64.encode(&bytes),
            });
            aliases.insert(selection.specifier.clone(), virtual_path);
        }
    }

    let entry = prepared.entry.clone();
    let token = uuid::Uuid::new_v4().to_string();
    jobs.0
        .lock()
        .map_err(|_| "the converter is busy".to_owned())?
        .insert(token.clone(), (prepared, display_name));

    Ok(ConvertPrep {
        needs_build: true,
        kind,
        poid_base64: None,
        file_count: 0,
        byte_len: 0,
        token: Some(token),
        entry,
        build_files,
        aliases,
    })
}

/// Takes the webview's bundle back, inlines it, and packs the `.poid`.
#[tauri::command]
pub fn convert_finish(
    token: String,
    js_base64: String,
    css_base64: Option<String>,
    jobs: State<'_, ConvertJobs>,
) -> Result<ConvertOutcome, String> {
    let (prepared, display_name) = jobs
        .0
        .lock()
        .map_err(|_| "the converter is busy".to_owned())?
        .remove(&token)
        .ok_or_else(|| "that conversion is no longer in progress.".to_owned())?;

    let js = BASE64
        .decode(js_base64.as_bytes())
        .map_err(|_| "the build output was corrupted.".to_owned())?;
    let css = match css_base64 {
        Some(c) => Some(
            BASE64
                .decode(c.as_bytes())
                .map_err(|_| "the build output was corrupted.".to_owned())?,
        ),
        None => None,
    };

    let version = esbuild_engine::pinned_version()?;
    let built = poid_convert::finish(prepared, Some(Bundled { js, css }), Some(version))
        .map_err(|e| e.message)?;
    let file_count = built.app_files.len() + built.data_files.len();
    let bytes = poid_convert::pack_converted(&built, &display_name, &display_name, &builder_id())
        .map_err(|e| e.message)?;

    Ok(ConvertOutcome {
        poid_base64: BASE64.encode(&bytes),
        file_count,
        byte_len: bytes.len(),
    })
}

/// Writes converted `.poid` bytes to the path the user chose in the save
/// dialog. Kept separate so the bytes are produced once and only touch disk
/// when the user commits to a location.
#[tauri::command]
pub fn write_poid(path: String, poid_base64: String) -> Result<(), String> {
    let bytes = BASE64
        .decode(poid_base64.as_bytes())
        .map_err(|_| "The converted file was corrupted before it could be saved.".to_owned())?;
    std::fs::write(&path, &bytes).map_err(|e| format!("Could not save to `{path}`: {e}"))
}
