//! Project loading and the `pack` build pipeline: file collection, project
//! type detection, bare-import scanning and the esbuild sidecar.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::output::{err, CmdError};

/// Directories never packed into a container.
const SKIP_DIRS: [&str; 3] = ["node_modules", "target", "dist"];

/// A project file: path relative to the project root (forward slashes) plus
/// its content.
pub struct ProjectFile {
    /// Relative path, `/`-separated.
    pub rel: String,
    /// File content.
    pub content: Vec<u8>,
}

/// Reads all packable files under `dir`, skipping `poid.json`, dotfiles,
/// build/dependency directories and `.poid` outputs.
pub fn collect_files(dir: &Path) -> Result<Vec<ProjectFile>, CmdError> {
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<ProjectFile>) -> Result<(), CmdError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk(root, &path, out)?;
            continue;
        }
        if name == "poid.json" && path.parent() == Some(root) {
            continue;
        }
        if name.ends_with(".poid") {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|_| err("io", "path escapes project root"))?
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        out.push(ProjectFile {
            rel,
            content: std::fs::read(&path)?,
        });
    }
    Ok(())
}

/// Maps a project-relative path to its container path: `data/` travels as-is
/// (embedded state, SPEC §6), everything else lives under `app/`.
pub fn container_path(rel: &str) -> String {
    if rel == "data" || rel.starts_with("data/") {
        rel.to_owned()
    } else {
        format!("app/{rel}")
    }
}

/// True when the extension marks a source file that requires bundling.
pub fn needs_bundling(files: &[ProjectFile]) -> bool {
    files
        .iter()
        .any(|f| f.rel.ends_with(".ts") || f.rel.ends_with(".tsx") || f.rel.ends_with(".jsx"))
}

/// Scans JS/TS sources for bare imports (`import x from "react"`).
///
/// A heuristic line scan, not a parser — full resolution against the
/// Standard Library arrives in M06. Relative (`./`, `../`), absolute and
/// URL specifiers are fine; bare specifiers cannot be resolved yet.
pub fn bare_imports(files: &[ProjectFile]) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for file in files {
        let is_source = [".js", ".mjs", ".ts", ".tsx", ".jsx"]
            .iter()
            .any(|ext| file.rel.ends_with(ext));
        if !is_source {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&file.content) else {
            continue;
        };
        for spec in import_specifiers(text) {
            let relative = spec.starts_with("./") || spec.starts_with("../");
            let url = spec.contains("://") || spec.starts_with("data:");
            if !relative && !url && !spec.starts_with('/') && !spec.is_empty() {
                found.insert(spec);
            }
        }
    }
    found
}

/// Extracts string literals that follow `from` / `import(` / bare `import`.
fn import_specifiers(text: &str) -> Vec<String> {
    let mut specs = Vec::new();
    let bytes = text.as_bytes();
    let mut hits: Vec<usize> = Vec::new();
    for keyword in ["from", "import"] {
        hits.extend(text.match_indices(keyword).map(|(i, _)| i));
    }
    hits.sort_unstable();
    for i in hits {
        let mut j = i;
        // skip the keyword
        while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
            j += 1;
        }
        // skip whitespace and an optional `(` (dynamic import)
        while j < bytes.len() && (bytes[j].is_ascii_whitespace() || bytes[j] == b'(') {
            j += 1;
        }
        if j >= bytes.len() || (bytes[j] != b'"' && bytes[j] != b'\'') {
            continue;
        }
        let quote = bytes[j];
        let start = j + 1;
        if let Some(end) = text[start..].find(quote as char) {
            specs.push(text[start..start + end].to_owned());
        }
    }
    specs
}

/// The build contract shared with `@poid/toolchain` (ARCHITECTURE §5.1):
/// one pinned esbuild version, one option set, byte-identical output across
/// the CLI sidecar and the in-app esbuild-wasm engine.
const BUILD_CONTRACT: &str =
    include_str!("../../../packages/poid-toolchain/src/build-contract.json");

/// The typed slice of the contract the CLI consumes.
#[derive(serde::Deserialize)]
pub struct BuildContract {
    /// Exact esbuild version the sidecar must report.
    pub esbuild: String,
    format: String,
    platform: String,
    target: String,
    charset: String,
    #[serde(rename = "legalComments")]
    legal_comments: String,
    jsx: String,
    minify: bool,
    #[serde(rename = "entryName")]
    entry_name: String,
    define: std::collections::BTreeMap<String, String>,
    loader: std::collections::BTreeMap<String, String>,
}

