//! The conversion pipeline, split at the one boundary the CLI and Studio do
//! not share: the build engine.
//!
//! - [`prepare`] classifies the project, stages it, and resolves the Standard
//!   Library — everything up to, but not including, the bundle. Pure Rust, no
//!   filesystem, no esbuild.
//! - the caller runs the build: the CLI with its native esbuild sidecar,
//!   Studio with esbuild-wasm in the webview. It loads the [`Selection`]s
//!   `prepare` returned (verifying each with [`crate::verify_bundle`]) and
//!   hands the bundle back.
//! - [`finish`] inlines the bundle into the document and assembles the files
//!   destined for the container.
//! - [`pack_converted`] turns that into a signed-off `.poid`, manifest and all.
//!
//! Static projects and single HTML documents need no build at all: `prepare`
//! reports `needs_build == false`, and the caller goes straight to `finish`
//! with no bundle. That is the path Studio's converter walks first.

use crate::error::ConvertError;
use crate::{
    classify, converted_manifest, infer_permissions, inline_into_html, slug_of, ConvertPlan,
    InlineParts, InputKind, Selection, SourceFile,
};

use poid_core::PoidBuilder;

/// A build's output: the boundary type between the caller's bundler and
/// [`finish`]. The CLI's sidecar and Studio's esbuild-wasm both produce this.
#[derive(Debug, Clone)]
pub struct Bundled {
    /// Minified ESM.
    pub js: Vec<u8>,
    /// Minified CSS, when a stylesheet was bundled.
    pub css: Option<Vec<u8>>,
}

/// The result of [`prepare`]: what to build (if anything) and what [`finish`]
/// needs afterwards.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// What the input was classified as.
    pub kind: InputKind,
    /// Whether a build must run before [`finish`]. When false, call `finish`
    /// with `None`.
    pub needs_build: bool,
    /// The entry to bundle (only meaningful when `needs_build`).
    pub entry: Option<String>,
    /// Files to feed the bundler: the project sources plus any generated entry
    /// wrapper. Empty when no build is needed.
    pub staged: Vec<SourceFile>,
    /// Standard Library selections the caller must load and verify before
    /// building. Empty when no build is needed.
    pub selections: Vec<Selection>,
    /// Root-level passthrough trees (`data/`, `deps/`, `migrations/`).
    data_files: Vec<SourceFile>,
    /// Non-passthrough project files, kept for the consumed-file filter.
    rest: Vec<SourceFile>,
    /// The source HTML document's relative path, when there is one.
    html_rel: Option<String>,
    /// The source HTML document's text, when there is one.
    html_source: Option<String>,
    /// App files for the no-build path, already renamed.
    static_app_files: Vec<SourceFile>,
    /// The display title for the generated document.
    title: String,
}

/// A built application: the files destined for `app/`, the passthrough trees,
/// and the audit trail for `runtime.toolchain` / `runtime.bundled_deps`.
#[derive(Debug, Clone)]
pub struct BuiltApp {
    /// Files under `app/` (paths relative to `app/`).
    pub app_files: Vec<SourceFile>,
    /// Root-level passthrough trees (paths already prefixed).
    pub data_files: Vec<SourceFile>,
    /// `pkg@version` records of everything bundled from the Standard Library.
    pub bundled_deps: Vec<String>,
    /// Sidecar/engine version, when a build ran.
    pub esbuild_version: Option<String>,
    /// The final HTML document (also inside `app_files`), for inference.
    pub index_html: Option<String>,
}

fn is_passthrough(rel: &str) -> bool {
    rel == "data"
        || rel.starts_with("data/")
        || rel == "deps"
        || rel.starts_with("deps/")
        || rel == "migrations"
        || rel.starts_with("migrations/")
}

/// The generated mount wrapper for a single-component AI artifact.
fn artifact_entry(component_stem: &str) -> String {
    format!(
        "import App from \"./{component_stem}\";\nimport {{ createRoot }} from \"react-dom/client\";\nconst node = document.getElementById(\"root\") ?? (() => {{\n  const d = document.createElement(\"div\");\n  d.id = \"root\";\n  document.body.append(d);\n  return d;\n}})();\ncreateRoot(node).render(<App />);\n"
    )
}

