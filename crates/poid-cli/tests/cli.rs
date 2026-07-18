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