/// Parses the embedded contract. Infallible in practice — the file is
/// compiled in — but surfaced as an error rather than a panic.
pub fn build_contract() -> Result<BuildContract, CmdError> {
    serde_json::from_str(BUILD_CONTRACT).map_err(|e| {
        err(
            "internal",
            format!("embedded build contract is invalid: {e}"),
        )
    })
}

/// The contract as sidecar flags, in the same order `@poid/toolchain`
/// generates them (both sides sort `define`/`loader` keys). A unit test on
/// each side pins the exact sequence so they cannot drift silently.
pub fn contract_flags(contract: &BuildContract) -> Vec<String> {
    let mut flags = vec![
        "--bundle".to_owned(),
        format!("--format={}", contract.format),
        format!("--platform={}", contract.platform),
        format!("--target={}", contract.target),
        format!("--charset={}", contract.charset),
        format!("--legal-comments={}", contract.legal_comments),
        format!("--jsx={}", contract.jsx),
    ];
    if contract.minify {
        flags.push("--minify".to_owned());
    }
    flags.push(format!("--entry-names={}", contract.entry_name));
    for (key, value) in &contract.define {
        flags.push(format!("--define:{key}={value}"));
    }
    for (ext, loader) in &contract.loader {
        flags.push(format!("--loader:{ext}={loader}"));
    }
    flags
}

/// A discovered esbuild sidecar.
pub struct Esbuild {
    /// Path to the executable.
    pub exe: PathBuf,
    /// Output of `esbuild --version`, recorded in `runtime.toolchain`.
    pub version: String,
}

/// Finds the native esbuild sidecar: `POID_ESBUILD` first, then an `esbuild`
/// binary next to the `poid` executable. Never downloads anything — network
/// fetching is a Studio feature with a consent dialog, not a CLI default.
pub fn find_esbuild() -> Result<Esbuild, CmdError> {
    let candidate = match std::env::var_os("POID_ESBUILD") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => {
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| PathBuf::from("."));
            let name = if cfg!(windows) {
                "esbuild.exe"
            } else {
                "esbuild"
            };
            exe_dir.join(name)
        }
    };
    if !candidate.is_file() {
        return Err(err(
            "esbuild-missing",
            format!(
                "this project needs bundling (TypeScript/JSX sources), but no esbuild sidecar \
                 was found at `{}`. Place the native esbuild binary next to `poid`, or point \
                 the POID_ESBUILD environment variable at it. Nothing is ever downloaded \
                 silently.",
                candidate.display()
            ),
        ));
    }
    let version_out = Command::new(&candidate)
        .arg("--version")
        .output()
        .map_err(|e| err("esbuild-missing", format!("cannot run esbuild: {e}")))?;
    let version = String::from_utf8_lossy(&version_out.stdout)
        .trim()
        .to_owned();
    if version.is_empty() {
        return Err(err(
            "esbuild-missing",
            "esbuild --version produced no output",
        ));
    }
    let pinned = build_contract()?.esbuild;
    if version != pinned {
        return Err(err(
            "esbuild-version-mismatch",
            format!(
                "the esbuild sidecar reports {version}, but the build contract pins {pinned}. \
                 Builds must be reproducible, so the versions have to match exactly — install \
                 esbuild {pinned} and point POID_ESBUILD at it."
            ),
        ));
    }
    Ok(Esbuild {
        exe: candidate,
        version,
    })
}

/// The output of one contract build: bundled ESM plus CSS when any
/// stylesheet was imported.
pub struct Bundled {
    /// Minified ESM (`main.js`).
    pub js: Vec<u8>,
    /// Minified CSS (`main.css`), when present.
    pub css: Option<Vec<u8>>,
}

/// Entry candidates, most specific first. `src/` variants cover the layout
/// AI tools and Vite templates emit; root variants cover flat projects.
pub const ENTRY_CANDIDATES: [&str; 8] = [
    "src/main.tsx",
    "src/main.ts",
    "src/main.jsx",
    "src/main.js",
    "main.tsx",
    "main.ts",
    "main.jsx",
    "main.js",
];

