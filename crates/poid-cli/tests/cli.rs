//! End-to-end tests of the `poid` binary: the M02 DoD flow
//! (`init → pack → validate`), JSON output on every command, stable error
//! codes, and the sign/verify chain.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use assert_cmd::Command;

fn poid() -> Command {
    Command::cargo_bin("poid").expect("binary builds")
}

fn json_stdout(assert: assert_cmd::assert::Assert) -> serde_json::Value {
    let out = assert.get_output().stdout.clone();
    serde_json::from_slice(&out).expect("stdout is JSON")
}

fn error_code(mut cmd: Command) -> String {
    let assert = cmd.assert().failure();
    let v = json_stdout(assert);
    v["error"]["code"].as_str().expect("error.code").to_owned()
}

#[test]
fn dod_flow_init_pack_validate() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = tmp.path().join("demo");
    let out = tmp.path().join("demo.poid");

    poid().args(["init"]).arg(&demo).assert().success();
    for name in [
        "poid.json",
        "index.html",
        "main.js",
        "counter.js",
        "style.css",
    ] {
        assert!(demo.join(name).is_file(), "missing template file {name}");
    }

    poid()
        .args(["pack"])
        .arg(&demo)
        .args(["-o"])
        .arg(&out)
        .assert()
        .success();
    assert!(out.is_file());

    poid().args(["validate"]).arg(&out).assert().success();

    // --json validate
    let v = json_stdout(
        poid()
            .args(["--json", "validate"])
            .arg(&out)
            .assert()
            .success(),
    );
    assert_eq!(v["valid"], true);
    assert_eq!(v["type"], "app");
}

#[test]
fn packing_is_deterministic() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = tmp.path().join("demo");
    poid().args(["init"]).arg(&demo).assert().success();

    let out1 = tmp.path().join("a.poid");
    let out2 = tmp.path().join("b.poid");
    poid()
        .arg("pack")
        .arg(&demo)
        .arg("-o")
        .arg(&out1)
        .assert()
        .success();
    poid()
        .arg("pack")
        .arg(&demo)
        .arg("-o")
        .arg(&out2)
        .assert()
        .success();
    assert_eq!(
        std::fs::read(&out1).unwrap(),
        std::fs::read(&out2).unwrap(),
        "packing the same project twice must be byte-identical"
    );
}

#[test]
fn inspect_extract_and_data_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = tmp.path().join("demo");
    let out = tmp.path().join("demo.poid");
    poid().arg("init").arg(&demo).assert().success();
    // Embedded user data travels as the container's data/ tree.
    std::fs::create_dir_all(demo.join("data")).unwrap();
    std::fs::write(demo.join("data/store.json"), br#"{"cards":[1,2]}"#).unwrap();
    poid()
        .arg("pack")
        .arg(&demo)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();

    let v = json_stdout(
        poid()
            .args(["--json", "inspect"])
            .arg(&out)
            .assert()
            .success(),
    );
    assert_eq!(v["manifest"]["app"]["id"], "com.example.demo");
    assert_eq!(v["signature"], "unsigned");
    assert!(v["total_bytes"].as_u64().unwrap() > 0);
    let paths: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"app/index.html"));
    assert!(paths.contains(&"data/store.json"));

    let extracted = tmp.path().join("extracted");
    poid()
        .arg("extract")
        .arg(&out)
        .arg("-o")
        .arg(&extracted)
        .assert()
        .success();
    assert_eq!(
        std::fs::read(extracted.join("mimetype")).unwrap(),
        b"application/vnd.poid+zip"
    );
    assert!(extracted.join("manifest.json").is_file());
    assert!(extracted.join("app/index.html").is_file());

    let export = tmp.path().join("data.json");
    poid()
        .arg("data")
        .arg(&out)
        .arg("--export")
        .arg(&export)
        .assert()
        .success();
    assert_eq!(std::fs::read(&export).unwrap(), br#"{"cards":[1,2]}"#);
}

