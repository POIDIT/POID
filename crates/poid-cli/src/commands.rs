//! Implementations of the `poid` subcommands. All container logic lives in
//! `poid-core`; this layer does argument handling, filesystem plumbing and
//! output shaping.

use std::io::Write;
use std::path::{Path, PathBuf};

use poid_core::{
    open_path, ContainerType, ExtraFields, Instance, Manifest, PoidBuilder, PoidError,
    SignatureStatus, ToolchainRecord, MEDIA_TYPE,
};
use serde_json::json;

use crate::output::{err, human_size, CmdError, Report};
use crate::project::{self, ProjectFile};
use crate::stdlib;
use crate::templates;
use crate::Template;

/// A built application: the files destined for `app/`, the passthrough
/// `data/` tree, and the audit trail for `runtime.toolchain` /
/// `runtime.bundled_deps`.
pub struct BuiltApp {
    /// Files under `app/` (paths relative to `app/`).
    app_files: Vec<ProjectFile>,
    /// Root-level passthrough trees (`data/`, `deps/` — paths already
    /// prefixed).
    data_files: Vec<ProjectFile>,
    /// `pkg@version` records of everything bundled from the Standard Library.
    bundled_deps: Vec<String>,
    /// Sidecar version, when a build ran.
    esbuild_version: Option<String>,
    /// The final HTML document (also inside `app_files`), for inference.
    index_html: Option<String>,
}

/// Classifies the project, resolves Tier 1 (Standard Library), builds when
/// sources need building, and inlines the result into the HTML document
/// (M06 decision 1: readers execute inline output until the synthetic
/// origin, issue #5). This is the one build path `pack` and `convert` share.
fn build_app(files: Vec<ProjectFile>, title: &str) -> Result<BuiltApp, CmdError> {
    // `data/` (embedded state, SPEC §6), `deps/` (bundled runtime
    // dependencies — Python wheels, SPEC §2.2) and `migrations/` (ordered
    // schema scripts, SPEC §12) travel at the container root, not under
    // `app/`, and are not fed to the bundler.
    let (data_files, rest): (Vec<_>, Vec<_>) = files.into_iter().partition(|f| {
        f.rel == "data"
            || f.rel.starts_with("data/")
            || f.rel == "deps"
            || f.rel.starts_with("deps/")
            || f.rel == "migrations"
            || f.rel.starts_with("migrations/")
    });

    let sources: Vec<poid_convert::SourceFile> = rest
        .iter()
        .map(|f| poid_convert::SourceFile::new(f.rel.clone(), f.content.clone()))
        .collect();
    let shape = poid_convert::classify(&sources);

    // Static content: pack as-is; a lone document becomes index.html.
    if matches!(
        shape.kind,
        poid_convert::InputKind::Static | poid_convert::InputKind::SingleHtml
    ) {
        let mut app_files = rest;
        if let (poid_convert::InputKind::SingleHtml, Some(html)) = (shape.kind, &shape.html) {
            for f in &mut app_files {
                if &f.rel == html {
                    f.rel = "index.html".to_owned();
                }
            }
        }
        let index_html = app_files
            .iter()
            .find(|f| f.rel == "index.html")
            .and_then(|f| String::from_utf8(f.content.clone()).ok());
        return Ok(BuiltApp {
            app_files,
            data_files,
            bundled_deps: Vec::new(),
            esbuild_version: None,
            index_html,
        });
    }

    // A build is needed: resolve Tier 1, stage, bundle, inline.
    let mut requested: Vec<String> = project::bare_imports(&rest).into_iter().collect();
    if shape.uses_jsx && !requested.iter().any(|s| s == "react/jsx-runtime") {
        // The automatic JSX runtime imports `react/jsx-runtime` even though
        // no source names it.
        requested.push("react/jsx-runtime".to_owned());
    }
    let mut staged: Vec<ProjectFile> = rest.clone();
    let entry = match shape.kind {
        poid_convert::InputKind::Artifact => {
            let component = shape.entry.clone().unwrap_or_default();
            let stem = component.trim_end_matches(".tsx").trim_end_matches(".jsx");
            staged.push(ProjectFile {
                rel: "__poid_entry.jsx".to_owned(),
                content: artifact_entry(stem).into_bytes(),
            });
            for spec in ["react", "react/jsx-runtime", "react-dom/client"] {
                if !requested.iter().any(|s| s == spec) {
                    requested.push(spec.to_owned());
                }
            }
            "__poid_entry.jsx".to_owned()
        }
        _ => shape.entry.clone().ok_or_else(|| {
            err(
                "bundle-entry-missing",
                format!(
                    "this project has sources to build but no entry file; looked for {}",
                    poid_convert::ENTRY_CANDIDATES.join(", ")
                ),
            )
        })?,
    };

    let resolution = stdlib::resolve(requested)?;
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
        return Err(err("unresolved-dependency", msg));
    }

    let mut aliases = Vec::new();
    let mut bundled_deps: Vec<String> = Vec::new();
    if !resolution.selected.is_empty() {
        let lib_dir = stdlib::locate_dir()?;
        for selection in &resolution.selected {
            aliases.push((
                selection.specifier.clone(),
                stdlib::load_verified(&lib_dir, selection)?,
            ));
            if !bundled_deps.contains(&selection.record) {
                bundled_deps.push(selection.record.clone());
            }
        }
        bundled_deps.sort();
    }

    let esbuild = project::find_esbuild()?;
    let bundled = project::bundle_staged(&staged, &entry, &aliases, &esbuild)?;

    let html_source = shape
        .html
        .as_ref()
        .and_then(|rel| rest.iter().find(|f| &f.rel == rel))
        .and_then(|f| std::str::from_utf8(&f.content).ok().map(str::to_owned));
    let mut parts = poid_convert::InlineParts {
        js: Some(String::from_utf8_lossy(&bundled.js).into_owned()),
        css: None,
        title: title.to_owned(),
    };
    if let Some(css) = &bundled.css {
        parts.css = Some(String::from_utf8_lossy(css).into_owned());
    }
    let index_html = poid_convert::inline_into_html(html_source.as_deref(), &parts);

    // Sources, styles and the source HTML are consumed by the build; other
    // files (icons, datasets the app opens at runtime, docs) pack as-is.
    let consumed = |rel: &str| {
        [".js", ".mjs", ".ts", ".tsx", ".jsx", ".css"]
            .iter()
            .any(|ext| rel.ends_with(ext))
            || Some(rel) == shape.html.as_deref()
    };
    let mut app_files: Vec<ProjectFile> = rest.into_iter().filter(|f| !consumed(&f.rel)).collect();
    app_files.push(ProjectFile {
        rel: "index.html".to_owned(),
        content: index_html.clone().into_bytes(),
    });
    app_files.sort_by(|a, b| a.rel.cmp(&b.rel));

    Ok(BuiltApp {
        app_files,
        data_files,
        bundled_deps,
        esbuild_version: Some(esbuild.version),
        index_html: Some(index_html),
    })
}

