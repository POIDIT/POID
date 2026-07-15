//! Hostile fixtures: every one must be rejected with the correct stable code.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

#[allow(dead_code)]
mod common;

use common::{app_manifest, err_code, manifest_json, mini_zip, RawZip};
use poid_core::{open, open_with_limits, pack, Limits, Manifest, PoidBuilder};

#[test]
fn rejects_random_bytes_as_not_zip() {
    assert_eq!(err_code(open(b"hello")), "not-zip");
    assert_eq!(err_code(open(&[])), "not-zip");
}

#[test]
fn rejects_when_first_entry_is_not_mimetype() {
    let bytes = RawZip::new()
        .stored("a.txt", b"x")
        .mimetype()
        .manifest(&app_manifest())
        .stored("app/index.html", b"<html>")
        .build();
    assert_eq!(err_code(open(&bytes)), "mimetype-not-first");
}

#[test]
fn rejects_compressed_mimetype() {
    // The real writer compresses everything except `mimetype`; use it against
    // itself by writing `mimetype` as a normal (deflated) entry.
    let bytes = mini_zip(&[
        ("mimetype", poid_core::MEDIA_TYPE.as_bytes()),
        ("manifest.json", &manifest_json(&app_manifest())),
        ("app/index.html", b"<html>"),
    ]);
    assert_eq!(err_code(open(&bytes)), "mimetype-not-stored");
}

#[test]
fn rejects_mimetype_with_extra_field() {
    let bytes = RawZip::new()
        .stored_with_extra("mimetype", poid_core::MEDIA_TYPE.as_bytes(), &[1, 2, 3, 4])
        .manifest(&app_manifest())
        .stored("app/index.html", b"<html>")
        .build();
    assert_eq!(err_code(open(&bytes)), "mimetype-not-stored");
}

#[test]
fn rejects_mimetype_with_trailing_newline() {
    let bytes = RawZip::new()
        .stored("mimetype", b"application/vnd.poid+zip\n")
        .manifest(&app_manifest())
        .stored("app/index.html", b"<html>")
        .build();
    assert_eq!(err_code(open(&bytes)), "mimetype-mismatch");
}

#[test]
fn rejects_wrong_media_type() {
    let bytes = RawZip::new()
        .stored("mimetype", b"application/zip")
        .manifest(&app_manifest())
        .stored("app/index.html", b"<html>")
        .build();
    assert_eq!(err_code(open(&bytes)), "mimetype-mismatch");
}

#[test]
fn rejects_missing_manifest() {
    let bytes = RawZip::new()
        .mimetype()
        .stored("app/index.html", b"<html>")
        .build();
    assert_eq!(err_code(open(&bytes)), "manifest-missing");
}

#[test]
fn rejects_executable_magic_bytes() {
    let cases: [(&[u8], &str); 6] = [
        (b"MZ\x90\x00\x03rest-of-a-pe-file", "PE"),
        (b"\x7fELF\x02\x01\x01\x00", "ELF"),
        (b"\xfe\xed\xfa\xce\x00\x00", "Mach-O BE"),
        (b"\xce\xfa\xed\xfe\x0c\x00", "Mach-O 32 LE"),
        (b"\xcf\xfa\xed\xfe\x07\x00", "Mach-O 64 LE"),
        (b"!<arch>\nlib.a", "ar archive"),
    ];
    for (payload, label) in cases {
        let bytes = RawZip::new()
            .mimetype()
            .manifest(&app_manifest())
            .stored("app/index.html", b"<html>")
            .stored("app/lib.bin", payload)
            .build();
        assert_eq!(err_code(open(&bytes)), "native-code", "case: {label}");
    }
}

