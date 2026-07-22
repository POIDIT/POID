//! Converter logic (ARCHITECTURE §5, M06): from "whatever the author has" to
//! a build plan and a manifest.
//!
//! Pure functions over an in-memory file map — no filesystem, no network, no
//! build engine. The CLI feeds it files from disk and runs the native
//! esbuild sidecar; Studio and the Web Reader feed it files from a drop and
//! run esbuild-wasm. Both consume the same classification, the same entry
//! detection, the same permission inference and the same manifest — so a
//! project converts identically everywhere.
//!
//! The build itself happens outside: the plan says *what* to build; the
//! caller builds it and hands the outputs to [`assemble`].

mod classify;
mod error;
mod html;
mod imports;
mod infer;
mod manifest;
mod pipeline;
mod stdlib;

pub use classify::{classify, InputKind, ProjectShape, ENTRY_CANDIDATES};
pub use error::ConvertError;
pub use html::{inline_into_html, InlineParts};
pub use imports::bare_imports;
pub use infer::{infer_permissions, InferredPermissions};
pub use manifest::{converted_manifest, slug_of, ConvertPlan};
pub use pipeline::{finish, pack_converted, prepare, BuiltApp, Bundled, Prepared};
pub use stdlib::{resolve, verify_bundle, Resolution, Selection};

/// A project file: relative `/`-separated path plus content.
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Relative path, `/`-separated, no leading slash.
    pub rel: String,
    /// Raw content.
    pub content: Vec<u8>,
}

impl SourceFile {
    /// Convenience constructor.
    pub fn new(rel: impl Into<String>, content: impl Into<Vec<u8>>) -> Self {
        Self {
            rel: rel.into(),
            content: content.into(),
        }
    }
}