/// The generated mount wrapper for a single-component AI artifact.
fn artifact_entry(component_stem: &str) -> String {
    format!(
        "import App from \"./{component_stem}\";\nimport {{ createRoot }} from \"react-dom/client\";\nconst node = document.getElementById(\"root\") ?? (() => {{\n  const d = document.createElement(\"div\");\n  d.id = \"root\";\n  document.body.append(d);\n  return d;\n}})();\ncreateRoot(node).render(<App />);\n"
    )
}

pub fn init(dir: &Path, template: Template, force: bool) -> Result<Report, CmdError> {
    if dir.exists() && std::fs::read_dir(dir)?.next().is_some() && !force {
        return Err(err(
            "dir-not-empty",
            format!(
                "`{}` is not empty; pass --force to write into it anyway",
                dir.display()
            ),
        ));
    }
    std::fs::create_dir_all(dir)?;

    let raw_name = dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app".to_owned());
    let slug = sanitize_label(&raw_name);
    let app_name = capitalize(&raw_name);
    let app_id = format!("com.example.{slug}");

    let files = templates::files(template, &app_name, &app_id);
    let mut written = Vec::new();
    for (rel, content) in &files {
        std::fs::write(dir.join(rel), content)?;
        written.push(rel.clone());
    }

    let dirname = dir.display();
    Ok(Report {
        exit_failure: false,
        human: format!(
            "Created project in {dirname}\n  {}\n\nNext:\n  poid pack {dirname} -o {slug}.poid",
            written.join("\n  ")
        ),
        json: json!({
            "dir": dir.display().to_string(),
            "app_id": app_id,
            "files": written,
        }),
    })
}

