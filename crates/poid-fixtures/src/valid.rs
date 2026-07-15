//! The `spec/conformance/valid/` fixtures — every one must open cleanly.

use poid_core::{
    open, pack, ContainerType, Manifest, PoidBuilder, PoidError, StorageMode, ToolchainRecord, Uuid,
};

use crate::{mini_zip, pseudo_random_bytes, Fixture, TEST_KEY_SEED};

/// A fixed, valid UUIDv4 for fixtures that model an already-opened copy.
const FIXED_INSTANCE_ID: u128 = 0xa1b2c3d4_e5f6_4a7b_8c9d_0e1f2a3b4c5d;

pub(crate) fn all() -> Result<Vec<Fixture>, PoidError> {
    Ok(vec![
        fixture("minimal-html", minimal_html()?),
        fixture("react-app", react_app()?),
        fixture("python-pyodide", python_pyodide()?),
        fixture("embedded-data", embedded_data()?),
        fixture("vault-mode", vault_mode()?),
        fixture("slots", slots()?),
        fixture("protected", protected()?),
        fixture("workspace", workspace()?),
        fixture("data-container", data_container()?),
        fixture("signed", signed()?),
        fixture("unknown-manifest-fields", unknown_manifest_fields()?),
        fixture("large-but-legal", large_but_legal()?),
    ])
}

fn fixture(name: &'static str, bytes: Vec<u8>) -> Fixture {
    Fixture {
        name,
        valid: true,
        bytes,
        expected_code: None,
    }
}

fn app(name_suffix: &str, display: &str) -> Manifest {
    Manifest::new_app(
        format!("org.poid.conformance.{name_suffix}"),
        display,
        "1.0.0",
        "app/index.html",
    )
}

const INDEX_HTML: &[u8] =
    b"<!doctype html><html><head><meta charset=\"utf-8\"></head><body><h1>ok</h1></body></html>";

fn minimal_html() -> Result<Vec<u8>, PoidError> {
    pack(PoidBuilder::new(app("minimal", "Minimal HTML")).file("app/index.html", INDEX_HTML)?)
}

/// Shaped like the build output of a bundled React application: the container
/// stores build output, never source projects (SPEC §5.3), so no actual React
/// is needed to exercise the shape.
fn react_app() -> Result<Vec<u8>, PoidError> {
    let mut manifest = app("react", "React App");
    if let Some(runtime) = &mut manifest.runtime {
        runtime.bundled_deps = Some(vec!["react@18.3.1".into(), "react-dom@18.3.1".into()]);
        runtime.toolchain = Some(ToolchainRecord {
            builder: Some("poid-fixtures@0.0.1".into()),
            esbuild: Some("0.25.0".into()),
            extra: Default::default(),
        });
    }
    pack(
        PoidBuilder::new(manifest)
            .file("app/index.html", &b"<!doctype html><div id=\"root\"></div><script type=\"module\" src=\"main.js\"></script>"[..])?
            .file(
                "app/main.js",
                &b"// bundled output (esbuild); relative imports only\nconst e=document.getElementById(\"root\");e.textContent=\"hello from a bundle\";"[..],
            )?,
    )
}

fn python_pyodide() -> Result<Vec<u8>, PoidError> {
    let mut manifest = app("python", "Python App");
    if let Some(runtime) = &mut manifest.runtime {
        runtime.profile = "web+python".into();
        runtime.engines = Some(
            [("pyodide".to_owned(), ">=0.26 <0.28".to_owned())]
                .into_iter()
                .collect(),
        );
    }
    let wheel = mini_zip(&[
        (
            "hello_poid/__init__.py",
            &b"def greet():\n    return \"hello from a wheel\"\n"[..],
        ),
        (
            "hello_poid-1.0.0.dist-info/METADATA",
            &b"Metadata-Version: 2.1\nName: hello-poid\nVersion: 1.0.0\n"[..],
        ),
    ])?;
    pack(
        PoidBuilder::new(manifest)
            .file("app/index.html", INDEX_HTML)?
            .file(
                "app/main.py",
                &b"import hello_poid\nprint(hello_poid.greet())\n"[..],
            )?
            .file("deps/hello_poid-1.0.0-py3-none-any.whl", wheel)?,
    )
}

fn embedded_data() -> Result<Vec<u8>, PoidError> {
    pack(
        PoidBuilder::new(app("embedded", "Embedded Data"))
            .file("app/index.html", INDEX_HTML)?
            .file(
                "data/store.json",
                &br#"{"todos":[{"id":1,"text":"ship the conformance suite","done":true}]}"#[..],
            )?,
    )
}

