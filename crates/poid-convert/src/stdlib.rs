//! Tier 1: resolving bare imports against the Standard Library
//! (ARCHITECTURE §5.2).
//!
//! The catalog is compiled in — the converter trusts the checksums it was
//! built with, not whatever sits on disk. Resolution is pure (no filesystem):
//! it returns the selections a caller must load, and [`verify_bundle`] checks a
//! loaded bundle's bytes against the embedded checksum. *Where* the bundles are
//! loaded from is the caller's business — the CLI reads a directory on disk,
//! Studio reads its own copy — but the checksum both must pass is defined here,
//! once.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::error::ConvertError;

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
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub struct Resolution {
    /// Resolved selections (externals followed transitively), sorted.
    pub selected: Vec<Selection>,
    /// Specifiers the catalog cannot serve, sorted.
    pub missing: Vec<String>,
    /// Human reasons for known exclusions among the missing.
    pub exclusions: Vec<(String, String)>,
}

fn parse_catalog() -> Result<Catalog, ConvertError> {
    serde_json::from_str(CATALOG).map_err(|e| {
        ConvertError::new(
            "internal",
            format!("embedded stdlib catalog is invalid: {e}"),
        )
    })
}

/// Resolves bare imports transitively against the catalog. Pure — no disk.
pub fn resolve(bare: impl IntoIterator<Item = String>) -> Result<Resolution, ConvertError> {
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

/// Verifies a loaded bundle's bytes against the checksum the catalog was built
/// with. The caller loads the bytes from wherever it keeps the library; this
/// is the one gate both the CLI and Studio pass a bundle through before it is
/// ever fed to a build.
pub fn verify_bundle(selection: &Selection, bytes: &[u8]) -> Result<(), ConvertError> {
    let digest = hex(&Sha256::digest(bytes));
    if digest != selection.sha256 {
        return Err(ConvertError::new(
            "stdlib-checksum-mismatch",
            format!(
                "`{}` does not match the catalog checksum this build was made with; the library \
                 on disk is stale or tampered — rebuild it from the catalog",
                selection.rel
            ),
        ));
    }
    Ok(())
}

/// Lowercase hex without pulling a dependency.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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

    #[test]
    fn verify_bundle_rejects_wrong_bytes() {
        let selection = Selection {
            specifier: "react".into(),
            record: "react@18".into(),
            rel: "react.js".into(),
            sha256: "0".repeat(64),
        };
        assert!(verify_bundle(&selection, b"anything").is_err());
    }
}