pub fn pack(dir: &Path, output: Option<&Path>) -> Result<Report, CmdError> {
    if !dir.is_dir() {
        return Err(err(
            "project-missing",
            format!("`{}` is not a directory", dir.display()),
        ));
    }
    let manifest_bytes = std::fs::read(dir.join("poid.json")).map_err(|_| {
        err(
            "poid-json-missing",
            format!(
                "`{}` has no poid.json — run `poid init` first, or author one (SPEC 3.1)",
                dir.display()
            ),
        )
    })?;
    let mut manifest = Manifest::parse(&manifest_bytes).map_err(PoidError::from)?;

    let mut files = project::collect_files(dir)?;
    if files.is_empty() {
        return Err(err(
            "project-empty",
            format!("`{}` contains no packable files", dir.display()),
        ));
    }

    let title = manifest
        .app
        .as_ref()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "App".to_owned());
    let built = build_app(std::mem::take(&mut files), &title)?;

    // Generated fields the author never writes by hand.
    if manifest.container_type != ContainerType::Data && manifest.instance.is_none() {
        manifest.instance = Some(Instance {
            id: None,
            extra: ExtraFields::new(),
        });
    }
    if let Some(runtime) = &mut manifest.runtime {
        let toolchain = runtime.toolchain.get_or_insert_with(|| ToolchainRecord {
            builder: None,
            esbuild: None,
            extra: ExtraFields::new(),
        });
        toolchain.builder = Some(format!("poid-cli@{}", env!("CARGO_PKG_VERSION")));
        if let Some(v) = &built.esbuild_version {
            toolchain.esbuild = Some(v.clone());
        }
        if !built.bundled_deps.is_empty() {
            let mut deps = runtime.bundled_deps.take().unwrap_or_default();
            for record in &built.bundled_deps {
                if !deps.contains(record) {
                    deps.push(record.clone());
                }
            }
            deps.sort();
            runtime.bundled_deps = Some(deps);
        }
    }

    let mut builder = PoidBuilder::new(manifest);
    let mut count = 0usize;
    for f in &built.app_files {
        builder = builder.file(format!("app/{}", f.rel), f.content.clone())?;
        count += 1;
    }
    for f in &built.data_files {
        builder = builder.file(f.rel.clone(), f.content.clone())?;
        count += 1;
    }
    let bytes = poid_core::pack(builder)?;

    // Self-check: this CLI never writes a file it would not itself open.
    let poid = poid_core::open(&bytes)?;
    poid.verify()?;

    let out_path = match output {
        Some(p) => p.to_path_buf(),
        None => PathBuf::from(format!(
            "{}.poid",
            dir.file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "out".to_owned())
        )),
    };
    atomic_write(&out_path, &bytes)?;

    let (digest_app, digest_deps) = match &poid.manifest().integrity {
        Some(i) => (i.app.clone(), i.deps.clone()),
        None => (None, None),
    };
    Ok(Report {
        exit_failure: false,
        human: format!(
            "Packed {count} files ({}) -> {}",
            human_size(bytes.len() as u64),
            out_path.display()
        ),
        json: json!({
            "output": out_path.display().to_string(),
            "files": count,
            "bytes": bytes.len(),
            "integrity": { "app": digest_app, "deps": digest_deps },
            "esbuild": built.esbuild_version,
        }),
    })
}

