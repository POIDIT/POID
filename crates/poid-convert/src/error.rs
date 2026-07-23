//! The converter's error type: a stable code plus a human message.
//!
//! The code is the same slug the CLI has always surfaced (`bundle-entry-missing`,
//! `unresolved-dependency`, `stdlib-checksum-mismatch`, …), so moving the
//! pipeline here does not change what a caller sees. The CLI maps this to its
//! own `CmdError`; Studio maps it to an IPC error string.

use std::fmt;

/// A conversion failure: a stable machine-readable code and a message written
/// for a person.
#[derive(Debug, Clone)]
pub struct ConvertError {
    /// Stable slug, e.g. `bundle-entry-missing`.
    pub code: String,
    /// Human-readable explanation (no stack traces, no jargon).
    pub message: String,
}

impl ConvertError {
    /// Builds a converter error.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ConvertError {}

impl From<poid_core::ManifestError> for ConvertError {
    fn from(e: poid_core::ManifestError) -> Self {
        Self::new("manifest-invalid", e.to_string())
    }
}

impl From<poid_core::PoidError> for ConvertError {
    fn from(e: poid_core::PoidError) -> Self {
        Self::new(e.code().to_owned(), e.to_string())
    }
}
