//! The Converter's IPC surface (M12.2): a dropped folder or file becomes a
//! `.poid`.
//!
//! The conversion logic is `poid-convert`'s — the exact pipeline the CLI runs,
//! so a project converts identically in Studio and on the command line. This
//! module only adapts it to the hub: it decodes the files the webview read,
//! runs the shared pipeline, and writes the result where the user chose.
//!
//! The build path (TypeScript/JSX that must be bundled) is not here yet: it
//! needs the esbuild-wasm engine, which is a downloaded runtime. Until then a
//! project that needs building is reported honestly as such rather than
//! failing deep inside a bundle — `needs_build` says so and the hub explains.
//!
//! Reachable only from the hub window (like the connection commands): these
//! take file bytes, never a scope identifier, so there is no window-scope to
//! confuse.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use poid_convert::SourceFile;

/// One file the webview read from the drop or the picker.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputFile {
    /// Path relative to the dropped root, `/`-separated.
    pub rel: String,
    /// Base64 of the file's bytes.
    pub bytes_base64: String,
}

/// The outcome of a conversion attempt.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertOutcome {
    /// True when the project needs a build Studio cannot yet run in-app.
    /// `poid_base64` is then absent and `message` explains.
    pub needs_build: bool,
    /// The `.poid` bytes, base64, when conversion succeeded.
    pub poid_base64: Option<String>,
    /// How the input was classified, for the hub to show.
    pub kind: String,
    /// Number of files in the container.
    pub file_count: usize,
    /// Size of the produced `.poid` in bytes.
    pub byte_len: usize,
    /// A human explanation when there is nothing to download.
    pub message: Option<String>,
}

fn kind_word(kind: poid_convert::InputKind) -> &'static str {
    match kind {
        poid_convert::InputKind::Static => "static",
        poid_convert::InputKind::SingleHtml => "html",
        poid_convert::InputKind::Artifact => "artifact",
        poid_convert::InputKind::Project => "project",
    }
}

/// Converts a dropped folder or file into `.poid` bytes (the no-build path).
///
/// `display_name` is what the application is called; it is also slugged into
/// the container's `local.poid.<slug>` id.
#[tauri::command]
pub fn convert_to_poid(
    files: Vec<InputFile>,
    display_name: String,
) -> Result<ConvertOutcome, String> {
    if files.is_empty() {
        return Err("There were no files to convert.".to_owned());
    }

    let mut sources = Vec::with_capacity(files.len());
    for file in files {
        let bytes = BASE64
            .decode(file.bytes_base64.as_bytes())
            .map_err(|_| format!("`{}` could not be read.", file.rel))?;
        sources.push(SourceFile::new(file.rel, bytes));
    }

    let prepared = poid_convert::prepare(sources, &display_name).map_err(|e| e.message)?;
    let kind = kind_word(prepared.kind).to_owned();

    if prepared.needs_build {
        return Ok(ConvertOutcome {
            needs_build: true,
            poid_base64: None,
            kind,
            file_count: 0,
            byte_len: 0,
            message: Some(
                "This project has TypeScript or JSX that needs building. Studio will be able \
                 to build it once the build engine is installed — that is coming shortly. \
                 A folder of ready-to-run files (HTML, CSS, JavaScript) converts today."
                    .to_owned(),
            ),
        });
    }

    let built = poid_convert::finish(prepared, None, None).map_err(|e| e.message)?;
    let file_count = built.app_files.len() + built.data_files.len();
    let builder = format!("poid-studio@{}", env!("CARGO_PKG_VERSION"));
    let bytes = poid_convert::pack_converted(&built, &display_name, &display_name, &builder)
        .map_err(|e| e.message)?;

    Ok(ConvertOutcome {
        needs_build: false,
        byte_len: bytes.len(),
        poid_base64: Some(BASE64.encode(&bytes)),
        kind,
        file_count,
        message: None,
    })
}

/// Writes converted `.poid` bytes to the path the user chose in the save
/// dialog. Kept separate from [`convert_to_poid`] so the bytes are produced
/// once and only touch disk when the user commits to a location.
#[tauri::command]
pub fn write_poid(path: String, poid_base64: String) -> Result<(), String> {
    let bytes = BASE64
        .decode(poid_base64.as_bytes())
        .map_err(|_| "The converted file was corrupted before it could be saved.".to_owned())?;
    std::fs::write(&path, &bytes).map_err(|e| format!("Could not save to `{path}`: {e}"))
}