#[test]
fn data_export_without_data_fails_cleanly() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = tmp.path().join("demo");
    let out = tmp.path().join("demo.poid");
    poid().arg("init").arg(&demo).assert().success();
    poid()
        .arg("pack")
        .arg(&demo)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();

    let mut cmd = poid();
    cmd.args(["--json", "data"])
        .arg(&out)
        .arg("--export")
        .arg(tmp.path().join("x.json"));
    assert_eq!(error_code(cmd), "no-data");
}

#[test]
fn sign_and_verify_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = tmp.path().join("demo");
    let out = tmp.path().join("demo.poid");
    let key = tmp.path().join("publisher.key");
    poid().arg("init").arg(&demo).assert().success();
    poid()
        .arg("pack")
        .arg(&demo)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();

    // Unsigned file verifies (integrity only), reported as unsigned.
    let v = json_stdout(
        poid()
            .args(["--json", "verify"])
            .arg(&out)
            .assert()
            .success(),
    );
    assert_eq!(v["signature"], "unsigned");

    let v = json_stdout(
        poid()
            .args(["--json", "keygen", "-o"])
            .arg(&key)
            .assert()
            .success(),
    );
    let public_key = v["public_key"].as_str().unwrap().to_owned();
    assert_eq!(public_key.len(), 64);

    // Refuses to clobber a key.
    let mut cmd = poid();
    cmd.args(["--json", "keygen", "-o"]).arg(&key);
    assert_eq!(error_code(cmd), "key-exists");

    let v = json_stdout(
        poid()
            .args(["--json", "sign"])
            .arg(&out)
            .arg("--key")
            .arg(&key)
            .assert()
            .success(),
    );
    assert_eq!(v["public_key"], public_key.as_str());

    let v = json_stdout(
        poid()
            .args(["--json", "verify"])
            .arg(&out)
            .assert()
            .success(),
    );
    assert_eq!(v["signature"], "valid");
    assert_eq!(v["public_key"], public_key.as_str());

    // The signed container is still conformant and still opens.
    poid().arg("validate").arg(&out).assert().success();

    // Garbage key file → invalid-key.
    let bad_key = tmp.path().join("bad.key");
    std::fs::write(&bad_key, "definitely-not-hex\n").unwrap();
    let mut cmd = poid();
    cmd.args(["--json", "sign"])
        .arg(&out)
        .arg("--key")
        .arg(&bad_key);
    assert_eq!(error_code(cmd), "invalid-key");
}

#[test]
fn stable_error_codes_in_json() {
    let tmp = tempfile::tempdir().unwrap();

    // Not a POID at all.
    let garbage = tmp.path().join("garbage.poid");
    std::fs::write(&garbage, b"hello world").unwrap();
    let mut cmd = poid();
    cmd.args(["--json", "validate"]).arg(&garbage);
    assert_eq!(error_code(cmd), "not-zip");

    // Project without poid.json.
    let empty = tmp.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::write(empty.join("index.html"), "<html>").unwrap();
    let mut cmd = poid();
    cmd.args(["--json", "pack"]).arg(&empty);
    assert_eq!(error_code(cmd), "poid-json-missing");

    // init refuses a non-empty directory.
    let mut cmd = poid();
    cmd.args(["--json", "init"]).arg(&empty);
    assert_eq!(error_code(cmd), "dir-not-empty");
}

#[test]
fn bare_imports_outside_the_stdlib_fail_with_the_dependency_named() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = tmp.path().join("demo");
    poid().arg("init").arg(&demo).assert().success();
    std::fs::write(
        demo.join("main.js"),
        "import tone from \"tone\";\nconsole.log(tone);\n",
    )
    .unwrap();

    let mut cmd = poid();
    cmd.args(["--json", "pack"]).arg(&demo);
    let assert = cmd.assert().failure();
    let v = json_stdout(assert);
    assert_eq!(v["error"]["code"], "unresolved-dependency");
    let message = v["error"]["message"].as_str().unwrap();
    assert!(message.contains("tone"));
    assert!(message.contains("Resolver"), "points at Tier 2");
}

