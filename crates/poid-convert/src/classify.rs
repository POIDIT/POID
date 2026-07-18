//! Input classification: what did the author hand us, and what has to
//! happen to it before it can be packed?

use crate::SourceFile;

/// What kind of input the converter recognized (M06 §5: a folder, a ZIP —
/// already expanded to files by the caller — a single HTML file, or a
/// single-file AI artifact).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// A lone `.html` document — packs as-is (after runtime injection).
    SingleHtml,
    /// A lone `.jsx`/`.tsx` component — an AI-chat artifact. Wrapped in the
    /// HTML shell, mounted, built.
    Artifact,
    /// A source project (has JS/TS sources that need bundling).
    Project,
    /// Static files only — packs as-is.
    Static,
}

/// The classification result: kind, the entry to build (when building is
/// needed) and the HTML document to serve.
#[derive(Debug, Clone)]
pub struct ProjectShape {
    /// Recognized input kind.
    pub kind: InputKind,
    /// Bundle entry (relative path into the file map), when a build is needed.
    pub entry: Option<String>,
    /// The HTML file that becomes `app/index.html`, when one exists.
    pub html: Option<String>,
    /// True when any source uses JSX/TSX (the automatic runtime import must
    /// be added to the Standard Library selection).
    pub uses_jsx: bool,
}

/// Entry candidates, most specific first — the same list the CLI bundles
/// with and Vite-style templates emit.
pub const ENTRY_CANDIDATES: [&str; 10] = [
    "src/main.tsx",
    "src/main.ts",
    "src/main.jsx",
    "src/main.js",
    "src/index.tsx",
    "src/index.jsx",
    "main.tsx",
    "main.ts",
    "main.jsx",
    "main.js",
];

fn has(files: &[SourceFile], rel: &str) -> bool {
    files.iter().any(|f| f.rel == rel)
}

fn is_source(rel: &str) -> bool {
    [".ts", ".tsx", ".jsx", ".mjs", ".js"]
        .iter()
        .any(|ext| rel.ends_with(ext))
}

/// Classifies a file map. ZIPs and folders are already flattened to files by
/// the caller; `package.json`, lockfiles and `node_modules` never reach this
/// point (the CLI skips them on collection; in-app conversion filters the
/// same way).
pub fn classify(files: &[SourceFile]) -> ProjectShape {
    let jsx = files
        .iter()
        .any(|f| (f.rel.ends_with(".jsx") || f.rel.ends_with(".tsx")) && !f.rel.contains('/'))
        || files
            .iter()
            .any(|f| f.rel.ends_with(".jsx") || f.rel.ends_with(".tsx"));

    // A single file is either a document or an artifact.
    if files.len() == 1 {
        if let Some(only) = files.first() {
            if only.rel.ends_with(".html") || only.rel.ends_with(".htm") {
                return ProjectShape {
                    kind: InputKind::SingleHtml,
                    entry: None,
                    html: Some(only.rel.clone()),
                    uses_jsx: false,
                };
            }
            if only.rel.ends_with(".jsx") || only.rel.ends_with(".tsx") {
                return ProjectShape {
                    kind: InputKind::Artifact,
                    entry: Some(only.rel.clone()),
                    html: None,
                    uses_jsx: true,
                };
            }
        }
    }

    // A build is needed only when plain packing cannot work: TypeScript/JSX
    // sources, or bare imports that must resolve through the Standard
    // Library. Plain JS with its own HTML packs as-is — the author wrote
    // something browsers already execute.
    let needs_build = files
        .iter()
        .any(|f| f.rel.ends_with(".ts") || f.rel.ends_with(".tsx") || f.rel.ends_with(".jsx"))
        || files.iter().any(|f| is_source(&f.rel) && bare_import_in(f));

    let entry = ENTRY_CANDIDATES
        .iter()
        .find(|c| has(files, c))
        .map(|c| (*c).to_owned());

    let html = ["index.html", "public/index.html", "app/index.html"]
        .iter()
        .find(|c| has(files, c))
        .map(|c| (*c).to_owned());

    if needs_build {
        return ProjectShape {
            kind: InputKind::Project,
            entry,
            html,
            uses_jsx: jsx,
        };
    }

    ProjectShape {
        kind: InputKind::Static,
        entry: None,
        html,
        uses_jsx: false,
    }
}

fn bare_import_in(file: &SourceFile) -> bool {
    let Ok(text) = std::str::from_utf8(&file.content) else {
        return false;
    };
    text.contains(" from \"")
        && text.lines().any(|l| {
            let t = l.trim_start();
            t.starts_with("import ")
                && !t.contains("\"./")
                && !t.contains("\"../")
                && !t.contains("'./")
                && !t.contains("'../")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lone_html_file_is_a_document() {
        let files = [SourceFile::new("chart.html", "<!doctype html><h1>hi</h1>")];
        let shape = classify(&files);
        assert_eq!(shape.kind, InputKind::SingleHtml);
        assert_eq!(shape.html.as_deref(), Some("chart.html"));
    }

    #[test]
    fn a_lone_tsx_file_is_an_artifact() {
        let files = [SourceFile::new(
            "App.tsx",
            "export default () => <h1>hi</h1>;",
        )];
        let shape = classify(&files);
        assert_eq!(shape.kind, InputKind::Artifact);
        assert_eq!(shape.entry.as_deref(), Some("App.tsx"));
        assert!(shape.uses_jsx);
    }

    #[test]
    fn a_vite_style_project_is_recognized_with_its_entry() {
        let files = [
            SourceFile::new("index.html", "<div id=\"root\"></div>"),
            SourceFile::new("src/main.tsx", "import App from \"./App\";"),
            SourceFile::new("src/App.tsx", "export default () => <p>ok</p>;"),
        ];
        let shape = classify(&files);
        assert_eq!(shape.kind, InputKind::Project);
        assert_eq!(shape.entry.as_deref(), Some("src/main.tsx"));
        assert_eq!(shape.html.as_deref(), Some("index.html"));
        assert!(shape.uses_jsx);
    }

    #[test]
    fn plain_files_are_static() {
        let files = [
            SourceFile::new("index.html", "<h1>docs</h1>"),
            SourceFile::new("style.css", "h1 { color: teal }"),
        ];
        assert_eq!(classify(&files).kind, InputKind::Static);
    }
}