#[test]
fn rejects_executable_hidden_inside_a_wheel() {
    // deps/*.whl are ZIPs and must be scanned, not trusted.
    let wheel = mini_zip(&[
        ("pkg/__init__.py", b"print('hi')"),
        ("pkg/native.so", b"\x7fELF\x02\x01\x01\x00evil"),
    ]);
    let bytes = RawZip::new()
        .mimetype()
        .manifest(&app_manifest())
        .stored("app/index.html", b"<html>")
        .stored("deps/pkg.whl", &wheel)
        .build();
    let err = open(&bytes).expect_err("wheel must be scanned");
    assert_eq!(err.code(), "native-code");
    assert!(
        err.to_string().contains("deps/pkg.whl!pkg/native.so"),
        "error should name the nested path: {err}"
    );
}

#[test]
fn rejects_path_traversal_names() {
    for (name, expected) in [
        ("../evil.txt", "path-traversal"),
        ("app/../../evil.txt", "path-traversal"),
        ("/etc/passwd", "path-traversal"),
        ("C:/windows/evil.txt", "path-traversal"),
        ("app\\evil.txt", "invalid-path"),
    ] {
        let bytes = RawZip::new()
            .mimetype()
            .manifest(&app_manifest())
            .stored("app/index.html", b"<html>")
            .stored(name, b"x")
            .build();
        assert_eq!(err_code(open(&bytes)), expected, "case: {name}");
    }
}

#[test]
fn rejects_symlinks() {
    let bytes = RawZip::new()
        .mimetype()
        .manifest(&app_manifest())
        .stored("app/index.html", b"<html>")
        .symlink("app/link", "/etc/passwd")
        .build();
    assert_eq!(err_code(open(&bytes)), "link");
}

#[test]
fn rejects_duplicate_paths_case_insensitively() {
    let bytes = RawZip::new()
        .mimetype()
        .manifest(&app_manifest())
        .stored("app/index.html", b"<html>")
        .stored("app/Readme.txt", b"a")
        .stored("app/readme.txt", b"b")
        .build();
    assert_eq!(err_code(open(&bytes)), "duplicate-path");
}

#[test]
fn rejects_unsupported_compression_method() {
    let bytes = RawZip::new()
        .mimetype()
        .manifest(&app_manifest())
        .stored("app/index.html", b"<html>")
        .with_method("app/data.bin", 12, b"pretend-bzip2")
        .build();
    assert_eq!(err_code(open(&bytes)), "unsupported-compression");
}

#[test]
fn rejects_zip_bomb_by_ratio_with_default_limits() {
    // 8 MiB of zeros deflate to a few KiB: far beyond 100:1 and beyond the
    // 1 MiB grace floor, so default limits must refuse it.
    let zeros = vec![0u8; 8 * 1024 * 1024];
    let bytes = mini_zip_container(&zeros);
    assert_eq!(err_code(open(&bytes)), "zip-bomb-ratio");
}

#[test]
fn rejects_zip_bomb_by_absolute_budget() {
    let zeros = vec![0u8; 4 * 1024 * 1024];
    let bytes = mini_zip_container(&zeros);
    let limits = Limits {
        max_total_uncompressed: 1024 * 1024,
        ..Limits::default()
    };
    assert_eq!(err_code(open_with_limits(&bytes, &limits)), "zip-bomb-size");
}

/// A container written with the real zip writer: conformant mimetype, valid
/// manifest, plus a highly compressible payload.
fn mini_zip_container(payload: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    w.start_file("mimetype", stored).unwrap();
    w.write_all(poid_core::MEDIA_TYPE.as_bytes()).unwrap();
    let deflated = zip::write::SimpleFileOptions::default();
    w.start_file("manifest.json", deflated).unwrap();
    w.write_all(&manifest_json(&app_manifest())).unwrap();
    w.start_file("app/index.html", deflated).unwrap();
    w.write_all(b"<html>").unwrap();
    w.start_file("app/zeros.bin", deflated).unwrap();
    w.write_all(payload).unwrap();
    w.finish().unwrap().into_inner()
}

