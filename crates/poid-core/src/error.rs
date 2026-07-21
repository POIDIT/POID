//! Typed errors with stable, machine-readable codes.
//!
//! Every rejection maps to a code via [`PoidError::code`]. The codes are part
//! of the conformance contract (SPEC §14): once published they never change,
//! so the conformance suite can assert on them across implementations.

use thiserror::Error;

/// Errors produced when reading, validating, writing or mutating a `.poid` container.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PoidError {
    /// The bytes are not a ZIP archive at all.
    #[error("not a ZIP archive")]
    NotZip,
    /// The archive contains no `mimetype` entry at all (SPEC §2.1).
    #[error("the archive has no `mimetype` entry (SPEC 2.1)")]
    MimetypeMissing,
    /// The first ZIP entry is not named `mimetype` (SPEC §2.1).
    #[error("first ZIP entry must be `mimetype`, found `{found}` (SPEC 2.1)")]
    MimetypeNotFirst {
        /// Name of the entry that was found first instead.
        found: String,
    },
    /// The `mimetype` entry is compressed, streamed, or carries an extra field (SPEC §2.1).
    #[error("`mimetype` must be STORED, with a known size and no extra field (SPEC 2.1)")]
    MimetypeNotStored,
    /// The `mimetype` content is not exactly the POID media type (SPEC §2.1).
    #[error(
        "`mimetype` content must be exactly `{}` with no trailing newline (SPEC 2.1)",
        crate::MEDIA_TYPE
    )]
    MimetypeMismatch,
    /// The container has no `manifest.json` (SPEC §2.2).
    #[error("container has no manifest.json (SPEC 2.2)")]
    ManifestMissing,
    /// The manifest failed to parse or violates a SPEC rule.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// An entry starts with executable magic bytes (SPEC §2.3).
    #[error(
        "`{path}` looks like native code ({kind}); a POID must not contain machine code (SPEC 2.3)"
    )]
    NativeCode {
        /// Container path of the offending entry (nested archives use `outer!inner`).
        path: String,
        /// Human-readable name of the detected format.
        kind: &'static str,
    },
    /// An entry is a symbolic link, hard link, or other special file (SPEC §2.3).
    #[error("`{path}` is a link or special file; these are prohibited (SPEC 2.3)")]
    Link {
        /// Container path of the offending entry.
        path: String,
    },
    /// An entry path escapes the container (SPEC §2.3).
    #[error("`{path}` escapes the container ({why}) (SPEC 2.3)")]
    PathTraversal {
        /// The offending path.
        path: String,
        /// Which traversal rule it violates.
        why: &'static str,
    },
    /// An entry path is malformed (empty segment, backslash, non-UTF-8, …).
    #[error("`{path}` is not a valid container path ({why})")]
    InvalidPath {
        /// The offending path.
        path: String,
        /// Which well-formedness rule it violates.
        why: &'static str,
    },
    /// Two entries resolve to the same path (case-insensitively, so the
    /// container extracts identically on every filesystem).
    #[error("duplicate entry `{path}`")]
    DuplicatePath {
        /// The path that appears more than once.
        path: String,
    },
    /// An entry uses a compression method other than STORED or DEFLATE.
    #[error("`{path}` uses an unsupported compression method")]
    UnsupportedCompression {
        /// Container path of the offending entry.
        path: String,
    },
    /// An entry's decompression ratio exceeded the cap (SPEC §2.3).
    #[error("`{path}` exceeds the decompression ratio cap; refusing a likely zip bomb (SPEC 2.3)")]
    ZipBombRatio {
        /// Container path of the entry that blew the ratio cap.
        path: String,
    },
    /// The container exceeded the absolute uncompressed size budget (SPEC §2.3).
    #[error(
        "`{path}` exceeds the uncompressed size budget; refusing a likely zip bomb (SPEC 2.3)"
    )]
    ZipBombSize {
        /// Container path of the entry that blew the budget.
        path: String,
    },
    /// The container has more entries than [`crate::Limits::max_entries`].
    #[error("too many entries in the container")]
    TooManyEntries,
    /// Nested archives exceed [`crate::Limits::max_nesting_depth`].
    #[error("`{path}`: nested archives exceed the allowed depth")]
    NestedTooDeep {
        /// Container path of the archive that nests too deep.
        path: String,
    },
    /// An entry declared by the archive cannot be decoded.
    #[error("`{path}` is corrupt and cannot be read")]
    CorruptEntry {
        /// Container path of the unreadable entry.
        path: String,
    },
    /// A `type: data` container carries code trees (SPEC §4.2).
    #[error("a data container must not contain `{dir}` (SPEC 4.2)")]
    DataContainerWithCode {
        /// The prohibited tree that was found.
        dir: &'static str,
    },
    /// The `app/` tree required for this container type is missing (SPEC §2.2).
    #[error("this container type requires an `app/` tree (SPEC 2.2)")]
    AppTreeMissing,
    /// A workspace has no nested POIDs under `apps/` (SPEC §4.3).
    #[error("a workspace must contain at least one nested POID under `apps/` (SPEC 4.3)")]
    WorkspaceAppsMissing,
    /// The manifest `entry` file does not exist in the container.
    #[error("manifest entry `{path}` does not exist in the container")]
    EntryMissing {
        /// The declared entry path.
        path: String,
    },
    /// Stored integrity digests do not match the content (SPEC §3.3).
    #[error("integrity digest mismatch for the `{tree}` tree (SPEC 3.3)")]
    IntegrityMismatch {
        /// Which tree failed: `app` or `deps`.
        tree: &'static str,
    },
    /// The signature is well-formed but does not verify (SPEC §9.3.2).
    #[error("the signature does not match the content; the file was modified after signing or the signature is forged (SPEC 9.3)")]
    SignatureInvalid,
    /// The signature file exists but cannot be understood (SPEC §9.3.1).
    #[error("signature/signature.json is malformed: {reason}")]
    SignatureMalformed {
        /// What is wrong with the block.
        reason: String,
    },
    /// An "update program, keep data" (SPEC §12) was attempted between two
    /// containers whose `app.id` differ, or that are not both `type: app`.
    #[error("cannot update: {why}")]
    UpdateMismatch {
        /// Why the two containers cannot be an update pair.
        why: String,
    },
    /// Filesystem error (only reachable with the `fs` feature).
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

