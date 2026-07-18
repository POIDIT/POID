//! Converter bindings: the same classification, inference, inline assembly
//! and manifest generation the CLI uses (`crates/poid-convert`), exposed to
//! Studio and the Web Reader. The JS side supplies the build (esbuild-wasm)
//! and packing; the *rules* stay single-sourced in Rust.
//!
//! Transport is JSON: file sets arrive as `[{ "rel": "...", "text": "..." }]`
//! (classification and inference read text sources only — binary assets are
//! irrelevant to both), results leave as JSON. Small, explicit, and free of
//! extra binding dependencies.

use wasm_bindgen::prelude::*;

use crate::ContainerError;

#[derive(serde::Deserialize)]
struct TextFile {
    rel: String,
    #[serde(default)]
    text: String,
}

fn parse_files(files_json: &str) -> Result<Vec<poid_convert::SourceFile>, ContainerError> {
    let files: Vec<TextFile> = serde_json::from_str(files_json).map_err(|e| ContainerError {
        code: "invalid-argument".to_owned(),
        registry: None,
        message: format!("files must be JSON `[{{rel, text}}]`: {e}"),
    })?;
    Ok(files
        .into_iter()
        .map(|f| poid_convert::SourceFile::new(f.rel, f.text.into_bytes()))
        .collect())
}

/// Classifies a project file set. Input: JSON `[{rel, text}]`; output JSON:
/// `{kind, entry, html, usesJsx}` with `kind` one of
/// `single-html | artifact | project | static`.
#[wasm_bindgen(js_name = classifyProject)]
pub fn classify_project(files_json: &str) -> Result<String, ContainerError> {
    let files = parse_files(files_json)?;
    let shape = poid_convert::classify(&files);
    let kind = match shape.kind {
        poid_convert::InputKind::SingleHtml => "single-html",
        poid_convert::InputKind::Artifact => "artifact",
        poid_convert::InputKind::Project => "project",
        poid_convert::InputKind::Static => "static",
    };
    Ok(serde_json::json!({
        "kind": kind,
        "entry": shape.entry,
        "html": shape.html,
        "usesJsx": shape.uses_jsx,
    })
    .to_string())
}

/// Inlines built output into an HTML document — the byte-identical
/// counterpart of the CLI's assembly step. `html` may be empty (the shell
/// template is used); empty `js`/`css` mean "nothing to inline".
#[wasm_bindgen(js_name = inlineIntoHtml)]
pub fn inline_into_html(html: &str, js: &str, css: &str, title: &str) -> String {
    let mut parts = poid_convert::InlineParts {
        js: None,
        css: None,
        title: title.to_owned(),
    };
    if !js.is_empty() {
        parts.js = Some(js.to_owned());
    }
    if !css.is_empty() {
        parts.css = Some(css.to_owned());
    }
    let base = if html.is_empty() { None } else { Some(html) };
    poid_convert::inline_into_html(base, &parts)
}

/// Generates the converted manifest, inferring permissions from the final
/// document (built output — dead code requests nothing). Input:
/// `bundledDeps` as JSON `["react@18.3.1", ...]`; returns manifest JSON.
#[wasm_bindgen(js_name = convertedManifest)]
pub fn converted_manifest(
    name: &str,
    index_html: &str,
    bundled_deps_json: &str,
    builder: &str,
    esbuild_version: &str,
) -> Result<String, ContainerError> {
    let bundled_deps: Vec<String> =
        serde_json::from_str(bundled_deps_json).map_err(|e| ContainerError {
            code: "invalid-argument".to_owned(),
            registry: None,
            message: format!("bundledDeps must be a JSON string array: {e}"),
        })?;
    let plan = poid_convert::ConvertPlan {
        app_id: format!("local.poid.{}", poid_convert::slug_of(name)),
        name: name.to_owned(),
        inferred: poid_convert::infer_permissions(&[index_html]),
        bundled_deps,
        builder: builder.to_owned(),
        esbuild: if esbuild_version.is_empty() {
            None
        } else {
            Some(esbuild_version.to_owned())
        },
    };
    let manifest = poid_convert::converted_manifest(&plan).map_err(poid_core::PoidError::from)?;
    let bytes = manifest
        .to_json_bytes()
        .map_err(poid_core::PoidError::from)?;
    String::from_utf8(bytes).map_err(|e| ContainerError {
        code: "internal".to_owned(),
        registry: None,
        message: format!("manifest serialization: {e}"),
    })
}