/// `poid convert <input>` — the Converter (M06 §5): a folder, a ZIP, a
/// single HTML file or a single-file AI artifact becomes a `.poid`, with a
/// generated manifest whose permissions are inferred from the built code —
/// the most restrictive set that still works. Network is never inferred.
pub fn convert(input: &Path, output: Option<&Path>) -> Result<Report, CmdError> {
    let (files, raw_name) = load_convert_input(input)?;
    if files.is_empty() {
        return Err(err(
            "project-empty",
            format!("`{}` contains no convertible files", input.display()),
        ));
    }
    let name = capitalize(&raw_name);
    let built = build_app(files, &name)?;

    if !built.app_files.iter().any(|f| f.rel == "index.html") {
        return Err(err(
            "convert-no-document",
            "the input produced no HTML document to open — a POID needs an app/index.html",
        ));
    }

    let inferred =
        poid_convert::infer_permissions(&[built.index_html.as_deref().unwrap_or_default()]);
    let plan = poid_convert::ConvertPlan {
        app_id: format!("local.poid.{}", poid_convert::slug_of(&raw_name)),
        name: name.clone(),
        inferred,
        bundled_deps: built.bundled_deps.clone(),
        builder: format!("poid-cli@{}", env!("CARGO_PKG_VERSION")),
        esbuild: built.esbuild_version.clone(),
    };
    let manifest = poid_convert::converted_manifest(&plan).map_err(PoidError::from)?;

    let mut builder = PoidBuilder::new(manifest);
    let mut count = 0usize;
    for f in &built.app_files {
        builder = builder.file(format!("app/{}", f.rel), f.content.clone())?;
        count += 1;
    }
    for f in &built.data_files {
        builder = builder.file(f.rel.clone(), f.content.clone())?;
        count += 1;
    }
    let bytes = poid_core::pack(builder)?;

    // Self-check, like pack: never write a file this CLI would not open.
    let poid = poid_core::open(&bytes)?;
    poid.verify()?;

    let out_path = match output {
        Some(p) => p.to_path_buf(),
        None => PathBuf::from(format!("{}.poid", poid_convert::slug_of(&raw_name))),
    };
    atomic_write(&out_path, &bytes)?;

    Ok(Report {
        exit_failure: false,
        human: format!(
            "Converted `{}` -> {} ({count} files, {})",
            input.display(),
            out_path.display(),
            human_size(bytes.len() as u64)
        ),
        json: json!({
            "output": out_path.display().to_string(),
            "files": count,
            "bytes": bytes.len(),
            "bundled_deps": built.bundled_deps,
            "esbuild": built.esbuild_version,
        }),
    })
}

/// Loads the converter input: a directory, a `.zip`, or a single
/// `.html`/`.jsx`/`.tsx` file. Returns the file set and a display name.
fn load_convert_input(input: &Path) -> Result<(Vec<ProjectFile>, String), CmdError> {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app".to_owned());

    if input.is_dir() {
        return Ok((project::collect_files(input)?, stem));
    }
    let ext = input
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "zip" => {
            let file = std::fs::File::open(input)?;
            let mut archive = zip::ZipArchive::new(file)
                .map_err(|e| err("convert-bad-zip", format!("cannot read the ZIP: {e}")))?;
            let mut files = Vec::new();
            for i in 0..archive.len() {
                let mut entry = archive
                    .by_index(i)
                    .map_err(|e| err("convert-bad-zip", format!("cannot read the ZIP: {e}")))?;
                if entry.is_dir() {
                    continue;
                }
                let Some(path) = entry.enclosed_name() else {
                    return Err(err(
                        "convert-bad-zip",
                        "the ZIP contains a path that escapes its root",
                    ));
                };
                let rel = path
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                let mut content = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut content)?;
                files.push(ProjectFile { rel, content });
            }
            // A ZIP of a folder usually roots everything under one directory.
            strip_common_root(&mut files);
            files.sort_by(|a, b| a.rel.cmp(&b.rel));
            Ok((files, stem))
        }
        "html" | "htm" | "jsx" | "tsx" => Ok((
            vec![ProjectFile {
                rel: input
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "index.html".to_owned()),
                content: std::fs::read(input)?,
            }],
            stem,
        )),
        _ => Err(err(
            "convert-unsupported-input",
            format!(
                "`{}` is not something the converter understands — give it a folder, a .zip, \
                 a .html document or a single .jsx/.tsx artifact",
                input.display()
            ),
        )),
    }
}

/// Strips the single shared top-level directory a zipped folder produces.
fn strip_common_root(files: &mut [ProjectFile]) {
    let Some(first) = files.first() else { return };
    let Some(root) = first.rel.split('/').next().map(str::to_owned) else {
        return;
    };
    let prefix = format!("{root}/");
    if files.len() > 1 && files.iter().all(|f| f.rel.starts_with(&prefix)) {
        for f in files {
            f.rel = f.rel[prefix.len()..].to_owned();
        }
    }
}

