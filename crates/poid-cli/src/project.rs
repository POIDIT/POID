//! Project loading and the `pack` build pipeline: file collection, project
//! type detection, bare-import scanning and the esbuild sidecar.

use std::path::{Path, PathBuf};
use std::process::Command;

use poid_convert::{Bundled, SourceFile};

use crate::output::{err, CmdError};

/// Directories never packed into a container.
const SKIP_DIRS: [&str; 3] = ["node_modules", "target", "dist"];

/// Root-level authoring/tooling files a real `create-vite`-style project
/// carries that have no place in a POID: the recipient never builds
/// anything, so `vite.config.ts` etc. are noise at best — and at worst their
/// bare imports (`import { defineConfig } from "vite"`) would be
/// misidentified as application dependencies and block the whole pack.
/// Matched by exact name at the project root only (a source file the app
/// actually imports named e.g. `src/tailwind.config.ts` is untouched).
const SKIP_ROOT_FILES: [&str; 9] = [
    "package.json",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "tsconfig.json",
    "vite.config.ts",
    "vite.config.js",
    "eslint.config.js",
    "postcss.config.js",
];

/// A project file: path relative to the project root (forward slashes) plus
/// its content.
#[derive(Clone)]
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
        let at_root = path.parent() == Some(root);
        if name == "poid.json" && at_root {
            continue;
        }
        if at_root && SKIP_ROOT_FILES.contains(&name.as_str()) {
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

/// Bundles `entry` from an in-memory file set with the sidecar under the
/// build contract. Produces the [`poid_convert::Bundled`] the shared pipeline's
/// `finish` step consumes.
///
/// The files are **staged into a temporary directory** and the sidecar runs
/// with that directory as its working directory: output never depends on
/// where the project lives on disk (reproducibility — CSS module class
/// names derive from relative paths, and the in-app engine sees the same
/// ones). `aliases` map bare specifiers to files outside the stage (the
/// verified Standard Library bundles).
pub fn bundle_staged(
    files: &[SourceFile],
    entry: &str,
    aliases: &[(String, PathBuf)],
    esbuild: &Esbuild,
) -> Result<Bundled, CmdError> {
    let contract = build_contract()?;
    let stage = tempfile::tempdir()?;
    for file in files {
        let path = stage.path().join(&file.rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &file.content)?;
    }
    let out = tempfile::tempdir()?;
    let mut cmd = Command::new(&esbuild.exe);
    cmd.current_dir(stage.path())
        .arg(entry)
        .args(contract_flags(&contract));
    for (specifier, path) in aliases {
        cmd.arg(format!("--alias:{specifier}={}", path.display()));
    }
    let output = cmd
        .arg(format!("--outdir={}", out.path().display()))
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
    let js = std::fs::read(out.path().join(format!("{}.js", contract.entry_name)))?;
    let css_path = out.path().join(format!("{}.css", contract.entry_name));
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

    #[test]
    fn a_real_vite_project_ships_tooling_config_that_collect_files_must_skip() {
        // A create-vite output really does import "vite" and
        // "@vitejs/plugin-react" from its config — files no reader ever
        // builds. Before SKIP_ROOT_FILES, these would have poisoned
        // bare_imports and blocked every real create-vite project from
        // packing at all.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("package.json"), r#"{"name":"demo"}"#).unwrap();
        std::fs::write(
            dir.join("vite.config.ts"),
            "import { defineConfig } from \"vite\";\nimport react from \"@vitejs/plugin-react\";\nexport default defineConfig({ plugins: [react()] });\n",
        )
        .unwrap();
        std::fs::write(dir.join("tsconfig.json"), "{}").unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("src/main.tsx"),
            "import App from \"./App\";\nconsole.log(App);\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("src/App.tsx"),
            "export default function App() {}\n",
        )
        .unwrap();

        let Ok(files) = collect_files(dir) else {
            panic!("collect_files must succeed");
        };
        let rels: Vec<&str> = files.iter().map(|f| f.rel.as_str()).collect();
        assert!(!rels.contains(&"vite.config.ts"), "got {rels:?}");
        assert!(!rels.contains(&"package.json"), "got {rels:?}");
        assert!(!rels.contains(&"tsconfig.json"), "got {rels:?}");
        assert!(rels.contains(&"src/main.tsx"));
        assert!(rels.contains(&"src/App.tsx"));

        // And critically: no bare import from the excluded config leaks in.
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|f| SourceFile::new(f.rel.clone(), f.content.clone()))
            .collect();
        let bare = poid_convert::bare_imports(&sources);
        assert!(bare.is_empty(), "got {bare:?}");
    }

    #[test]
    fn a_source_file_that_happens_to_share_a_config_filename_is_kept() {
        // The exclusion is root-only and by exact relative depth, not by
        // basename anywhere — a nested file the app actually imports must
        // survive.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/vite.config.ts"), "export const x = 1;\n").unwrap();

        let Ok(files) = collect_files(dir) else {
            panic!("collect_files must succeed");
        };
        let rels: Vec<&str> = files.iter().map(|f| f.rel.as_str()).collect();
        assert!(rels.contains(&"src/vite.config.ts"), "got {rels:?}");
    }
}
