//! Project templates for `poid init`.
//!
//! Every template is plain HTML/CSS/JS (or Python) with relative imports
//! only, so it packs in copy mode with zero build tooling and zero network.

use crate::Template;

/// Files (relative path, content) for a template.
pub fn files(template: Template, app_name: &str, app_id: &str) -> Vec<(String, String)> {
    match template {
        Template::Web => web(app_name, app_id),
        Template::Python => python(app_name, app_id),
        Template::Survey => survey(app_name, app_id),
    }
}

fn manifest_json(app_id: &str, app_name: &str, profile: &str, engines: Option<&str>) -> String {
    let engines_field = match engines {
        Some(e) => format!(",\n    \"engines\": {{ {e} }}"),
        None => String::new(),
    };
    format!(
        r#"{{
  "poid": "1.0",
  "type": "app",
  "app": {{
    "id": "{app_id}",
    "name": "{app_name}",
    "version": "0.1.0"
  }},
  "runtime": {{
    "profile": "{profile}"{engines_field}
  }},
  "entry": "app/index.html",
  "storage": {{
    "mode": "embedded"
  }},
  "permissions": {{
    "network": []
  }}
}}
"#
    )
}

fn index_html(title: &str, script: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{title}</title>
    <link rel="stylesheet" href="style.css" />
  </head>
  <body>
    <main id="app"></main>
    <script type="module" src="{script}"></script>
  </body>
</html>
"#
    )
}

const STYLE_CSS: &str = r#"body {
  font-family: system-ui, sans-serif;
  display: grid;
  place-items: center;
  min-height: 100vh;
  margin: 0;
}
main {
  text-align: center;
}
button {
  font-size: 1.25rem;
  padding: 0.5rem 1.5rem;
  cursor: pointer;
}
"#;

fn web(app_name: &str, app_id: &str) -> Vec<(String, String)> {
    let main_js = r#"// Relative imports only — this app packs with no build step and no network.
// The `window.poid` API (storage, files, …) arrives with the Reader runtime.
import { createCounter } from "./counter.js";

createCounter(document.getElementById("app"));
"#;
    let counter_js = r#"export function createCounter(root) {
  let count = 0;
  const heading = document.createElement("h1");
  const button = document.createElement("button");
  const render = () => {
    heading.textContent = `Count: ${count}`;
    button.textContent = "+1";
  };
  button.addEventListener("click", () => {
    count += 1;
    render();
  });
  render();
  root.append(heading, button);
  return { render };
}
"#;
    vec![
        (
            "poid.json".into(),
            manifest_json(app_id, app_name, "web", None),
        ),
        ("index.html".into(), index_html(app_name, "main.js")),
        ("main.js".into(), main_js.into()),
        ("counter.js".into(), counter_js.into()),
        ("style.css".into(), STYLE_CSS.into()),
    ]
}

fn python(app_name: &str, app_id: &str) -> Vec<(String, String)> {
    let main_py = r#"# Runs inside the reader via Pyodide (runtime profile "web+python").
# Third-party wheels ship in deps/ — never fetched at run time (SPEC 5.4).


def greet() -> str:
    return "Hello from Python inside a POID!"


if __name__ == "__main__":
    print(greet())
"#;
    let index = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <title>Python POID</title>
    <link rel="stylesheet" href="style.css" />
  </head>
  <body>
    <main id="app">
      <h1>Python POID</h1>
      <p>
        This template targets the <code>web+python</code> runtime profile.
        The reader provides the Pyodide engine; <code>main.py</code> is the
        application. Wiring Python to the DOM arrives with the engine
        milestone.
      </p>
    </main>
  </body>
</html>
"#;
    vec![
        (
            "poid.json".into(),
            manifest_json(
                app_id,
                app_name,
                "web+python",
                Some(r#""pyodide": ">=0.26""#),
            ),
        ),
        ("index.html".into(), index.into()),
        ("main.py".into(), main_py.into()),
        ("style.css".into(), STYLE_CSS.into()),
    ]
}

fn survey(app_name: &str, app_id: &str) -> Vec<(String, String)> {
    let main_js = r#"// Offline survey: answers stay on this machine. The respondent exports a
// small data file and sends it back — no server, no account (SPEC 11).
const form = document.getElementById("survey");
const exportBtn = document.getElementById("export");

exportBtn.addEventListener("click", () => {
  const answers = Object.fromEntries(new FormData(form).entries());
  const blob = new Blob([JSON.stringify({ schema: "responses/v1", answers }, null, 2)], {
    type: "application/json",
  });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = "response.json";
  a.click();
  URL.revokeObjectURL(a.href);
});
"#;
    let index = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <title>Survey</title>
    <link rel="stylesheet" href="style.css" />
  </head>
  <body>
    <main id="app">
      <h1>Survey</h1>
      <form id="survey">
        <p><label>Name: <input name="name" /></label></p>
        <p><label>How did it go? <input name="feedback" /></label></p>
      </form>
      <button id="export">Export answers</button>
    </main>
    <script type="module" src="main.js"></script>
  </body>
</html>
"#;
    vec![
        (
            "poid.json".into(),
            manifest_json(app_id, app_name, "web", None),
        ),
        ("index.html".into(), index.into()),
        ("main.js".into(), main_js.into()),
        ("style.css".into(), STYLE_CSS.into()),
    ]
}