pub fn validate(file: &Path) -> Result<Report, CmdError> {
    let poid = open_path(file)?;
    poid.verify()?;
    // A present-but-invalid signature is nonconformant (POID-050); absence
    // of a signature is fine — signing is optional (SPEC §9.3).
    if poid.signature_status()? == SignatureStatus::Invalid {
        return Err(PoidError::SignatureInvalid.into());
    }
    let m = poid.manifest();
    Ok(Report {
        exit_failure: false,
        human: format!(
            "OK: {} is a conformant POID (spec {})",
            file.display(),
            m.poid
        ),
        json: json!({ "valid": true, "spec": m.poid, "type": type_name(m.container_type) }),
    })
}

pub fn inspect(file: &Path) -> Result<Report, CmdError> {
    let poid = open_path(file)?;
    let m = poid.manifest();

    let mut file_rows = Vec::new();
    let mut total: u64 = 0;
    for (path, content) in poid.files() {
        total += content.len() as u64;
        file_rows.push((path.to_owned(), content.len() as u64));
    }

    let signature = match poid.signature_status() {
        Ok(SignatureStatus::Unsigned) => "unsigned".to_owned(),
        Ok(SignatureStatus::Valid { public_key }) => format!("valid ({public_key})"),
        Ok(SignatureStatus::Invalid) => "INVALID".to_owned(),
        Err(_) => "MALFORMED".to_owned(),
    };

    let mut human = String::new();
    if let Some(app) = &m.app {
        human.push_str(&format!("{} {}  ({})\n", app.name, app.version, app.id));
    }
    human.push_str(&format!("type: {}\n", type_name(m.container_type)));
    if let Some(entry) = &m.entry {
        human.push_str(&format!("entry: {entry}\n"));
    }
    if let Some(runtime) = &m.runtime {
        human.push_str(&format!("runtime: {}\n", runtime.profile));
    }
    if let Some(storage) = &m.storage {
        human.push_str(&format!("storage: {:?}\n", storage.mode).to_lowercase());
    }
    human.push_str(&format!("signature: {signature}\n"));
    human.push_str("permissions:\n");
    for line in permissions_lines(m) {
        human.push_str(&format!("  {line}\n"));
    }
    human.push_str(&format!(
        "files: {} entries, {} uncompressed\n",
        file_rows.len(),
        human_size(total)
    ));
    for (path, size) in &file_rows {
        human.push_str(&format!("  {:>9}  {}\n", human_size(*size), path));
    }

    let manifest_value: serde_json::Value =
        serde_json::from_slice(&m.to_json_bytes().map_err(PoidError::from)?)
            .map_err(|e| err("io", e.to_string()))?;
    Ok(Report {
        exit_failure: false,
        human: human.trim_end().to_owned(),
        json: json!({
            "manifest": manifest_value,
            "files": file_rows.iter().map(|(p, s)| json!({"path": p, "bytes": s})).collect::<Vec<_>>(),
            "total_bytes": total,
            "signature": signature,
        }),
    })
}

pub fn extract(file: &Path, output: &Path, force: bool) -> Result<Report, CmdError> {
    let poid = open_path(file)?;
    if output.exists() && std::fs::read_dir(output)?.next().is_some() && !force {
        return Err(err(
            "dir-not-empty",
            format!(
                "`{}` is not empty; pass --force to extract into it anyway",
                output.display()
            ),
        ));
    }
    std::fs::create_dir_all(output)?;

    std::fs::write(output.join("mimetype"), MEDIA_TYPE)?;
    let manifest_bytes = poid.manifest().to_json_bytes().map_err(PoidError::from)?;
    std::fs::write(output.join("manifest.json"), &manifest_bytes)?;

    let mut count = 2usize;
    for (path, content) in poid.files() {
        // Paths were validated on open; re-assert the invariant anyway.
        if path.starts_with('/') || path.contains("..") || path.contains('\\') {
            return Err(err(
                "path-traversal",
                format!("refusing to extract `{path}`"),
            ));
        }
        let dest = output.join(path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, content)?;
        count += 1;
    }

    Ok(Report {
        exit_failure: false,
        human: format!("Extracted {count} files to {}", output.display()),
        json: json!({ "output": output.display().to_string(), "files": count }),
    })
}