#[test]
fn stdlib_imports_resolve_and_inline_offline() {
    let Some(esbuild) = find_local_esbuild() else {
        eprintln!("skipped: no local esbuild binary found (install JS deps with pnpm to enable)");
        return;
    };
    let stdlib = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/poid-stdlib/lib");
    if !stdlib.join("react.js").is_file() {
        eprintln!("skipped: Standard Library not built (pnpm --filter @poid/stdlib build:lib)");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let demo = tmp.path().join("demo");
    let out = tmp.path().join("demo.poid");
    poid().arg("init").arg(&demo).assert().success();
    std::fs::remove_file(demo.join("main.js")).unwrap();
    std::fs::remove_file(demo.join("counter.js")).unwrap();
    std::fs::write(
        demo.join("main.jsx"),
        "import { useState } from \"react\";\nimport { createRoot } from \"react-dom/client\";\nfunction C() { const [n] = useState(1); return <b>{n}</b>; }\ncreateRoot(document.body).render(<C />);\n",
    )
    .unwrap();

    poid()
        .env("POID_ESBUILD", &esbuild)
        .env("POID_STDLIB", &stdlib)
        .arg("pack")
        .arg(&demo)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    poid().arg("validate").arg(&out).assert().success();

    let v = json_stdout(
        poid()
            .args(["--json", "inspect"])
            .arg(&out)
            .assert()
            .success(),
    );
    let deps: Vec<&str> = v["manifest"]["runtime"]["bundled_deps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap())
        .collect();
    assert!(
        deps.iter().any(|d| d.starts_with("react@")),
        "stdlib selections recorded in bundled_deps, got {deps:?}"
    );
}

#[test]
fn typescript_without_esbuild_fails_with_instructions() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = tmp.path().join("demo");
    poid().arg("init").arg(&demo).assert().success();
    std::fs::write(
        demo.join("main.ts"),
        "const x: number = 1;\nconsole.log(x);\n",
    )
    .unwrap();

    let mut cmd = poid();
    cmd.env("POID_ESBUILD", tmp.path().join("nonexistent-esbuild"));
    cmd.args(["--json", "pack"]).arg(&demo);
    assert_eq!(error_code(cmd), "esbuild-missing");
}

#[test]
fn typescript_bundles_when_esbuild_is_available() {
    let Some(esbuild) = find_local_esbuild() else {
        eprintln!("skipped: no local esbuild binary found (install JS deps with pnpm to enable)");
        return;
    };

    let tmp = tempfile::tempdir().unwrap();
    let demo = tmp.path().join("demo");
    let out = tmp.path().join("demo.poid");
    poid().arg("init").arg(&demo).assert().success();
    std::fs::remove_file(demo.join("main.js")).unwrap();
    std::fs::remove_file(demo.join("counter.js")).unwrap();
    std::fs::write(
        demo.join("main.ts"),
        "const el = document.getElementById(\"app\") as HTMLElement;\nel.textContent = \"typed\";\n",
    )
    .unwrap();

    poid()
        .env("POID_ESBUILD", &esbuild)
        .arg("pack")
        .arg(&demo)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    poid().arg("validate").arg(&out).assert().success();

    let v = json_stdout(
        poid()
            .args(["--json", "inspect"])
            .arg(&out)
            .assert()
            .success(),
    );
    assert!(
        v["manifest"]["runtime"]["toolchain"]["esbuild"].is_string(),
        "esbuild version must be recorded in runtime.toolchain"
    );
    let paths: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(
        paths.contains(&"app/index.html"),
        "the bundle is inlined into the document (M06), got {paths:?}"
    );
    assert!(
        !paths.contains(&"app/main.js"),
        "no separate bundle file — readers execute inline output until #5"
    );
    assert!(
        !paths.iter().any(|p| p.ends_with(".ts")),
        "sources consumed"
    );
}

#[test]
fn conformance_runner_passes_the_committed_suite() {
    let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/conformance");
    let v = json_stdout(
        poid()
            .args(["--json", "conformance"])
            .arg(&suite)
            .assert()
            .success(),
    );
    assert_eq!(v["failed"], 0);
    assert!(v["total"].as_u64().unwrap() >= 27);
}

#[test]
fn conformance_runner_fails_on_a_lying_suite() {
    let tmp = tempfile::tempdir().unwrap();
    let suite = tmp.path();
    std::fs::create_dir_all(suite.join("valid")).unwrap();
    std::fs::create_dir_all(suite.join("invalid")).unwrap();
    // A fixture that claims to be valid but is garbage.
    std::fs::write(suite.join("valid/liar.poid"), b"not a poid at all").unwrap();
    std::fs::write(
        suite.join("valid/liar.expected.json"),
        br#"{ "valid": true }"#,
    )
    .unwrap();

    let v = json_stdout(
        poid()
            .args(["--json", "conformance"])
            .arg(suite)
            .assert()
            .failure(),
    );
    assert_eq!(v["failed"], 1);
    assert_eq!(v["results"][0]["got"], "POID-003");

    // A fixture without its expected.json makes the suite itself invalid.
    std::fs::write(suite.join("invalid/orphan.poid"), b"x").unwrap();
    let mut cmd = poid();
    cmd.args(["--json", "conformance"]).arg(suite);
    assert_eq!(error_code(cmd), "suite-invalid");
}

#[test]
fn errors_carry_registry_codes() {
    let tmp = tempfile::tempdir().unwrap();
    let garbage = tmp.path().join("garbage.poid");
    std::fs::write(&garbage, b"hello").unwrap();
    let mut cmd = poid();
    cmd.args(["--json", "validate"]).arg(&garbage);
    let v = json_stdout(cmd.assert().failure());
    assert_eq!(v["error"]["code"], "not-zip");
    assert_eq!(v["error"]["poid"], "POID-003");
}

/// Finds the esbuild binary that pnpm installed for the JS workspace, if any.
fn find_local_esbuild() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("POID_TEST_ESBUILD") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let exe = if cfg!(windows) {
        "esbuild.exe"
    } else {
        "bin/esbuild"
    };
    let direct = repo_root.join("node_modules/@esbuild");
    let mut candidates = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&direct) {
        for entry in entries.flatten() {
            candidates.push(entry.path().join(exe));
        }
    }
    if let Ok(entries) = std::fs::read_dir(repo_root.join("node_modules/.pnpm")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("@esbuild+") {
                if let Ok(inner) = std::fs::read_dir(entry.path().join("node_modules/@esbuild")) {
                    for pkg in inner.flatten() {
                        candidates.push(pkg.path().join(exe));
                    }
                }
            }
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

#[test]
fn python_wheels_pack_under_the_container_deps_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = tmp.path().join("pydemo");
    let out = tmp.path().join("pydemo.poid");
    poid()
        .args(["init", "--template", "python"])
        .arg(&demo)
        .assert()
        .success();
    // A stand-in wheel: wheels are ZIPs; content is irrelevant to packing.
    std::fs::create_dir_all(demo.join("deps")).unwrap();
    let wheel = demo.join("deps/demo-1.0.0-py3-none-any.whl");
    let cursor = std::io::Cursor::new(Vec::new());
    let mut zipw = zip::ZipWriter::new(cursor);
    zipw.start_file::<_, ()>("demo/__init__.py", zip::write::FileOptions::default())
        .unwrap();
    std::io::Write::write_all(&mut zipw, b"VALUE = 1\n").unwrap();
    let bytes = zipw.finish().unwrap().into_inner();
    std::fs::write(&wheel, bytes).unwrap();

    poid()
        .arg("pack")
        .arg(&demo)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    poid().arg("validate").arg(&out).assert().success();

    let v = json_stdout(
        poid()
            .args(["--json", "inspect"])
            .arg(&out)
            .assert()
            .success(),
    );
    assert_eq!(v["manifest"]["runtime"]["profile"], "web+python");
    assert!(
        v["manifest"]["runtime"]["engines"]["pyodide"].is_string(),
        "the engine range travels in the manifest"
    );
    let paths: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(
        paths.contains(&"deps/demo-1.0.0-py3-none-any.whl"),
        "wheels live at the container root deps/, got {paths:?}"
    );
}