impl PoidError {
    /// The stable machine-readable code for this error.
    ///
    /// Codes are part of the conformance contract (SPEC §14) and never change
    /// once published.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotZip => "not-zip",
            Self::MimetypeMissing => "mimetype-missing",
            Self::MimetypeNotFirst { .. } => "mimetype-not-first",
            Self::MimetypeNotStored => "mimetype-not-stored",
            Self::MimetypeMismatch => "mimetype-mismatch",
            Self::ManifestMissing => "manifest-missing",
            Self::Manifest(e) => e.code(),
            Self::NativeCode { .. } => "native-code",
            Self::Link { .. } => "link",
            Self::PathTraversal { .. } => "path-traversal",
            Self::InvalidPath { .. } => "invalid-path",
            Self::DuplicatePath { .. } => "duplicate-path",
            Self::UnsupportedCompression { .. } => "unsupported-compression",
            Self::ZipBombRatio { .. } => "zip-bomb-ratio",
            Self::ZipBombSize { .. } => "zip-bomb-size",
            Self::TooManyEntries => "too-many-entries",
            Self::NestedTooDeep { .. } => "nested-too-deep",
            Self::CorruptEntry { .. } => "corrupt-entry",
            Self::DataContainerWithCode { .. } => "data-container-with-code",
            Self::AppTreeMissing => "app-tree-missing",
            Self::WorkspaceAppsMissing => "workspace-apps-missing",
            Self::EntryMissing { .. } => "entry-missing",
            Self::IntegrityMismatch { .. } => "integrity-mismatch",
            Self::SignatureInvalid => "signature-invalid",
            Self::SignatureMalformed { .. } => "signature-malformed",
            Self::UpdateMismatch { .. } => "update-mismatch",
            Self::Io(_) => "io",
        }
    }

    /// The normative conformance registry code (`POID-xxx`, `spec/errors.md`).
    ///
    /// Registry codes are coarser than [`Self::code`]: several diagnostic
    /// codes may share one registry entry. `None` for environmental errors
    /// (I/O) that say nothing about the container itself.
    pub fn conformance_code(&self) -> Option<&'static str> {
        Some(match self {
            Self::MimetypeMissing | Self::MimetypeMismatch => "POID-001",
            Self::MimetypeNotFirst { .. } | Self::MimetypeNotStored => "POID-002",
            Self::NotZip => "POID-003",
            Self::CorruptEntry { .. } => "POID-004",
            Self::ManifestMissing => "POID-010",
            Self::Manifest(e) => e.conformance_code(),
            Self::NativeCode { .. } => "POID-020",
            Self::Link { .. } => "POID-021",
            Self::PathTraversal { .. } | Self::InvalidPath { .. } => "POID-022",
            Self::ZipBombRatio { .. } => "POID-023",
            Self::ZipBombSize { .. } => "POID-024",
            Self::UnsupportedCompression { .. } => "POID-025",
            Self::NestedTooDeep { .. } => "POID-026",
            Self::DuplicatePath { .. } => "POID-027",
            Self::TooManyEntries => "POID-028",
            Self::EntryMissing { .. } => "POID-030",
            Self::IntegrityMismatch { .. } => "POID-031",
            Self::AppTreeMissing => "POID-032",
            Self::DataContainerWithCode { .. } => "POID-040",
            Self::WorkspaceAppsMissing => "POID-041",
            Self::SignatureInvalid => "POID-050",
            Self::SignatureMalformed { .. } => "POID-051",
            // Not a container-validity failure: an operation refused a bad
            // update pair. No conformance registry entry.
            Self::UpdateMismatch { .. } => return None,
            Self::Io(_) => return None,
        })
    }
}