pub fn data(file: &Path, export: &Path) -> Result<Report, CmdError> {
    let poid = open_path(file)?;
    let Some(bytes) = poid.data() else {
        let mode = poid
            .manifest()
            .storage
            .as_ref()
            .map(|s| format!("{:?}", s.mode).to_lowercase())
            .unwrap_or_else(|| "unknown".to_owned());
        return Err(err(
            "no-data",
            format!("this container has no embedded data (storage mode: {mode})"),
        ));
    };
    atomic_write(export, bytes)?;
    Ok(Report {
        exit_failure: false,
        human: format!(
            "Exported {} of user data to {}",
            human_size(bytes.len() as u64),
            export.display()
        ),
        json: json!({ "export": export.display().to_string(), "bytes": bytes.len() }),
    })
}

pub fn keygen(output: &Path, force: bool) -> Result<Report, CmdError> {
    if output.exists() && !force {
        return Err(err(
            "key-exists",
            format!(
                "`{}` already exists; pass --force to overwrite it",
                output.display()
            ),
        ));
    }
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| err("rng", format!("cannot gather randomness: {e}")))?;
    let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
    let public_key = hex_encode(signing.verifying_key().as_bytes());

    std::fs::write(output, format!("{}\n", hex_encode(&seed)))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(output, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(Report {
        exit_failure: false,
        human: format!(
            "Private key written to {} — keep this file secret.\nPublic key: {public_key}",
            output.display()
        ),
        json: json!({ "output": output.display().to_string(), "public_key": public_key }),
    })
}

pub fn sign(file: &Path, key: &Path) -> Result<Report, CmdError> {
    let key_text = std::fs::read_to_string(key).map_err(|_| {
        err(
            "key-missing",
            format!("cannot read key file `{}`", key.display()),
        )
    })?;
    let seed: [u8; 32] = hex_decode(key_text.trim())
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| {
            err(
                "invalid-key",
                "the key file must contain 64 hex characters (create one with `poid keygen`)",
            )
        })?;

    let mut poid = open_path(file)?;
    poid.sign(&seed)?;
    poid.save_path(file)?;

    let public_key = match poid.signature_status()? {
        SignatureStatus::Valid { public_key } => public_key,
        _ => return Err(err("sign-failed", "signature did not verify after signing")),
    };
    Ok(Report {
        exit_failure: false,
        human: format!("Signed {}\nPublic key: {public_key}", file.display()),
        json: json!({ "signed": file.display().to_string(), "public_key": public_key }),
    })
}

/// `poid update <file> --from <newer>`: the "Update program, keep data" flow
/// (SPEC §12). Swaps the program in `file` for the one in `newer` (same
/// `app.id`), preserving the user's data, identity, and storage choices.
pub fn update(file: &Path, from: &Path) -> Result<Report, CmdError> {
    let mut target = open_path(file)?;
    let newer = open_path(from)?;
    let report = target.update_program(&newer)?;
    target.save_path(file)?;

    let app = target.manifest().app.as_ref();
    let version = app.map(|a| a.version.as_str()).unwrap_or("?");
    let mut notes: Vec<String> = Vec::new();
    if report.schema_advanced() {
        notes.push(format!(
            "schema {} → {}: migrations will run on next open",
            report.old_schema_version, report.new_schema_version
        ));
    }
    if report.permissions_widened {
        notes.push("permissions widened: the reader will re-request consent".to_owned());
    }
    let human = if notes.is_empty() {
        format!("Updated {} to v{version}. Data preserved.", file.display())
    } else {
        format!(
            "Updated {} to v{version}. Data preserved.\n  {}",
            file.display(),
            notes.join("\n  ")
        )
    };
    Ok(Report {
        exit_failure: false,
        human,
        json: json!({
            "updated": file.display().to_string(),
            "version": version,
            "schema_advanced": report.schema_advanced(),
            "old_schema_version": report.old_schema_version,
            "new_schema_version": report.new_schema_version,
            "permissions_widened": report.permissions_widened,
        }),
    })
}

/// Reads the passphrase from `--passphrase` or `POID_PASSPHRASE`. A blank
/// passphrase is refused — silent no-encryption is worse than an error.
fn passphrase(flag: Option<&str>) -> Result<String, CmdError> {
    let value = flag
        .map(str::to_owned)
        .or_else(|| std::env::var("POID_PASSPHRASE").ok())
        .unwrap_or_default();
    if value.is_empty() {
        return Err(err(
            "no-passphrase",
            "provide a passphrase with --passphrase or the POID_PASSPHRASE environment variable",
        ));
    }
    Ok(value)
}