#[test]
fn rejects_archives_nested_too_deep() {
    // Container (level 1) → nested.zip (2) → f2.zip (3) → f3.zip would be
    // level 4: one past the default limit.
    let level4 = mini_zip(&[("f4.txt", b"bottom")]);
    let level3 = mini_zip(&[("f3.zip", &level4)]);
    let level2 = mini_zip(&[("f2.zip", &level3)]);
    let bytes = RawZip::new()
        .mimetype()
        .manifest(&app_manifest())
        .stored("app/index.html", b"<html>")
        .stored("deps/nested.zip", &level2)
        .build();
    assert_eq!(err_code(open(&bytes)), "nested-too-deep");

    // Exactly at the limit (a ZIP inside a wheel) is fine.
    let ok_level3 = mini_zip(&[("data.zip", &mini_zip(&[("f.txt", b"x")]))]);
    let bytes = RawZip::new()
        .mimetype()
        .manifest(&app_manifest())
        .stored("app/index.html", b"<html>")
        .stored("deps/pkg.whl", &ok_level3)
        .build();
    open(&bytes).expect("nesting at the limit must open");
}

#[test]
fn rejects_data_container_with_code_trees() {
    let data_manifest = Manifest::new_data("com.example.survey", "1.0.0", "responses/v1");
    let bytes = RawZip::new()
        .mimetype()
        .manifest(&data_manifest)
        .stored("app/index.html", b"<html>")
        .build();
    assert_eq!(err_code(open(&bytes)), "data-container-with-code");

    // The writer refuses to produce one, too.
    let builder = PoidBuilder::new(Manifest::new_data("com.example.survey", "1.0.0", "v1"))
        .file("deps/x.whl", b"\x00not-a-real-wheel".to_vec())
        .unwrap();
    assert_eq!(err_code(pack(builder)), "data-container-with-code");
}

#[test]
fn rejects_missing_entry_file() {
    let bytes = RawZip::new()
        .mimetype()
        .manifest(&app_manifest()) // entry: app/index.html
        .stored("app/other.html", b"<html>")
        .build();
    assert_eq!(err_code(open(&bytes)), "entry-missing");
}

#[test]
fn rejects_app_without_app_tree() {
    let bytes = RawZip::new()
        .mimetype()
        .manifest(&app_manifest())
        .stored("assets/icon.svg", b"<svg/>")
        .build();
    assert_eq!(err_code(open(&bytes)), "app-tree-missing");
}

#[test]
fn manifest_rejections_have_stable_codes() {
    let with_manifest = |json: &[u8]| {
        RawZip::new()
            .mimetype()
            .stored("manifest.json", json)
            .stored("app/index.html", b"<html>")
            .build()
    };

    assert_eq!(
        err_code(open(&with_manifest(b"{ not json"))),
        "manifest-syntax"
    );

    let v2 = serde_json::json!({
        "poid": "2.0", "type": "app",
        "app": {"id": "com.example.x", "name": "X", "version": "1.0.0"},
        "instance": {"id": null},
        "runtime": {"profile": "web"},
        "entry": "app/index.html",
        "storage": {"mode": "embedded"},
        "permissions": {},
        "integrity": {"algo": "sha256"}
    });
    assert_eq!(
        err_code(open(&with_manifest(v2.to_string().as_bytes()))),
        "manifest-unsupported-version"
    );

    let connection = serde_json::json!({
        "poid": "1.0", "type": "app",
        "app": {"id": "com.example.x", "name": "X", "version": "1.0.0"},
        "instance": {"id": null},
        "runtime": {"profile": "web"},
        "entry": "app/index.html",
        "storage": {"mode": "connection"},
        "permissions": {},
        "integrity": {"algo": "sha256"}
    });
    assert_eq!(
        err_code(open(&with_manifest(connection.to_string().as_bytes()))),
        "manifest-connection-requires"
    );

    let no_entry = serde_json::json!({
        "poid": "1.0", "type": "app",
        "app": {"id": "com.example.x", "name": "X", "version": "1.0.0"},
        "instance": {"id": null},
        "runtime": {"profile": "web"},
        "storage": {"mode": "embedded"},
        "permissions": {},
        "integrity": {"algo": "sha256"}
    });
    assert_eq!(
        err_code(open(&with_manifest(no_entry.to_string().as_bytes()))),
        "manifest-missing-field"
    );
}