// ---------------------------------------------------------------------------
// M06 Definition of Done
// ---------------------------------------------------------------------------

/// Writes a `create-vite --template react-ts`-shaped project: the tooling
/// noise (package.json, vite.config.ts importing "vite") a real scaffold
/// carries, plus a component that imports a Standard Library package.
fn write_vite_react_project(dir: &std::path::Path) {
    std::fs::write(
        dir.join("package.json"),
        r#"{"name":"demo","private":true,"type":"module","scripts":{"dev":"vite"},"dependencies":{"react":"^18.3.1","react-dom":"^18.3.1"},"devDependencies":{"vite":"^5.3.0","@vitejs/plugin-react":"^4.3.0"}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("vite.config.ts"),
        "import { defineConfig } from \"vite\";\nimport react from \"@vitejs/plugin-react\";\nexport default defineConfig({ plugins: [react()] });\n",
    )
    .unwrap();
    std::fs::write(dir.join("tsconfig.json"), "{}").unwrap();
    std::fs::write(
        dir.join("index.html"),
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"UTF-8\" /><title>Vite + React</title></head>\n<body><div id=\"root\"></div><script type=\"module\" src=\"/src/main.tsx\"></script></body></html>\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/main.tsx"),
        "import { StrictMode } from \"react\";\nimport { createRoot } from \"react-dom/client\";\nimport App from \"./App\";\nimport \"./index.css\";\n\ncreateRoot(document.getElementById(\"root\")!).render(\n  <StrictMode>\n    <App />\n  </StrictMode>,\n);\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/App.tsx"),
        "import { useState } from \"react\";\n\nexport default function App() {\n  const [count, setCount] = useState(0);\n  return (\n    <button onClick={() => setCount((c) => c + 1)}>count is {count}</button>\n  );\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/index.css"),
        ":root { color-scheme: light dark; }\nbutton { font-weight: 600; }\n",
    )
    .unwrap();
}