pub fn protect(file: &Path, pass: Option<&str>) -> Result<Report, CmdError> {
    let passphrase = passphrase(pass)?;
    let mut poid = open_path(file)?;
    if poid.is_protected() {
        return Err(err(
            "already-protected",
            "this container's data is already encrypted",
        ));
    }
    let Some(plaintext) = poid.data().map(<[u8]>::to_vec) else {
        return Err(err(
            "no-data",
            "this container has no embedded data to protect (SPEC §9.2 protects `data/`)",
        ));
    };

    // Fresh salt and nonce per SPEC §9.2 (nonce is per-write).
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut salt).map_err(|e| err("rng", format!("cannot gather randomness: {e}")))?;
    getrandom::fill(&mut nonce)
        .map_err(|e| err("rng", format!("cannot gather randomness: {e}")))?;

    let envelope = poid_vault::protect::seal(
        &plaintext,
        passphrase.as_bytes(),
        salt,
        nonce,
        poid_vault::KdfParams::default(),
    )
    .map_err(|e| err("encrypt", e.to_string()))?;
    let bytes =
        poid_vault::protect::to_bytes(&envelope).map_err(|e| err("encrypt", e.to_string()))?;

    poid.set_protected_blob(&bytes);
    poid.save_path(file)?;
    Ok(Report {
        exit_failure: false,
        human: format!(
            "Encrypted the data in {} (AES-256-GCM + Argon2id).\n\
             Note: sending a POID with no data is safer still — absent data cannot leak.",
            file.display()
        ),
        json: json!({ "protected": file.display().to_string(), "alg": "aes-256-gcm" }),
    })
}

pub fn unprotect(file: &Path, pass: Option<&str>) -> Result<Report, CmdError> {
    let passphrase = passphrase(pass)?;
    let mut poid = open_path(file)?;
    let Some(blob) = poid.protected_blob().map(<[u8]>::to_vec) else {
        return Err(err(
            "not-protected",
            "this container has no encrypted data (`data/store.enc`)",
        ));
    };
    let envelope =
        poid_vault::protect::from_bytes(&blob).map_err(|e| err("decrypt", e.to_string()))?;
    let plaintext = poid_vault::protect::open(&envelope, passphrase.as_bytes())
        .map_err(|e| err("decrypt", e.to_string()))?;

    poid.set_plain_data(&plaintext);
    poid.save_path(file)?;
    Ok(Report {
        exit_failure: false,
        human: format!(
            "Decrypted the data in {} back to plaintext.",
            file.display()
        ),
        json: json!({ "unprotected": file.display().to_string() }),
    })
}

pub fn verify(file: &Path) -> Result<Report, CmdError> {
    let poid = open_path(file)?;
    poid.verify()?;
    match poid.signature_status()? {
        SignatureStatus::Unsigned => Ok(Report {
            exit_failure: false,
            human: "integrity: OK\nsignature: none (this POID is unsigned)".to_owned(),
            json: json!({ "integrity": "ok", "signature": "unsigned" }),
        }),
        SignatureStatus::Valid { public_key } => Ok(Report {
            exit_failure: false,
            human: format!("integrity: OK\nsignature: valid\npublic key: {public_key}"),
            json: json!({ "integrity": "ok", "signature": "valid", "public_key": public_key }),
        }),
        SignatureStatus::Invalid => Err(err(
            "signature-invalid",
            "integrity is OK, but the signature does not match the content — the file was \
             modified after signing, or the signature is not genuine",
        )),
    }
}

