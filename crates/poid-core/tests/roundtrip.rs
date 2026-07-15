//! Determinism, lossless round-trips, mutations, integrity, atomic save.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

#[allow(dead_code)]
mod common;

use common::{app_manifest, err_code, sample_bytes, RawZip};
use poid_core::{open, pack, Manifest, PoidBuilder, StorageMode, Uuid};

#[test]
fn packing_twice_is_byte_identical() {
    // Reproducibility is a headline property of the format.
    assert_eq!(sample_bytes(), sample_bytes());
}

#[test]
fn open_then_repack_is_byte_identical() {
    let bytes = sample_bytes();
    let poid = open(&bytes).unwrap();
    assert_eq!(poid.to_bytes().unwrap(), bytes);
}

#[test]
fn mimetype_is_first_stored_and_detectable_in_60_bytes() {
    let bytes = sample_bytes();
    // SPEC §2.1: type detection by reading the first ~60 bytes.
    assert_eq!(&bytes[0..4], b"PK\x03\x04");
    let name_len = u16::from_le_bytes([bytes[26], bytes[27]]) as usize;
    assert_eq!(&bytes[30..30 + name_len], b"mimetype");
    let content = &bytes[30 + name_len..30 + name_len + poid_core::MEDIA_TYPE.len()];
    assert_eq!(content, poid_core::MEDIA_TYPE.as_bytes());
}

