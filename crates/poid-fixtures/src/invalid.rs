//! The `spec/conformance/invalid/` fixtures — every one must be rejected
//! with exactly the registry code its `expected.json` names.

use poid_core::{open, pack, ContainerType, Manifest, PoidBuilder, PoidError, SIGNATURE_PATH};

use crate::{Fixture, RawZip, TEST_KEY_SEED};

pub(crate) fn all() -> Result<Vec<Fixture>, PoidError> {
    Ok(vec![
        fixture("no-mimetype", "POID-001", no_mimetype()),
        fixture("mimetype-not-first", "POID-002", mimetype_not_first()),
        fixture("mimetype-compressed", "POID-002", mimetype_compressed()),
        fixture("bad-manifest", "POID-011", bad_manifest()),
        fixture("schema-violation", "POID-012", schema_violation()),
        fixture("contains-exe", "POID-020", contains_exe()),
        fixture("contains-elf", "POID-020", contains_elf()),
        fixture("symlink", "POID-021", symlink()),
        fixture("path-traversal", "POID-022", path_traversal()),
        fixture("zip-bomb", "POID-023", zip_bomb()?),
        fixture("integrity-mismatch", "POID-031", integrity_mismatch()?),
        fixture(
            "data-container-with-code",
            "POID-040",
            data_container_with_code(),
        ),
        fixture("entry-missing", "POID-030", entry_missing()),
        fixture("tampered-signature", "POID-050", tampered_signature()?),
        fixture(
            "workspace-missing-apps",
            "POID-041",
            workspace_missing_apps(),
        ),
    ])
}

fn fixture(name: &'static str, code: &'static str, bytes: Vec<u8>) -> Fixture {
    Fixture {
        name,
        valid: false,
        bytes,
        expected_code: Some(code),
    }
}

fn app(name_suffix: &str) -> Manifest {
    Manifest::new_app(
        format!("org.poid.conformance.{name_suffix}"),
        "Invalid Fixture",
        "1.0.0",
        "app/index.html",
    )
}

const INDEX_HTML: &[u8] = b"<!doctype html><h1>invalid fixture</h1>";

fn no_mimetype() -> Vec<u8> {
    RawZip::new()
        .manifest(&app("no-mimetype"))
        .stored("app/index.html", INDEX_HTML)
        .build()
}

fn mimetype_not_first() -> Vec<u8> {
    RawZip::new()
        .stored("app/index.html", INDEX_HTML)
        .mimetype()
        .manifest(&app("not-first"))
        .build()
}

fn mimetype_compressed() -> Vec<u8> {
    // Method 8 (DEFLATE) claimed for mimetype; content bytes are irrelevant —
    // the raw header check must reject before reading them.
    RawZip::new()
        .with_method("mimetype", 8, b"\x01\x02\x03")
        .manifest(&app("compressed"))
        .stored("app/index.html", INDEX_HTML)
        .build()
}

fn bad_manifest() -> Vec<u8> {
    RawZip::new()
        .mimetype()
        .stored("manifest.json", b"{ this is not json")
        .stored("app/index.html", INDEX_HTML)
        .build()
}

fn schema_violation() -> Vec<u8> {
    // type=app without the required `entry` field (SPEC §3.1).
    let json = serde_json::json!({
        "poid": "1.0",
        "type": "app",
        "app": { "id": "org.poid.conformance.schema", "name": "Schema", "version": "1.0.0" },
        "instance": { "id": null },
        "runtime": { "profile": "web" },
        "storage": { "mode": "embedded" },
        "permissions": { "network": [] },
        "integrity": { "algo": "sha256" }
    });
    RawZip::new()
        .mimetype()
        .stored("manifest.json", json.to_string().as_bytes())
        .stored("app/index.html", INDEX_HTML)
        .build()
}

fn contains_exe() -> Vec<u8> {
    RawZip::new()
        .mimetype()
        .manifest(&app("exe"))
        .stored("app/index.html", INDEX_HTML)
        .stored(
            "app/tool.exe",
            b"MZ\x90\x00\x03\x00\x00\x00rest-of-a-pe-file",
        )
        .build()
}