/// Classifies the project, resolves Tier 1, and stages it for a build — the
/// half of conversion that is identical everywhere. `title` is the display
/// title for the generated document.
///
/// The build itself is the caller's: run esbuild over [`Prepared::staged`]
/// with the verified [`Prepared::selections`] as aliases, then call [`finish`].
pub fn prepare(files: Vec<SourceFile>, title: &str) -> Result<Prepared, ConvertError> {
    // `data/` (embedded state), `deps/` (bundled runtime dependencies) and
    // `migrations/` (schema scripts) travel at the container root, not under
    // `app/`, and are not fed to the bundler.
    let (data_files, rest): (Vec<_>, Vec<_>) =
        files.into_iter().partition(|f| is_passthrough(&f.rel));

    let shape = classify(&rest);

    // Static content: pack as-is; a lone document becomes index.html.
    if matches!(shape.kind, InputKind::Static | InputKind::SingleHtml) {
        let mut app_files = rest.clone();
        if let (InputKind::SingleHtml, Some(html)) = (shape.kind, &shape.html) {
            for f in &mut app_files {
                if &f.rel == html {
                    f.rel = "index.html".to_owned();
                }
            }
        }
        return Ok(Prepared {
            kind: shape.kind,
            needs_build: false,
            entry: None,
            staged: Vec::new(),
            selections: Vec::new(),
            data_files,
            rest,
            html_rel: shape.html.clone(),
            html_source: None,
            static_app_files: app_files,
            title: title.to_owned(),
        });
    }

    // A build is needed: resolve Tier 1, stage, and inject the artifact entry.
    let mut requested: Vec<String> = crate::bare_imports(&rest).into_iter().collect();
    if shape.uses_jsx && !requested.iter().any(|s| s == "react/jsx-runtime") {
        requested.push("react/jsx-runtime".to_owned());
    }
    let mut staged: Vec<SourceFile> = rest.clone();
    let entry = match shape.kind {
        InputKind::Artifact => {
            let component = shape.entry.clone().unwrap_or_default();
            let stem = component.trim_end_matches(".tsx").trim_end_matches(".jsx");
            staged.push(SourceFile::new(
                "__poid_entry.jsx",
                artifact_entry(stem).into_bytes(),
            ));
            for spec in ["react", "react/jsx-runtime", "react-dom/client"] {
                if !requested.iter().any(|s| s == spec) {
                    requested.push(spec.to_owned());
                }
            }
            "__poid_entry.jsx".to_owned()
        }
        _ => shape.entry.clone().ok_or_else(|| {
            ConvertError::new(
                "bundle-entry-missing",
                format!(
                    "this project has sources to build but no entry file; looked for {}",
                    crate::ENTRY_CANDIDATES.join(", ")
                ),
            )
        })?,
    };

    let resolution = crate::stdlib::resolve(requested)?;
    if !resolution.missing.is_empty() {
        let mut msg = format!(
            "cannot resolve bare import(s): `{}`. They are not in the Standard Library \
             (Tier 1). In Studio, the Resolver can download a dependency with your consent \
             and store it inside the POID (Tier 2); with the CLI, vendor the files into the \
             project and import them with relative paths. Nothing is fetched from the \
             network without consent.",
            resolution.missing.join("`, `")
        );
        for (name, reason) in &resolution.exclusions {
            msg.push_str(&format!("\n  `{name}`: {reason}"));
        }
        return Err(ConvertError::new("unresolved-dependency", msg));
    }

    let html_source = shape
        .html
        .as_ref()
        .and_then(|rel| rest.iter().find(|f| &f.rel == rel))
        .and_then(|f| std::str::from_utf8(&f.content).ok().map(str::to_owned));

    Ok(Prepared {
        kind: shape.kind,
        needs_build: true,
        entry: Some(entry),
        staged,
        selections: resolution.selected,
        data_files,
        rest,
        html_rel: shape.html.clone(),
        html_source,
        static_app_files: Vec::new(),
        title: title.to_owned(),
    })
}

/// Inlines the bundle into the document and assembles the container files
/// (M06 decision 1: readers execute inline output until the synthetic origin).
///
/// `bundled` is `None` for the no-build path and `Some` otherwise;
/// `esbuild_version` records which engine produced the bundle.
pub fn finish(
    prepared: Prepared,
    bundled: Option<Bundled>,
    esbuild_version: Option<String>,
) -> Result<BuiltApp, ConvertError> {
    if !prepared.needs_build {
        let index_html = prepared
            .static_app_files
            .iter()
            .find(|f| f.rel == "index.html")
            .and_then(|f| String::from_utf8(f.content.clone()).ok());
        return Ok(BuiltApp {
            app_files: prepared.static_app_files,
            data_files: prepared.data_files,
            bundled_deps: Vec::new(),
            esbuild_version: None,
            index_html,
        });
    }

    let bundled = bundled.ok_or_else(|| {
        ConvertError::new(
            "internal",
            "a project that needs building was finished without a bundle",
        )
    })?;

    let mut parts = InlineParts {
        js: Some(String::from_utf8_lossy(&bundled.js).into_owned()),
        css: None,
        title: prepared.title.clone(),
    };
    if let Some(css) = &bundled.css {
        parts.css = Some(String::from_utf8_lossy(css).into_owned());
    }
    let index_html = inline_into_html(prepared.html_source.as_deref(), &parts);

    // Sources, styles and the source HTML are consumed by the build; other
    // files (icons, datasets, docs) pack as-is.
    let html_rel = prepared.html_rel.as_deref();
    let consumed = |rel: &str| {
        [".js", ".mjs", ".ts", ".tsx", ".jsx", ".css"]
            .iter()
            .any(|ext| rel.ends_with(ext))
            || Some(rel) == html_rel
    };
    let mut app_files: Vec<SourceFile> = prepared
        .rest
        .into_iter()
        .filter(|f| !consumed(&f.rel))
        .collect();
    app_files.push(SourceFile::new(
        "index.html",
        index_html.clone().into_bytes(),
    ));
    app_files.sort_by(|a, b| a.rel.cmp(&b.rel));

    let mut bundled_deps: Vec<String> = Vec::new();
    for selection in &prepared.selections {
        if !bundled_deps.contains(&selection.record) {
            bundled_deps.push(selection.record.clone());
        }
    }
    bundled_deps.sort();

    Ok(BuiltApp {
        app_files,
        data_files: prepared.data_files,
        bundled_deps,
        esbuild_version,
        index_html: Some(index_html),
    })
}

