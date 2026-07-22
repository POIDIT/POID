//! Finding the bare imports a project needs resolved (ARCHITECTURE §5.2).
//!
//! Pure text scanning over the source files: relative and URL specifiers are
//! not dependencies, so only bare names survive. The same scan feeds the CLI
//! and Studio, so both resolve exactly the same set.

use std::collections::BTreeSet;

use crate::SourceFile;

/// The bare import specifiers used across a project's source files, sorted and
/// de-duplicated. Relative (`./`, `../`), absolute (`/`) and URL (`https://`,
/// `data:`) specifiers are excluded — they are not dependencies to resolve.
pub fn bare_imports(files: &[SourceFile]) -> BTreeSet<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn src(rel: &str, content: &str) -> SourceFile {
        SourceFile::new(rel, content.as_bytes().to_vec())
    }

    #[test]
    fn finds_bare_imports_and_ignores_relative_ones() {
        let files = vec![
            src(
                "main.tsx",
                "import React from \"react\";\nimport { z } from \"./local\";\nimport(\"lodash-es\");",
            ),
            src("styles.css", "body { color: red }"),
        ];
        let bare = bare_imports(&files);
        assert!(bare.contains("react"));
        assert!(bare.contains("lodash-es"));
        assert!(!bare.contains("./local"));
    }

    #[test]
    fn ignores_urls_and_absolute_paths() {
        let files = vec![src(
            "main.js",
            "import a from \"https://cdn/x.js\";\nimport b from \"/abs\";\nimport c from \"data:text/js,\";",
        )];
        assert!(bare_imports(&files).is_empty());
    }
}