#[test]
fn open_restores_manifest_and_files() {
    let poid = open(&sample_bytes()).unwrap();
    assert_eq!(
        poid.manifest().app.as_ref().unwrap().id,
        "com.example.kanban"
    );
    assert_eq!(
        poid.file("app/index.html").unwrap(),
        b"<!doctype html><h1>kanban</h1>"
    );
    assert_eq!(poid.data().unwrap(), br#"{"cards":[]}"#);
    // mimetype and manifest.json are generated, not stored as files
    assert!(poid.file("mimetype").is_none());
    assert!(poid.file("manifest.json").is_none());
}

#[test]
fn unknown_manifest_fields_survive_a_full_container_roundtrip() {
    let json = serde_json::json!({
        "poid": "1.0", "type": "app",
        "x_vendor": {"anything": [1, 2, 3]},
        "app": {"id": "com.example.x", "name": "X", "version": "1.0.0", "x_nested": "keep"},
        "instance": {"id": null, "x_inst": 7},
        "runtime": {"profile": "web", "x_rt": false},
        "entry": "app/index.html",
        "storage": {"mode": "embedded", "x_st": "yes"},
        "permissions": {"network": [], "x_perm": null},
        "integrity": {"algo": "sha256"}
    });
    let manifest = Manifest::parse(json.to_string().as_bytes()).unwrap();
    let bytes = pack(
        PoidBuilder::new(manifest)
            .file("app/index.html", b"<html>".to_vec())
            .unwrap(),
    )
    .unwrap();

    let reopened = open(&bytes).unwrap();
    let out: serde_json::Value =
        serde_json::from_slice(&reopened.manifest().to_json_bytes().unwrap()).unwrap();
    assert_eq!(out["x_vendor"]["anything"][2], 3);
    assert_eq!(out["app"]["x_nested"], "keep");
    assert_eq!(out["instance"]["x_inst"], 7);
    assert_eq!(out["runtime"]["x_rt"], false);
    assert_eq!(out["storage"]["x_st"], "yes");
    assert!(out["permissions"].get("x_perm").is_some());
}

#[test]
fn set_instance_id_survives_reopen() {
    let mut poid = open(&sample_bytes()).unwrap();
    assert_eq!(poid.manifest().instance.as_ref().unwrap().id, None);

    let id = Uuid::new_v4();
    poid.set_instance_id(id);
    let reopened = open(&poid.to_bytes().unwrap()).unwrap();
    assert_eq!(reopened.manifest().instance.as_ref().unwrap().id, Some(id));
}

#[test]
fn set_data_and_clear_data() {
    let mut poid = open(&sample_bytes()).unwrap();
    poid.set_instance_id(Uuid::new_v4());
    poid.set_data(br#"{"cards":["x"]}"#);
    assert_eq!(poid.data().unwrap(), br#"{"cards":["x"]}"#);

    // "Duplicate as empty" (SPEC §6.3): data gone, instance.id back to null.
    poid.clear_data();
    assert!(poid.data().is_none());
    assert_eq!(poid.manifest().instance.as_ref().unwrap().id, None);

    let reopened = open(&poid.to_bytes().unwrap()).unwrap();
    assert!(reopened.data().is_none());
}

#[test]
fn convert_storage_mode_drops_embedded_data_on_the_way_out() {
    let mut poid = open(&sample_bytes()).unwrap();
    assert!(poid.data().is_some());

    poid.convert_storage_mode(StorageMode::Vault);
    assert_eq!(
        poid.manifest().storage.as_ref().unwrap().mode,
        StorageMode::Vault
    );
    assert!(poid.data().is_none(), "data/ must not travel with the file");

    // Back to embedded: the mode flips, data is written by the caller.
    poid.convert_storage_mode(StorageMode::Embedded);
    poid.set_data(b"{}");
    let reopened = open(&poid.to_bytes().unwrap()).unwrap();
    assert_eq!(reopened.data().unwrap(), b"{}");
}

#[test]
fn verify_accepts_fresh_pack_and_detects_tampering() {
    let poid = open(&sample_bytes()).unwrap();
    poid.verify().expect("fresh pack must verify");

    // Rebuild the container with the same manifest (same digests) but altered
    // app content — the digest check must catch it.
    let manifest = poid.manifest().clone();
    let tampered = RawZip::new()
        .mimetype()
        .manifest(&manifest)
        .stored("app/index.html", b"<!doctype html><h1>TAMPERED</h1>")
        .stored("app/main.js", b"console.log('hi')")
        .stored("assets/icon.svg", b"<svg xmlns='x'/>")
        .stored("data/store.json", br#"{"cards":[]}"#)
        .build();
    let opened = open(&tampered).expect("opens; verify is a separate step");
    assert_eq!(err_code(opened.verify()), "integrity-mismatch");
}

#[test]
fn integrity_ignores_user_data_by_design() {
    // Consent is keyed to the app hash (SECURITY §5): writing user data must
    // not change the app/deps digests.
    let mut poid = open(&sample_bytes()).unwrap();
    let before = poid.manifest().integrity.clone().unwrap();
    poid.set_data(b"{\"changed\":true}");
    let reopened = open(&poid.to_bytes().unwrap()).unwrap();
    let after = reopened.manifest().integrity.clone().unwrap();
    assert_eq!(before.app, after.app);
    assert_eq!(before.deps, after.deps);
    reopened.verify().expect("still verifies");
}

#[test]
fn data_container_roundtrip() {
    let manifest = Manifest::new_data("com.example.survey", "1.0.0", "responses/v1");
    let bytes = pack(
        PoidBuilder::new(manifest)
            .file("data/store.json", br#"{"answers":[1,2]}"#.to_vec())
            .unwrap(),
    )
    .unwrap();
    let poid = open(&bytes).unwrap();
    assert_eq!(poid.data().unwrap(), br#"{"answers":[1,2]}"#);
    poid.verify()
        .expect("no app/deps trees, nothing to mismatch");
}

#[test]
fn save_path_is_atomic_and_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("kanban.poid");

    let poid = open(&sample_bytes()).unwrap();
    poid.save_path(&target).unwrap();
    let reopened = poid_core::open_path(&target).unwrap();
    assert_eq!(reopened.to_bytes().unwrap(), sample_bytes());

    // Overwriting an existing file must replace it cleanly.
    let mut mutated = open(&sample_bytes()).unwrap();
    mutated.set_data(b"{\"n\":1}");
    mutated.save_path(&target).unwrap();
    let after = poid_core::open_path(&target).unwrap();
    assert_eq!(after.data().unwrap(), b"{\"n\":1}");

    // No stray temp files left behind.
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|n| n != "kanban.poid")
        .collect();
    assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
}

#[test]
fn writer_refuses_reserved_names_and_traversal() {
    let b = PoidBuilder::new(app_manifest());
    assert_eq!(
        err_code(b.clone().file("manifest.json", b"{}".to_vec())),
        "invalid-path"
    );
    assert_eq!(
        err_code(b.clone().file("mimetype", b"x".to_vec())),
        "invalid-path"
    );
    assert_eq!(
        err_code(b.clone().file("../evil", b"x".to_vec())),
        "path-traversal"
    );
    assert_eq!(
        err_code(pack(
            b.file("app/evil.exe", b"MZ\x90\x00".to_vec()).unwrap()
        )),
        "native-code"
    );
}