fn contains_elf() -> Vec<u8> {
    RawZip::new()
        .mimetype()
        .manifest(&app("elf"))
        .stored("app/index.html", INDEX_HTML)
        .stored("app/libnative.so", b"\x7fELF\x02\x01\x01\x00payload")
        .build()
}

fn symlink() -> Vec<u8> {
    RawZip::new()
        .mimetype()
        .manifest(&app("symlink"))
        .stored("app/index.html", INDEX_HTML)
        .symlink("app/link", "/etc/passwd")
        .build()
}

fn path_traversal() -> Vec<u8> {
    RawZip::new()
        .mimetype()
        .manifest(&app("traversal"))
        .stored("app/index.html", INDEX_HTML)
        .stored("../escape.txt", b"outside the container")
        .build()
}

/// 8 MiB of zeros deflate to a few KiB — far beyond the 100:1 ratio cap.
fn zip_bomb() -> Result<Vec<u8>, PoidError> {
    use std::io::Write;
    let manifest_json = app("bomb").to_json_bytes().map_err(PoidError::from)?;
    let stored = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6))
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let zerr = |e: zip::result::ZipError| PoidError::Io(std::io::Error::other(e.to_string()));
    w.start_file("mimetype", stored).map_err(zerr)?;
    w.write_all(poid_core::MEDIA_TYPE.as_bytes())?;
    w.start_file("manifest.json", deflated).map_err(zerr)?;
    w.write_all(&manifest_json)?;
    w.start_file("app/index.html", deflated).map_err(zerr)?;
    w.write_all(INDEX_HTML)?;
    w.start_file("app/zeros.bin", deflated).map_err(zerr)?;
    w.write_all(&vec![0u8; 8 * 1024 * 1024])?;
    Ok(w.finish().map_err(zerr)?.into_inner())
}

/// The manifest's digests describe different content than the container
/// carries; opening succeeds, integrity verification must fail.
fn integrity_mismatch() -> Result<Vec<u8>, PoidError> {
    let genuine =
        pack(PoidBuilder::new(app("integrity")).file("app/index.html", &b"genuine content"[..])?)?;
    let manifest_with_genuine_digests = open(&genuine)?.manifest().clone();
    Ok(RawZip::new()
        .mimetype()
        .manifest(&manifest_with_genuine_digests)
        .stored("app/index.html", b"tampered content")
        .build())
}

fn data_container_with_code() -> Vec<u8> {
    let manifest = Manifest::new_data("org.poid.conformance.data", "1.0.0", "responses/v1");
    RawZip::new()
        .mimetype()
        .manifest(&manifest)
        .stored("app/index.html", INDEX_HTML)
        .build()
}

fn entry_missing() -> Vec<u8> {
    // Manifest promises app/index.html; the container carries only other files.
    RawZip::new()
        .mimetype()
        .manifest(&app("entry-missing"))
        .stored("app/other.html", INDEX_HTML)
        .build()
}

/// Integrity digests are refreshed for the new content, so POID-031 passes —
/// but the signature still covers the original digests: POID-050.
fn tampered_signature() -> Result<Vec<u8>, PoidError> {
    let manifest = app("tampered");
    let original =
        pack(PoidBuilder::new(manifest.clone()).file("app/index.html", &b"original content"[..])?)?;
    let mut signed = open(&original)?;
    signed.sign(&TEST_KEY_SEED)?;
    let signature_block = signed
        .file(SIGNATURE_PATH)
        .ok_or(PoidError::SignatureInvalid)?
        .to_vec();

    pack(
        PoidBuilder::new(manifest)
            .file("app/index.html", &b"attacker-modified content"[..])?
            .file(SIGNATURE_PATH, signature_block)?,
    )
}

fn workspace_missing_apps() -> Vec<u8> {
    let mut manifest = app("workspace-empty");
    manifest.container_type = ContainerType::Workspace;
    manifest.entry = None;
    RawZip::new()
        .mimetype()
        .manifest(&manifest)
        .stored("app/shell.html", INDEX_HTML)
        .build()
}