/// Runs the conformance suite at `dir` (SPEC §14, `spec/CONFORMANCE.md`):
/// every `valid/*.poid` must pass [`poid_core::conformance_check`], every
/// `invalid/*.poid` must fail with exactly the registry code its sibling
/// `*.expected.json` names.
pub fn conformance(dir: &Path) -> Result<Report, CmdError> {
    let limits = poid_core::Limits::default();
    let mut rows: Vec<(String, bool, String, String)> = Vec::new(); // name, pass, expected, got
    for group in ["valid", "invalid"] {
        let group_dir = dir.join(group);
        if !group_dir.is_dir() {
            return Err(err(
                "suite-missing",
                format!(
                    "`{}` does not contain a `{group}/` directory",
                    dir.display()
                ),
            ));
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&group_dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "poid"))
            .collect();
        files.sort();
        for path in files {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let expected_bytes = std::fs::read(group_dir.join(format!("{stem}.expected.json")))
                .map_err(|_| {
                    err(
                        "suite-invalid",
                        format!("fixture {group}/{stem} lacks a sibling {stem}.expected.json"),
                    )
                })?;
            let expected: serde_json::Value = serde_json::from_slice(&expected_bytes)
                .map_err(|e| err("suite-invalid", format!("{stem}.expected.json: {e}")))?;
            let want_valid = expected["valid"].as_bool().ok_or_else(|| {
                err(
                    "suite-invalid",
                    format!("{stem}.expected.json lacks a boolean `valid`"),
                )
            })?;
            let want = if want_valid {
                "valid".to_owned()
            } else {
                expected["code"].as_str().unwrap_or("invalid").to_owned()
            };

            let bytes = std::fs::read(&path)?;
            let got = match poid_core::conformance_check(&bytes, &limits) {
                Ok(_) => "valid".to_owned(),
                Err(e) => e.conformance_code().unwrap_or("io").to_owned(),
            };
            let pass = got == want;
            rows.push((format!("{group}/{stem}"), pass, want, got));
        }
    }
    if rows.is_empty() {
        return Err(err("suite-missing", "no fixtures found"));
    }

    let total = rows.len();
    let passed = rows.iter().filter(|(_, pass, _, _)| *pass).count();
    let failed = total - passed;
    let mut human = String::new();
    for (name, pass, want, got) in &rows {
        if *pass {
            human.push_str(&format!("PASS  {name}\n"));
        } else {
            human.push_str(&format!("FAIL  {name}  expected {want}, got {got}\n"));
        }
    }
    human.push_str(&format!(
        "\n{passed}/{total} fixtures passed{}",
        if failed == 0 { " — conformant" } else { "" }
    ));

    Ok(Report {
        exit_failure: failed > 0,
        human: human.trim_end().to_owned(),
        json: json!({
            "total": total,
            "passed": passed,
            "failed": failed,
            "results": rows.iter().map(|(name, pass, want, got)| json!({
                "fixture": name, "pass": pass, "expected": want, "got": got,
            })).collect::<Vec<_>>(),
        }),
    })
}

fn type_name(t: ContainerType) -> &'static str {
    match t {
        ContainerType::App => "app",
        ContainerType::Data => "data",
        ContainerType::Workspace => "workspace",
    }
}

fn permissions_lines(m: &Manifest) -> Vec<String> {
    let Some(p) = &m.permissions else {
        return vec!["(none requested)".to_owned()];
    };
    let mut lines = Vec::new();
    match p.network.as_deref() {
        None | Some([]) => lines.push("network: none — enforced by CSP".to_owned()),
        Some(origins) => lines.push(format!("network: {}", origins.join(", "))),
    }
    let fs_line = match p.filesystem {
        None | Some(poid_core::FilesystemAccess::None) => "filesystem: none",
        Some(poid_core::FilesystemAccess::UserInitiated) => {
            "filesystem: only via a file dialog the user opens"
        }
    };
    lines.push(fs_line.to_owned());
    for (name, value) in [
        ("clipboard", p.clipboard),
        ("print", p.print),
        ("notifications", p.notifications),
    ] {
        if value == Some(true) {
            lines.push(format!("{name}: requested"));
        }
    }
    if let Some(mcp) = &p.mcp {
        if !mcp.is_empty() {
            lines.push(format!("mcp: {}", mcp.join(", ")));
        }
    }
    lines
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), CmdError> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(&dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| CmdError::from(e.error))?;
    Ok(())
}

fn sanitize_label(name: &str) -> String {
    let cleaned: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "app".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "App".to_owned(),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let nibble = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        }
    };
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        out.push(nibble(pair[0])? << 4 | nibble(pair[1])?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_sanitized() {
        assert_eq!(sanitize_label("Demo App!"), "demo-app");
        assert_eq!(sanitize_label("demo"), "demo");
        assert_eq!(sanitize_label("__"), "app");
    }
}
