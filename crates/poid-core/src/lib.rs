//! POID container logic: reading, writing and validating `.poid` files.
//!
//! This crate is pure logic — bytes in, structures out. It MUST NOT depend on
//! Tauri, on any UI crate, or on the network (see `CONVENTIONS.md`). It is
//! consumed by the CLI, the desktop app and, via WASM, the Web Reader.

/// The POID specification version implemented by this crate (SPEC §3.1, `poid` field).
pub const SPEC_VERSION: &str = "1.0";

/// The media type of a POID container (SPEC §2.1, the `mimetype` entry).
pub const MEDIA_TYPE: &str = "application/vnd.poid+zip";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_version_matches_manifest_field() {
        assert_eq!(SPEC_VERSION, "1.0");
    }

    #[test]
    fn media_type_has_no_trailing_newline() {
        assert!(!MEDIA_TYPE.ends_with('\n'));
        assert_eq!(MEDIA_TYPE, "application/vnd.poid+zip");
    }
}