fn vault_mode() -> Result<Vec<u8>, PoidError> {
    let mut manifest = app("vault", "Vault Mode");
    if let Some(storage) = &mut manifest.storage {
        storage.mode = StorageMode::Vault;
    }
    if let Some(instance) = &mut manifest.instance {
        // A vault-mode file that has been opened once: identity assigned,
        // data lives in the reader's vault, none travels in the file.
        instance.id = Some(Uuid::from_u128(FIXED_INSTANCE_ID));
    }
    pack(PoidBuilder::new(manifest).file("app/index.html", INDEX_HTML)?)
}

fn slots() -> Result<Vec<u8>, PoidError> {
    let mut manifest = app("slots", "Slots");
    if let Some(storage) = &mut manifest.storage {
        storage.slots = Some(true);
    }
    pack(
        PoidBuilder::new(manifest)
            .file("app/index.html", INDEX_HTML)?
            .file("slots/project-a/store.json", &br#"{"cards":["a"]}"#[..])?
            .file("slots/project-b/store.json", &br#"{"cards":["b"]}"#[..])?,
    )
}

fn protected() -> Result<Vec<u8>, PoidError> {
    let mut manifest = app("protected", "Protected");
    if let Some(storage) = &mut manifest.storage {
        storage.protected = Some(true);
    }
    // Opaque placeholder for encrypted state: SPEC §6.2 requires
    // human-readable JSON *unless* protected. Real AES-256-GCM content
    // arrives with the encryption milestone; container-level conformance
    // only needs the shape.
    let mut blob = vec![0u8];
    blob.extend_from_slice(&pseudo_random_bytes(0x9e37_79b9, 64));
    pack(
        PoidBuilder::new(manifest)
            .file("app/index.html", INDEX_HTML)?
            .file("data/store.json", blob)?,
    )
}

fn workspace() -> Result<Vec<u8>, PoidError> {
    let inner = pack(
        PoidBuilder::new(app("workspace.notes", "Notes")).file("app/index.html", INDEX_HTML)?,
    )?;
    let mut manifest = app("workspace", "Workspace");
    manifest.container_type = ContainerType::Workspace;
    manifest.entry = None;
    pack(
        PoidBuilder::new(manifest)
            .file("app/index.html", INDEX_HTML)?
            .file("apps/notes.poid", inner)?,
    )
}

fn data_container() -> Result<Vec<u8>, PoidError> {
    let manifest = Manifest::new_data("org.poid.conformance.survey", "1.0.0", "responses/v1");
    pack(PoidBuilder::new(manifest).file(
        "data/store.json",
        &br#"{"answers":{"q1":"yes","q2":4}}"#[..],
    )?)
}

fn signed() -> Result<Vec<u8>, PoidError> {
    let bytes =
        pack(PoidBuilder::new(app("signed", "Signed")).file("app/index.html", INDEX_HTML)?)?;
    let mut poid = open(&bytes)?;
    poid.sign(&TEST_KEY_SEED)?;
    poid.to_bytes()
}

fn unknown_manifest_fields() -> Result<Vec<u8>, PoidError> {
    // Forward compatibility (SPEC §3): unknown fields MUST NOT cause
    // rejection and MUST survive round-trips.
    let json = serde_json::json!({
        "poid": "1.0",
        "type": "app",
        "x_future_top_level": { "anything": [1, 2, 3] },
        "app": {
            "id": "org.poid.conformance.forward",
            "name": "Forward Compatible",
            "version": "1.0.0",
            "x_future_app_field": true
        },
        "instance": { "id": null },
        "runtime": { "profile": "web", "x_future_runtime_field": "keep" },
        "entry": "app/index.html",
        "storage": { "mode": "embedded" },
        "permissions": { "network": [], "x_future_permission": null },
        "integrity": { "algo": "sha256" }
    });
    let manifest = Manifest::parse(json.to_string().as_bytes())?;
    pack(PoidBuilder::new(manifest).file("app/index.html", INDEX_HTML)?)
}

fn large_but_legal() -> Result<Vec<u8>, PoidError> {
    let mut builder =
        PoidBuilder::new(app("large", "Large But Legal")).file("app/index.html", INDEX_HTML)?;
    // Incompressible payload: big, but ratio ≈ 1 and far under the absolute
    // budget — generosity of the limits is part of the contract too.
    builder = builder.file(
        "app/assets/blob.bin",
        pseudo_random_bytes(0x00c0_ffee, 1_200_000),
    )?;
    for i in 0..300 {
        builder = builder.file(
            format!("app/files/f{i:03}.txt"),
            format!("file number {i}\n").into_bytes(),
        )?;
    }
    pack(builder)
}
