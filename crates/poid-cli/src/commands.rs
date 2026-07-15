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
use crate::templates;
use crate::Template;

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

    // Bare imports cannot be resolved before the Standard Library (M06).
    // Fail with the dependency's name — never fetch silently.
    let bare = project::bare_imports(&files);
    if !bare.is_empty() {
        let list = bare.into_iter().collect::<Vec<_>>().join("`, `");
        return Err(err(
            "unresolved-dependency",
            format!(
                "cannot resolve bare import(s): `{list}`. The Standard Library is not \
                 available yet (M06); vendor these dependencies and import them with \
                 relative paths. Nothing is fetched from the network without consent."
            ),
        ));
    }

    let mut esbuild_version: Option<String> = None;
    if project::needs_bundling(&files) {
        let esbuild = project::find_esbuild()?;
        let bundled = project::bundle(dir, &esbuild)?;
        esbuild_version = Some(esbuild.version);
        // Every JS/TS source is consumed by the bundle; other files pack as-is.
        files.retain(|f| {
            ![".js", ".mjs", ".ts", ".tsx", ".jsx"]
                .iter()
                .any(|ext| f.rel.ends_with(ext))
        });
        files.push(ProjectFile {
            rel: "main.js".to_owned(),
            content: bundled,
        });
    }

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
        if let Some(v) = &esbuild_version {
            toolchain.esbuild = Some(v.clone());
        }
    }

    let mut builder = PoidBuilder::new(manifest);
    let mut count = 0usize;
    for f in files {
        builder = builder.file(project::container_path(&f.rel), f.content)?;
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
            "esbuild": esbuild_version,
        }),
    })
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
    if s.len() % 2 != 0 {
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
