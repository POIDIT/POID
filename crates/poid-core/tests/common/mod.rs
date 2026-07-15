//! Shared fixtures for poid-core's integration tests. The raw ZIP builder
//! lives in `poid-fixtures` (it also generates the conformance suite).

use poid_core::{pack, Manifest, PoidBuilder, PoidError};

// Not every test crate that includes this module touches RawZip.
#[allow(unused_imports)]
pub use poid_fixtures::RawZip;

/// A minimal valid `type: app` manifest.
pub fn app_manifest() -> Manifest {
    Manifest::new_app("com.example.kanban", "Kanban", "1.0.0", "app/index.html")
}

/// A builder holding a small valid application.
pub fn sample_builder() -> PoidBuilder {
    PoidBuilder::new(app_manifest())
        .file("app/index.html", b"<!doctype html><h1>kanban</h1>".to_vec())
        .unwrap()
        .file("app/main.js", b"console.log('hi')".to_vec())
        .unwrap()
        .file("assets/icon.svg", b"<svg xmlns='x'/>".to_vec())
        .unwrap()
        .file("data/store.json", br#"{"cards":[]}"#.to_vec())
        .unwrap()
}

/// Bytes of the sample application container.
pub fn sample_bytes() -> Vec<u8> {
    pack(sample_builder()).unwrap()
}

/// JSON bytes of a manifest.
pub fn manifest_json(m: &Manifest) -> Vec<u8> {
    m.to_json_bytes().unwrap()
}

/// Asserts the result failed and returns its stable error code.
pub fn err_code<T: std::fmt::Debug>(r: Result<T, PoidError>) -> &'static str {
    r.expect_err("expected rejection").code()
}

/// Builds a small ZIP in memory with the real `zip` writer (deflate).
pub fn mini_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    poid_fixtures::mini_zip(entries).unwrap()
}