fn env_for_stdlib_build(cmd: &mut Command) {
    if let Some(esbuild) = find_local_esbuild() {
        cmd.env("POID_ESBUILD", esbuild);
    }
    let stdlib = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/poid-stdlib/lib");
    cmd.env("POID_STDLIB", stdlib);
}

/// DoD: "A React project from create-vite -> .poid -> runs. No Node.js on
/// the machine." -- the sidecar and Standard Library are the only
/// prerequisites; nothing here shells out to npm/node.
#[test]
fn dod_vite_react_project_converts_and_runs() {
    if find_local_esbuild().is_none() {
        eprintln!("skipped: no local esbuild binary found (install JS deps with pnpm to enable)");
        return;
    }
    if !Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/poid-stdlib/lib/react.js")
        .is_file()
    {
        eprintln!("skipped: Standard Library not built (pnpm --filter @poid/stdlib build:lib)");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("vite-react-demo");
    std::fs::create_dir_all(&project).unwrap();
    write_vite_react_project(&project);
    let out = tmp.path().join("demo.poid");

    let mut cmd = poid();
    env_for_stdlib_build(&mut cmd);
    cmd.arg("convert").arg(&project).arg("-o").arg(&out);
    cmd.assert().success();

    poid().arg("validate").arg(&out).assert().success();

    let v = json_stdout(
        poid()
            .args(["--json", "inspect"])
            .arg(&out)
            .assert()
            .success(),
    );
    // Tooling noise never entered the container.
    let paths: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(
        !paths.iter().any(|p| p.contains("vite.config")),
        "{paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.ends_with("package.json")),
        "{paths:?}"
    );
    assert!(paths.contains(&"app/index.html"));

    // The permission set is the most restrictive one that still works --
    // this app touches no capability, so it requests none.
    let perms = &v["manifest"]["permissions"];
    assert_eq!(perms["network"], serde_json::json!([]));
    assert_eq!(perms["clipboard"], false);

    // react + react-dom (+ scheduler, react's own dependency) are recorded.
    let deps: Vec<&str> = v["manifest"]["runtime"]["bundled_deps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap())
        .collect();
    assert!(deps.iter().any(|d| d.starts_with("react@")), "{deps:?}");
    assert!(deps.iter().any(|d| d.starts_with("react-dom@")), "{deps:?}");

    // The inlined document actually contains the rendered app's source, CSS
    // included, self-contained -- this is what "runs" means for a reader
    // that executes inline content (M06 decision 1 / issue #5).
    let extracted = tmp.path().join("extracted");
    poid()
        .arg("extract")
        .arg(&out)
        .arg("-o")
        .arg(&extracted)
        .assert()
        .success();
    let html = std::fs::read_to_string(extracted.join("app/index.html")).unwrap();
    assert!(html.contains("count is"));
    assert!(html.contains("color-scheme"), "CSS is inlined too");
    assert!(html.contains("<script type=\"module\">"));
}

/// DoD: "Building the same input twice produces byte-identical output" --
/// exercised on the M06 surface (Standard Library resolution + inline
/// assembly), not just the M02 static template.
#[test]
fn dod_stdlib_build_is_deterministic() {
    if find_local_esbuild().is_none() {
        eprintln!("skipped: no local esbuild binary found (install JS deps with pnpm to enable)");
        return;
    }
    if !Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/poid-stdlib/lib/react.js")
        .is_file()
    {
        eprintln!("skipped: Standard Library not built (pnpm --filter @poid/stdlib build:lib)");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("vite-react-demo");
    std::fs::create_dir_all(&project).unwrap();
    write_vite_react_project(&project);

    let out1 = tmp.path().join("a.poid");
    let out2 = tmp.path().join("b.poid");
    let mut cmd1 = poid();
    env_for_stdlib_build(&mut cmd1);
    cmd1.arg("convert").arg(&project).arg("-o").arg(&out1);
    cmd1.assert().success();

    let mut cmd2 = poid();
    env_for_stdlib_build(&mut cmd2);
    cmd2.arg("convert").arg(&project).arg("-o").arg(&out2);
    cmd2.assert().success();

    assert_eq!(
        std::fs::read(&out1).unwrap(),
        std::fs::read(&out2).unwrap(),
        "converting the same project twice must be byte-identical"
    );
}

/// DoD: "A Claude artifact (single JSX file) -> .poid -> runs."
#[test]
fn dod_single_jsx_artifact_converts_and_runs() {
    if find_local_esbuild().is_none() {
        eprintln!("skipped: no local esbuild binary found (install JS deps with pnpm to enable)");
        return;
    }
    if !Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/poid-stdlib/lib/react.js")
        .is_file()
    {
        eprintln!("skipped: Standard Library not built (pnpm --filter @poid/stdlib build:lib)");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let artifact = tmp.path().join("TipCalculator.jsx");
    std::fs::write(
        &artifact,
        "import { useState } from \"react\";\n\nexport default function TipCalculator() {\n  const [bill, setBill] = useState(100);\n  const tip = Math.round(bill * 0.18);\n  return (\n    <div>\n      <input value={bill} onChange={(e) => setBill(Number(e.target.value))} />\n      <p>Suggested tip: {tip}</p>\n    </div>\n  );\n}\n",
    )
    .unwrap();
    let out = tmp.path().join("tip-calculator.poid");

    let mut cmd = poid();
    env_for_stdlib_build(&mut cmd);
    cmd.arg("convert").arg(&artifact).arg("-o").arg(&out);
    cmd.assert().success();
    poid().arg("validate").arg(&out).assert().success();

    let v = json_stdout(
        poid()
            .args(["--json", "inspect"])
            .arg(&out)
            .assert()
            .success(),
    );
    // slug_of lowercases but does not split CamelCase (no separator to key
    // off of) - "TipCalculator" -> "tipcalculator".
    assert_eq!(v["manifest"]["app"]["id"], "local.poid.tipcalculator");

    let extracted = tmp.path().join("extracted");
    poid()
        .arg("extract")
        .arg(&out)
        .arg("-o")
        .arg(&extracted)
        .assert()
        .success();
    let html = std::fs::read_to_string(extracted.join("app/index.html")).unwrap();
    assert!(html.contains("Suggested tip"));
}

/// DoD: "A Python + pandas + matplotlib script -> .poid -> runs offline, no
/// pip."
///
/// The offline **execution** half of this claim is fully proven:
/// `packages/poid-host/e2e/python.spec.ts` runs this exact pandas +
/// matplotlib fixture in a real browser — verified engine, wheels installed
/// from local files, zero PyPI/network contact.
///
/// The **packing** half — routing these same wheels through `poid pack` —
/// is currently blocked by a genuine, documented limitation rather than
/// silently narrowed to a lighter example: Pyodide's numpy wheel (a hard
/// transitive dependency of pandas) bundles `numpy/core/lib/libnpymath.a`,
/// a Unix `ar`-format static archive. Its members are verified WebAssembly
/// object code (`\0asm` magic; confirmed zero ELF/PE/Mach-O members), but
/// `poid-core`'s POID-020 native-code check currently rejects on the outer
/// `ar` magic bytes alone (`!<arch>`), the same signature a real native
/// static library would have — the two are byte-indistinguishable without
/// inspecting archive members. Weakening that check is a change to the
/// format's foundational safety rule ("no native code, ever") and needs its
/// own deliberate milestone, not a side effect of M06 packaging plumbing —
/// tracked in issue #9. This test asserts today's conservative, honest
/// behavior: the CLI refuses rather than silently admitting an archive it
/// cannot fully vouch for.
#[test]
fn python_pack_correctly_refuses_a_wasm_ar_archive_pending_issue_9() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/poid-host/e2e/fixtures/python-chart");
    if !fixture.join("deps").is_dir() {
        eprintln!(
            "skipped: wheels not fetched (node scripts/fetch-wheels.mjs packages/poid-host/e2e/fixtures/python-chart)"
        );
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("python-chart");
    poid()
        .args(["init", "--template", "python"])
        .arg(&project)
        .assert()
        .success();
    std::fs::copy(fixture.join("main.py"), project.join("main.py")).unwrap();
    std::fs::create_dir_all(project.join("deps")).unwrap();
    for entry in std::fs::read_dir(fixture.join("deps")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), project.join("deps").join(entry.file_name())).unwrap();
    }

    let out = tmp.path().join("python-chart.poid");
    let mut cmd = poid();
    cmd.args(["--json", "pack"])
        .arg(&project)
        .arg("-o")
        .arg(&out);
    let assert = cmd.assert().failure();
    let v = json_stdout(assert);
    assert_eq!(v["error"]["code"], "native-code");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("libnpymath.a"),
        "{v}"
    );
}

/// A minimal `web+python` project with wheels that carry no compiled
/// extensions packs cleanly end to end — the deps/ passthrough itself
/// (proven with a stand-in wheel in `python_wheels_pack_under_the_container_deps_tree`)
/// works for the realistic case too; only archives with genuinely
/// undecidable content (issue #9) are refused.
#[test]
fn python_pack_succeeds_for_pure_python_wheels() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/poid-host/e2e/fixtures/python-chart");
    if !fixture.join("deps").is_dir() {
        eprintln!(
            "skipped: wheels not fetched (node scripts/fetch-wheels.mjs packages/poid-host/e2e/fixtures/python-chart)"
        );
        return;
    }
    let pure_python_wheels: Vec<_> = std::fs::read_dir(fixture.join("deps"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            // "py2.py3-none-any" / "py3-none-any" wheels carry no compiled
            // extensions at all — pytz, six, python-dateutil and friends.
            name.contains("-none-any.whl")
        })
        .collect();
    assert!(
        !pure_python_wheels.is_empty(),
        "fixture has pure-python wheels"
    );

    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("python-pure");
    poid()
        .args(["init", "--template", "python"])
        .arg(&project)
        .assert()
        .success();
    std::fs::create_dir_all(project.join("deps")).unwrap();
    for wheel in &pure_python_wheels {
        std::fs::copy(wheel, project.join("deps").join(wheel.file_name().unwrap())).unwrap();
    }

    let out = tmp.path().join("python-pure.poid");
    poid()
        .arg("pack")
        .arg(&project)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    poid().arg("validate").arg(&out).assert().success();

    let v = json_stdout(
        poid()
            .args(["--json", "inspect"])
            .arg(&out)
            .assert()
            .success(),
    );
    let paths: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    let packed_wheels = paths
        .iter()
        .filter(|p| p.starts_with("deps/") && p.ends_with(".whl"))
        .count();
    assert_eq!(packed_wheels, pure_python_wheels.len(), "got {paths:?}");
}