/// Bundles the project entry with the sidecar under the build contract.
///
/// The entry is passed **relative to the project directory** and the sidecar
/// runs with the project as its working directory: output must not depend on
/// where the project happens to live on disk (reproducibility), and CSS
/// module class names are derived from relative paths — the in-app engine
/// sees the same ones.
pub fn bundle(dir: &Path, esbuild: &Esbuild) -> Result<Bundled, CmdError> {
    let entry = ENTRY_CANDIDATES
        .iter()
        .find(|name| dir.join(name).is_file())
        .ok_or_else(|| {
            err(
                "bundle-entry-missing",
                format!(
                    "bundling requires an entry file; looked for {}",
                    ENTRY_CANDIDATES.join(", ")
                ),
            )
        })?;
    let contract = build_contract()?;
    let tmp = tempfile::tempdir()?;
    let output = Command::new(&esbuild.exe)
        .current_dir(dir)
        .arg(entry)
        .args(contract_flags(&contract))
        .arg(format!("--outdir={}", tmp.path().display()))
        .output()
        .map_err(|e| err("build-failed", format!("cannot run esbuild: {e}")))?;
    if !output.status.success() {
        return Err(err(
            "build-failed",
            format!(
                "esbuild failed:\n{}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    let js = std::fs::read(tmp.path().join(format!("{}.js", contract.entry_name)))?;
    let css_path = tmp.path().join(format!("{}.css", contract.entry_name));
    let css = if css_path.is_file() {
        Some(std::fs::read(&css_path)?)
    } else {
        None
    };
    Ok(Bundled { js, css })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors `contractCliFlags` in `@poid/toolchain` — the exact same
    /// literal list is pinned in `src/parity.test.ts` there. If the contract
    /// changes, both tests must be updated together; that is the tripwire.
    #[test]
    fn contract_flags_match_the_toolchain_side() {
        let Ok(contract) = build_contract() else {
            panic!("embedded contract must parse");
        };
        assert_eq!(contract.esbuild, "0.25.12");
        let flags = contract_flags(&contract);
        assert_eq!(
            flags,
            [
                "--bundle",
                "--format=esm",
                "--platform=browser",
                "--target=es2022",
                "--charset=utf8",
                "--legal-comments=none",
                "--jsx=automatic",
                "--minify",
                "--entry-names=main",
                "--define:process.env.NODE_ENV=\"production\"",
                "--loader:.avif=dataurl",
                "--loader:.csv=text",
                "--loader:.gif=dataurl",
                "--loader:.ico=dataurl",
                "--loader:.jpeg=dataurl",
                "--loader:.jpg=dataurl",
                "--loader:.md=text",
                "--loader:.mp3=dataurl",
                "--loader:.otf=dataurl",
                "--loader:.png=dataurl",
                "--loader:.svg=dataurl",
                "--loader:.ttf=dataurl",
                "--loader:.txt=text",
                "--loader:.wav=dataurl",
                "--loader:.webp=dataurl",
                "--loader:.woff=dataurl",
                "--loader:.woff2=dataurl",
            ]
        );
    }

    fn src(rel: &str, content: &str) -> ProjectFile {
        ProjectFile {
            rel: rel.to_owned(),
            content: content.as_bytes().to_vec(),
        }
    }

    #[test]
    fn finds_bare_imports_and_ignores_relative_ones() {
        let files = [src(
            "main.js",
            r#"import { a } from "./local.js";
import react from "react";
import "side-effect-pkg";
const lazy = await import("lodash-es");
import x from "../up.js";
import url from "https://example.com/mod.js";
"#,
        )];
        let bare = bare_imports(&files);
        let names: Vec<_> = bare.iter().map(String::as_str).collect();
        assert_eq!(names, ["lodash-es", "react", "side-effect-pkg"]);
    }

    #[test]
    fn non_source_files_are_not_scanned() {
        let files = [src("notes.md", r#"import fake from "react""#)];
        assert!(bare_imports(&files).is_empty());
    }

    #[test]
    fn bundling_detection() {
        assert!(needs_bundling(&[src("main.ts", "")]));
        assert!(needs_bundling(&[src("ui.tsx", "")]));
        assert!(!needs_bundling(&[src("main.js", ""), src("style.css", "")]));
    }

    #[test]
    fn container_path_mapping() {
        assert_eq!(container_path("index.html"), "app/index.html");
        assert_eq!(container_path("assets/icon.svg"), "app/assets/icon.svg");
        assert_eq!(container_path("data/store.json"), "data/store.json");
    }
}
