//! HTML assembly for the inline build output (M06 decision 1: until the
//! synthetic origin lands — issue #5 — readers execute inline code, so the
//! converter embeds the bundle into the document instead of linking it).

/// The pieces to inline into a document.
#[derive(Debug, Clone, Default)]
pub struct InlineParts {
    /// Bundled ESM, inserted as `<script type="module">`.
    pub js: Option<String>,
    /// Bundled CSS, inserted as `<style>`.
    pub css: Option<String>,
    /// Window title when the shell template is used.
    pub title: String,
}

/// Escapes a closing tag so inlined content cannot break out of its
/// element. The only sequence that can end a `<script>`/`<style>` element is
/// the literal closing tag, so this single substitution is sufficient — and
/// it must be identical in every converter, or builds would not be
/// byte-reproducible across them.
fn escape_close(content: &str, tag: &str) -> String {
    let needle = format!("</{tag}");
    let replacement = format!("<\\/{tag}");
    content.replace(&needle, &replacement)
}

/// The shell used when the input has no HTML of its own (artifacts, bare
/// projects).
const SHELL: &str = "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{{title}}</title>\n</head>\n<body>\n<div id=\"root\"></div>\n</body>\n</html>\n";

/// Inlines built output into an HTML document.
///
/// When `html` is `None`, the standard shell is used. A `<script>` whose
/// `src` points at a project source (the Vite pattern
/// `<script type="module" src="/src/main.tsx">`) is removed — its built
/// replacement is inlined; `<link rel="stylesheet">` pointing at bundled CSS
/// disappears the same way.
pub fn inline_into_html(html: Option<&str>, parts: &InlineParts) -> String {
    let base = match html {
        Some(doc) => strip_source_references(doc),
        None => SHELL.replace("{{title}}", &escape_html(&parts.title)),
    };

    let mut injection = String::new();
    if let Some(css) = &parts.css {
        injection.push_str("<style>\n");
        injection.push_str(&escape_close(css, "style"));
        injection.push_str("\n</style>\n");
    }
    if let Some(js) = &parts.js {
        injection.push_str("<script type=\"module\">\n");
        injection.push_str(&escape_close(js, "script"));
        injection.push_str("\n</script>\n");
    }
    if injection.is_empty() {
        return base;
    }

    // Before `</body>` when present, appended otherwise.
    if let Some(idx) = base.rfind("</body>") {
        let mut out = String::with_capacity(base.len() + injection.len());
        out.push_str(&base[..idx]);
        out.push_str(&injection);
        out.push_str(&base[idx..]);
        out
    } else {
        let mut out = base;
        out.push_str(&injection);
        out
    }
}

/// Drops `<script … src="…">…</script>` and stylesheet links that reference
/// project-relative sources — the built bundle replaces them.
fn strip_source_references(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let Some(start) = find_source_tag(rest) else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..start.0]);
        rest = &rest[start.1..];
    }
}

/// Finds the next source-referencing `<script>`/`<link rel="stylesheet">`
/// tag; returns (start, end) byte offsets or None.
fn find_source_tag(html: &str) -> Option<(usize, usize)> {
    let lower = html.to_ascii_lowercase();
    for (open, close) in [("<script", Some("</script>")), ("<link", None)] {
        let mut from = 0;
        while let Some(rel_start) = lower[from..].find(open) {
            let start = from + rel_start;
            let tag_end = lower[start..].find('>').map(|i| start + i + 1)?;
            let tag = &lower[start..tag_end];
            let references_source = tag.contains(" src=\"")
                || tag.contains(" src='")
                || (open == "<link" && tag.contains("stylesheet") && tag.contains(" href=\""));
            let external = tag.contains("http://") || tag.contains("https://");
            if references_source && !external {
                let end = match close {
                    Some(c) => lower[tag_end..].find(c).map(|i| tag_end + i + c.len())?,
                    None => tag_end,
                };
                return Some((start, end));
            }
            from = tag_end;
        }
    }
    None
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inlines_into_the_shell_when_no_html_exists() {
        let parts = InlineParts {
            js: Some("console.log(1)".into()),
            css: Some("body{margin:0}".into()),
            title: "My App".into(),
        };
        let html = inline_into_html(None, &parts);
        assert!(html.contains("<title>My App</title>"));
        assert!(html.contains("<script type=\"module\">\nconsole.log(1)\n</script>"));
        assert!(html.contains("<style>\nbody{margin:0}\n</style>"));
        let body_at = html.rfind("</body>").unwrap();
        let js_at = html.find("console.log(1)").unwrap();
        assert!(js_at < body_at, "the bundle must be inlined before </body>");
    }

    #[test]
    fn replaces_the_vite_source_script() {
        let doc = "<!doctype html><html><body><div id=\"root\"></div>\n<script type=\"module\" src=\"/src/main.tsx\"></script>\n</body></html>";
        let parts = InlineParts {
            js: Some("bundled()".into()),
            css: None,
            title: String::new(),
        };
        let html = inline_into_html(Some(doc), &parts);
        assert!(!html.contains("src=\"/src/main.tsx\""));
        assert!(html.contains("bundled()"));
    }

    #[test]
    fn keeps_external_scripts_untouched() {
        let doc = "<body><script src=\"https://example.com/x.js\"></script></body>";
        let html = inline_into_html(
            Some(doc),
            &InlineParts {
                js: None,
                css: None,
                title: String::new(),
            },
        );
        assert!(html.contains("https://example.com/x.js"));
    }

    #[test]
    fn escapes_closing_tags_inside_inlined_code() {
        let parts = InlineParts {
            js: Some("const s = \"</script>\";".into()),
            css: None,
            title: String::new(),
        };
        let html = inline_into_html(None, &parts);
        assert!(html.contains("<\\/script>"));
    }
}
