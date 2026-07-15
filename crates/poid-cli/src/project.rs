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
    Ok(Esbuild {
        exe: candidate,
        version,
    })
}

/// Bundles the project entry with esbuild into a single ESM file.
///
/// Returns the bundled `main.js` content. All `.ts`/`.tsx`/`.jsx`/`.js`
/// sources are consumed by the bundle; other files pack as-is.
pub fn bundle(dir: &Path, esbuild: &Esbuild) -> Result<Vec<u8>, CmdError> {
    let entry = ["main.ts", "main.tsx", "main.jsx", "main.js"]
        .iter()
        .map(|name| dir.join(name))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            err(
                "bundle-entry-missing",
                "bundling requires an entry file named main.ts, main.tsx, main.jsx or main.js \
                 at the project root",
            )
        })?;
    let tmp = tempfile::tempdir()?;
    let outfile = tmp.path().join("main.js");
    let output = Command::new(&esbuild.exe)
        .arg(&entry)
        .arg("--bundle")
        .arg("--format=esm")
        .arg("--platform=browser")
        .arg(format!("--outfile={}", outfile.display()))
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
    Ok(std::fs::read(&outfile)?)
}

#[cfg(test)]
mod tests {
    use super::*;

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