/// Errors produced by manifest parsing and SPEC §3 validation.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ManifestError {
    /// The manifest is not valid JSON, or a field has the wrong JSON type.
    #[error("manifest.json is malformed: {message}")]
    Syntax {
        /// The underlying parser message.
        message: String,
    },
    /// The `poid` field names a spec version this reader does not implement.
    #[error(
        "unsupported spec version `{found}`; this implementation supports `{}`",
        crate::SPEC_VERSION
    )]
    UnsupportedVersion {
        /// The version string found in the manifest.
        found: String,
    },
    /// A field required for this container type is absent (SPEC §3.1).
    #[error("`{field}` is required for this container type (SPEC 3.1)")]
    MissingField {
        /// Dotted path of the missing field.
        field: &'static str,
    },
    /// An identifier is not reverse-DNS shaped (SPEC §3.1).
    #[error("`{field}` must be a reverse-DNS identifier like `com.example.app`, found `{value}` (SPEC 3.1)")]
    InvalidId {
        /// Dotted path of the offending field.
        field: &'static str,
        /// The value found.
        value: String,
    },
    /// A version is not a semantic version (SPEC §3.1).
    #[error("`{field}` must be a semantic version like `1.2.3`, found `{value}` (SPEC 3.1)")]
    InvalidVersion {
        /// Dotted path of the offending field.
        field: &'static str,
        /// The value found.
        value: String,
    },
    /// A path field does not stay inside the container (SPEC §2.3).
    #[error("`{field}` must be a relative path inside the container, found `{value}` (SPEC 2.3)")]
    InvalidPath {
        /// Dotted path of the offending field.
        field: &'static str,
        /// The value found.
        value: String,
    },
    /// `runtime.profile` is not `web` or `web+<engine>` (SPEC §5.4).
    #[error("`runtime.profile` must be `web` or `web+<engine>`, found `{value}` (SPEC 5.4)")]
    InvalidProfile {
        /// The value found.
        value: String,
    },
    /// `storage.mode` is `connection` but `storage.requires` is absent (SPEC §3.1).
    #[error("`storage.mode = \"connection\"` requires `storage.requires` (SPEC 3.1)")]
    ConnectionRequires,
    /// An integrity digest is not 64 lowercase hex characters (SPEC §3.3).
    #[error("`{field}` must be a 64-character lowercase hex SHA-256 digest (SPEC 3.3)")]
    InvalidDigest {
        /// Dotted path of the offending field.
        field: &'static str,
    },
    /// A numeric field is out of range.
    #[error("`{field}` must be at least 1")]
    InvalidNumber {
        /// Dotted path of the offending field.
        field: &'static str,
    },
}

impl ManifestError {
    /// The normative conformance registry code (see [`PoidError::conformance_code`]).
    pub fn conformance_code(&self) -> &'static str {
        match self {
            Self::Syntax { .. } => "POID-011",
            _ => "POID-012",
        }
    }

    /// The stable machine-readable code for this error (see [`PoidError::code`]).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Syntax { .. } => "manifest-syntax",
            Self::UnsupportedVersion { .. } => "manifest-unsupported-version",
            Self::MissingField { .. } => "manifest-missing-field",
            Self::InvalidId { .. } => "manifest-invalid-id",
            Self::InvalidVersion { .. } => "manifest-invalid-version",
            Self::InvalidPath { .. } => "manifest-invalid-path",
            Self::InvalidProfile { .. } => "manifest-invalid-profile",
            Self::ConnectionRequires => "manifest-connection-requires",
            Self::InvalidDigest { .. } => "manifest-invalid-digest",
            Self::InvalidNumber { .. } => "manifest-invalid-number",
        }
    }
}
