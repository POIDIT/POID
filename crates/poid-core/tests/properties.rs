//! Property-based tests: no input may ever panic the parser, and packing is
//! a lossless, deterministic round-trip.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

#[allow(dead_code)]
mod common;

use std::collections::BTreeMap;

use common::{app_manifest, sample_bytes};
use poid_core::{open, pack, Manifest, PoidBuilder};
use proptest::prelude::*;

proptest! {
    /// DoD: no input causes a panic. Ever. (Random garbage.)
    #[test]
    fn open_never_panics_on_arbitrary_bytes(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
        let _ = open(&bytes);
    }

    /// DoD: no input causes a panic. Ever. (A valid container, randomly
    /// corrupted — this walks far deeper into the parser than pure garbage.)
    #[test]
    fn open_never_panics_on_corrupted_containers(
        flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..24)
    ) {
        let mut bytes = sample_bytes();
        for (index, value) in flips {
            let i = index.index(bytes.len());
            bytes[i] ^= value;
        }
        let _ = open(&bytes);
    }

    /// Truncation at any point must fail cleanly, never panic.
    #[test]
    fn open_never_panics_on_truncation(cut in any::<prop::sample::Index>()) {
        let bytes = sample_bytes();
        let i = cut.index(bytes.len());
        let _ = open(&bytes[..i]);
    }

    /// Arbitrary (safe-path) file sets round-trip losslessly and repack
    /// byte-identically.
    #[test]
    fn pack_open_roundtrip_is_lossless_and_deterministic(
        files in prop::collection::btree_map(
            "[a-z][a-z0-9]{0,7}(/[a-z0-9]{1,8}){0,2}",
            prop::collection::vec(any::<u8>(), 0..512),
            0..12,
        )
    ) {
        let mut builder = PoidBuilder::new(app_manifest())
            .file("app/index.html", b"<html>".to_vec()).unwrap();
        let mut expected: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        expected.insert("app/index.html".to_owned(), b"<html>".to_vec());

        for (path, mut content) in files {
            // A leading NUL byte defeats every executable/archive magic, so
            // arbitrary content cannot trip the (correct) prohibited-content
            // rejection we are not testing here.
            content.insert(0, 0u8);
            let path = format!("app/f/{path}");
            // Case-insensitive duplicates are rejected by design; the
            // generator's lowercase alphabet plus BTreeMap keys make paths
            // unique already.
            builder = builder.file(path.clone(), content.clone()).unwrap();
            expected.insert(path, content);
        }

        let bytes = pack(builder).unwrap();
        let poid = open(&bytes).unwrap();
        poid.verify().unwrap();

        let got: BTreeMap<String, Vec<u8>> = poid
            .files()
            .map(|(p, c)| (p.to_owned(), c.to_vec()))
            .collect();
        prop_assert_eq!(&got, &expected);

        prop_assert_eq!(poid.to_bytes().unwrap(), bytes);
    }

    /// Unknown manifest fields round-trip losslessly at every level.
    #[test]
    fn manifest_extras_roundtrip(
        top in extra_value(),
        nested in extra_value(),
    ) {
        let json = serde_json::json!({
            "poid": "1.0", "type": "app",
            "x_top": top,
            "app": {"id": "com.example.x", "name": "X", "version": "1.0.0", "x_nested": nested},
            "instance": {"id": null},
            "runtime": {"profile": "web"},
            "entry": "app/index.html",
            "storage": {"mode": "embedded"},
            "permissions": {},
            "integrity": {"algo": "sha256"}
        });
        let manifest = Manifest::parse(json.to_string().as_bytes()).unwrap();
        manifest.validate().unwrap();
        let out = manifest.to_json_bytes().unwrap();
        let reparsed = Manifest::parse(&out).unwrap();
        prop_assert_eq!(&manifest, &reparsed);

        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        prop_assert_eq!(&v["x_top"], &json["x_top"]);
        prop_assert_eq!(&v["app"]["x_nested"], &json["app"]["x_nested"]);
    }
}

/// JSON leaves for unknown-field strategies.
fn extra_value() -> impl Strategy<Value = serde_json::Value> {
    prop_oneof![
        any::<bool>().prop_map(serde_json::Value::from),
        any::<i64>().prop_map(serde_json::Value::from),
        "[ -~]{0,24}".prop_map(serde_json::Value::from),
        prop::collection::vec(any::<i32>(), 0..4).prop_map(|v| serde_json::json!(v)),
    ]
}
