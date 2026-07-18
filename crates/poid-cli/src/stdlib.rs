//! Tier 1 in the CLI: resolving bare imports against the Standard Library
//! (ARCHITECTURE §5.2).
//!
//! The catalog is compiled in — the CLI trusts the checksums it was built
//! with, not whatever happens to sit on disk. Bundle files are looked up in
//! the directory `POID_STDLIB` points at (or `stdlib/` next to the
//! executable) and each file's sha256 is verified against the embedded
//! catalog before it is ever passed to the build.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::output::{err, CmdError};

/// The committed catalog, embedded at compile time — the same file
/// `@poid/stdlib` reads (single source of truth).
const CATALOG: &str = include_str!("../../../packages/poid-stdlib/src/catalog.json");

#[derive(serde::Deserialize)]
struct CatalogSpecifier {
    #[serde(default)]
    externals: Vec<String>,
    #[serde(default)]
    sha256: String,
}

#[derive(serde::Deserialize)]
struct CatalogPackage {
    version: String,
    specifiers: BTreeMap<String, CatalogSpecifier>,
}

#[derive(serde::Deserialize)]
struct Catalog {
    packages: BTreeMap<String, CatalogPackage>,
    #[serde(default)]
    excluded: BTreeMap<String, String>,
}

/// One resolved Standard Library selection.
pub struct Selection {
    /// The bare specifier, e.g. `react-dom/client`.
    pub specifier: String,
    /// `pkg@version` record for `runtime.bundled_deps`.
    pub record: String,
    /// Bundle path relative to the library root.
    pub rel: String,
    /// Expected sha256 (lowercase hex) of the bundle file.
    pub sha256: String,
}

/// The result of resolving bare imports against the embedded catalog.
pub struct Resolution {
    /// Resolved selections (externals followed transitively), sorted.
    pub selected: Vec<Selection>,
    /// Specifiers the catalog cannot serve, sorted.
    pub missing: Vec<String>,
    /// Human reasons for known exclusions among the missing.
    pub exclusions: Vec<(String, String)>,
}

fn parse_catalog() -> Result<Catalog, CmdError> {
    serde_json::from_str(CATALOG).map_err(|e| {
        err(
            "internal",
            format!("embedded stdlib catalog is invalid: {e}"),
        )
    })
}

/// Resolves bare imports transitively against the catalog. Pure — no disk.
pub fn resolve(bare: impl IntoIterator<Item = String>) -> Result<Resolution, CmdError> {
    let catalog = parse_catalog()?;
    let mut by_specifier: BTreeMap<String, (String, String, Vec<String>)> = BTreeMap::new();
    for (pkg, entry) in &catalog.packages {
        for (specifier, spec) in &entry.specifiers {
            by_specifier.insert(
                specifier.clone(),
                (
                    format!("{pkg}@{}", entry.version),
                    spec.sha256.clone(),
                    spec.externals.clone(),
                ),
            );
        }
    }

    let mut selected: BTreeMap<String, Selection> = BTreeMap::new();
    let mut missing: Vec<String> = Vec::new();
    let mut queue: Vec<String> = bare.into_iter().collect();
    while let Some(name) = queue.pop() {
        if selected.contains_key(&name) || missing.contains(&name) {
            continue;
        }
        match by_specifier.get(&name) {
            Some((record, sha256, externals)) => {
                queue.extend(externals.iter().cloned());
                selected.insert(
                    name.clone(),
                    Selection {
                        rel: format!("{name}.js"),
                        specifier: name,
                        record: record.clone(),
                        sha256: sha256.clone(),
                    },
                );
            }
            None => missing.push(name),
        }
    }
    missing.sort();

    let exclusions = missing
        .iter()
        .filter_map(|m| {
            let pkg = m.split('/').next().unwrap_or(m);
            catalog
                .excluded
                .get(pkg)
                .map(|reason| (m.clone(), reason.clone()))
        })
        .collect();

    Ok(Resolution {
        selected: selected.into_values().collect(),
        missing,
        exclusions,
    })
}

/// Locates the Standard Library directory: `POID_STDLIB`, then `stdlib/`
/// next to the executable. Never downloads anything.
pub fn locate_dir() -> Result<PathBuf, CmdError> {
    let candidate = match std::env::var_os("POID_STDLIB") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("stdlib"),
    };
    if !candidate.is_dir() {
        return Err(err(
            "stdlib-missing",
            format!(
                "this project imports Standard Library packages, but no library was found at \
                 `{}`. Point the POID_STDLIB environment variable at a built library \
                 (pnpm --filter @poid/stdlib build:lib produces one in \
                 packages/poid-stdlib/lib). Nothing is ever downloaded silently.",
                candidate.display()
            ),
        ));
    }
    Ok(candidate)
}

/// Loads one selection from the library directory, verifying its checksum
/// against the embedded catalog.
pub fn load_verified(dir: &Path, selection: &Selection) -> Result<PathBuf, CmdError> {
    let path = dir.join(&selection.rel);
    let content = std::fs::read(&path).map_err(|_| {
        err(
            "stdlib-incomplete",
            format!(
                "the Standard Library at `{}` has no `{}` — rebuild it \
                 (pnpm --filter @poid/stdlib build:lib)",
                dir.display(),
                selection.rel
            ),
        )
    })?;
    let digest = hex::encode(Sha256::digest(&content));
    if digest != selection.sha256 {
        return Err(err(
            "stdlib-checksum-mismatch",
            format!(
                "`{}` does not match the catalog checksum this CLI was built with; the library \
                 on disk is stale or tampered — rebuild it from the catalog",
                selection.rel
            ),
        ));
    }
    Ok(path)
}

mod hex {
    /// Lowercase hex without pulling a dependency.
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_transitively_and_reports_missing() {
        let Ok(resolution) = resolve(["react-dom/client".to_owned(), "tone".to_owned()]) else {
            panic!("embedded catalog must parse");
        };
        let names: Vec<_> = resolution
            .selected
            .iter()
            .map(|s| s.specifier.as_str())
            .collect();
        assert_eq!(
            names,
            ["react", "react-dom", "react-dom/client", "scheduler"]
        );
        assert_eq!(resolution.missing, ["tone"]);
        assert!(
            resolution.selected.iter().all(|s| s.sha256.len() == 64),
            "catalog carries checksums"
        );
    }

    #[test]
    fn exclusions_carry_reasons() {
        let Ok(resolution) = resolve(["svelte".to_owned()]) else {
            panic!("embedded catalog must parse");
        };
        assert_eq!(resolution.missing, ["svelte"]);
        assert_eq!(resolution.exclusions.len(), 1);
        assert!(resolution.exclusions[0].1.contains("compiler"));
    }
}