/// Assembles a converted project into `.poid` bytes: infers permissions from
/// the built document, generates the manifest, packs, and opens the result to
/// prove it is readable — a converter never emits a file it would not itself
/// open.
///
/// `display_name` is the application's name; `id_source` is slugged into its
/// `local.poid.<slug>` id; `builder` records `runtime.toolchain.builder`.
pub fn pack_converted(
    built: &BuiltApp,
    display_name: &str,
    id_source: &str,
    builder: &str,
) -> Result<Vec<u8>, ConvertError> {
    if !built.app_files.iter().any(|f| f.rel == "index.html") {
        return Err(ConvertError::new(
            "convert-no-document",
            "the input produced no HTML document to open — a POID needs an app/index.html",
        ));
    }

    let inferred = infer_permissions(&[built.index_html.as_deref().unwrap_or_default()]);
    let plan = ConvertPlan {
        app_id: format!("local.poid.{}", slug_of(id_source)),
        name: display_name.to_owned(),
        inferred,
        bundled_deps: built.bundled_deps.clone(),
        builder: builder.to_owned(),
        esbuild: built.esbuild_version.clone(),
    };
    let manifest = converted_manifest(&plan)?;

    let mut builder = PoidBuilder::new(manifest);
    for f in &built.app_files {
        builder = builder.file(format!("app/{}", f.rel), f.content.clone())?;
    }
    for f in &built.data_files {
        builder = builder.file(f.rel.clone(), f.content.clone())?;
    }
    let bytes = poid_core::pack(builder)?;

    // Self-check: never emit a file this converter would not itself open.
    let poid = poid_core::open(&bytes)?;
    poid.verify()?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_html_file_needs_no_build_and_packs() {
        let files = vec![SourceFile::new(
            "page.html",
            b"<!doctype html><title>x</title><h1>hi</h1>".to_vec(),
        )];
        let prepared = prepare(files, "Page").expect("prepare");
        assert!(!prepared.needs_build);
        assert_eq!(prepared.kind, InputKind::SingleHtml);

        let built = finish(prepared, None, None).expect("finish");
        assert!(built.app_files.iter().any(|f| f.rel == "index.html"));
        assert!(built.esbuild_version.is_none());

        let bytes = pack_converted(&built, "Page", "page", "test@0").expect("pack");
        let poid = poid_core::open(&bytes).expect("opens");
        poid.verify().expect("verifies");
    }

    #[test]
    fn a_static_site_keeps_its_files() {
        let files = vec![
            SourceFile::new("index.html", b"<!doctype html><h1>hi</h1>".to_vec()),
            SourceFile::new("style.css", b"h1{color:red}".to_vec()),
        ];
        let prepared = prepare(files, "Site").expect("prepare");
        assert!(!prepared.needs_build);
        let built = finish(prepared, None, None).expect("finish");
        assert!(built.app_files.iter().any(|f| f.rel == "style.css"));
    }

    #[test]
    fn a_jsx_project_asks_for_a_build_and_resolves_react() {
        let files = vec![SourceFile::new(
            "main.jsx",
            b"import React from \"react\"; export default () => <h1>hi</h1>;".to_vec(),
        )];
        let prepared = prepare(files, "App").expect("prepare");
        assert!(prepared.needs_build);
        assert!(prepared.selections.iter().any(|s| s.specifier == "react"));
    }

    #[test]
    fn passthrough_trees_are_not_built() {
        let files = vec![
            SourceFile::new("index.html", b"<!doctype html><h1>hi</h1>".to_vec()),
            SourceFile::new("data/database.sql", b"CREATE TABLE t(x);".to_vec()),
        ];
        let prepared = prepare(files, "App").expect("prepare");
        let built = finish(prepared, None, None).expect("finish");
        assert!(built
            .data_files
            .iter()
            .any(|f| f.rel == "data/database.sql"));
        assert!(!built.app_files.iter().any(|f| f.rel.starts_with("data/")));
    }
}
