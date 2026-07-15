//! POID container logic: reading, writing and validating `.poid` files.
//!
//! This crate is pure logic — bytes in, structures out. It MUST NOT depend on
//! Tauri, on any UI crate, or on the network (see `CONVENTIONS.md`). It is
//! consumed by the CLI, the desktop app and, via WASM, the Web Reader: with
//! `--no-default-features` it builds for `wasm32-unknown-unknown` (the `fs`
//! feature gates everything that touches a filesystem).
//!
//! # Reading
//!
//! [`open`] enforces SPEC §2 on untrusted bytes, in order: the `mimetype`
//! magic, manifest parsing and validation, prohibited content (executable
//! magic bytes, links, path traversal), zip-bomb limits, and type-specific
//! rules. Every rejection is a [`PoidError`] with a stable code.
//!
//! # Writing
//!
//! [`PoidBuilder`] + [`pack`] produce deterministic output: packing the same
//! input twice yields byte-identical containers. Integrity digests (SPEC
//! §3.3) are recomputed on every pack.

mod error;
mod integrity;
mod limits;
mod manifest;
mod paths;
mod poid;
mod read;
mod write;

pub use error::{ManifestError, PoidError};
pub use limits::Limits;
pub use manifest::{
    AppInfo, ContainerType, DataRef, ExtraFields, FilesystemAccess, Instance, Integrity,
    IntegrityAlgo, Manifest, Permissions, RequireKind, Runtime, Storage, StorageMode,
    StorageRequires, ToolchainRecord, WindowHints,
};
pub use poid::Poid;
pub use read::{open, open_with_limits};
#[cfg(feature = "fs")]
pub use read::{open_path, open_path_with_limits};
pub use write::{pack, PoidBuilder};

// Re-exported so callers pass ids without adding their own `uuid` dependency.
pub use uuid::Uuid;

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
