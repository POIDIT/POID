//! Deterministic container writer (SPEC §2.1).
//!
//! Packing the same input twice produces byte-identical output: entries are
//! sorted, timestamps fixed to the ZIP epoch (1980-01-01), the compression
//! level pinned, and no extra fields are emitted. Reproducibility is a
//! headline property of the format.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use crate::error::PoidError;
use crate::limits::Limits;
use crate::manifest::{ContainerType, Manifest};
use crate::paths;
use crate::read::{scan_content, MANIFEST_NAME, MIMETYPE_NAME};
use crate::MEDIA_TYPE;

/// Pinned DEFLATE level. Changing it would break byte-identical repacking,
/// so it is bumped only together with a `runtime.toolchain` record change.
const DEFLATE_LEVEL: i64 = 6;

/// Collects a manifest and files, then produces container bytes via [`pack`].
///
/// `mimetype` and `manifest.json` are reserved: the writer generates them, so
/// a stale manifest can never be smuggled into a container.
#[derive(Debug, Clone)]
pub struct PoidBuilder {
    pub(crate) manifest: Manifest,
    pub(crate) files: BTreeMap<String, Vec<u8>>,
}

impl PoidBuilder {
    /// Starts a builder for the given manifest.
    pub fn new(manifest: Manifest) -> Self {
        Self {
            manifest,
            files: BTreeMap::new(),
        }
    }

    /// Adds a file at a container path. Rejects invalid paths, the reserved
    /// names, and duplicates.
    pub fn file(
        mut self,
        path: impl Into<String>,
        content: impl Into<Vec<u8>>,
    ) -> Result<Self, PoidError> {
        let path = path.into();
        paths::check_container_path(&path)?;
        if path == MIMETYPE_NAME || path == MANIFEST_NAME {
            return Err(PoidError::InvalidPath {
                path,
                why: "reserved name; the writer generates it",
            });
        }
        if self.files.contains_key(&path) {
            return Err(PoidError::DuplicatePath { path });
        }
        self.files.insert(path, content.into());
        Ok(self)
    }
}

/// Packs a container deterministically (SPEC §2.1).
///
/// Recomputes `integrity` over `app/` and `deps/` (SPEC §3.3), validates the
/// manifest and the same content rules the reader enforces — this writer
/// cannot produce a container that [`crate::open`] would reject.
pub fn pack(builder: PoidBuilder) -> Result<Vec<u8>, PoidError> {
    let PoidBuilder {
        mut manifest,
        files,
    } = builder;

    // Case-insensitive uniqueness, mirroring the reader.
    let mut seen_fold: BTreeSet<String> = BTreeSet::new();
    for path in files.keys() {
        if !seen_fold.insert(path.to_ascii_lowercase()) {
            return Err(PoidError::DuplicatePath { path: path.clone() });
        }
    }

    // Same prohibited-content rules as the reader (SPEC §2.3).
    let limits = Limits::default();
    let mut budget = limits.max_total_uncompressed;
    for (path, content) in &files {
        scan_content(path, content, 1, &limits, &mut budget)?;
    }

    // Recompute integrity (SPEC §3.3); stale digests cannot survive a pack.
    crate::integrity::refresh(&mut manifest, &files);

    manifest.validate()?;

    // Type rules, mirroring the reader (SPEC §2.2, §4.2).
    match manifest.container_type {
        ContainerType::Data => {
            for dir in ["app/", "src/", "deps/"] {
                if files.keys().any(|k| k.starts_with(dir)) {
                    return Err(PoidError::DataContainerWithCode { dir });
                }
            }
        }
        ContainerType::App => {
            if !files.keys().any(|k| k.starts_with("app/")) {
                return Err(PoidError::AppTreeMissing);
            }
            if let Some(entry) = &manifest.entry {
                if !files.contains_key(entry) {
                    return Err(PoidError::EntryMissing {
                        path: entry.clone(),
                    });
                }
            }
        }
        ContainerType::Workspace => {
            if !files.keys().any(|k| k.starts_with("app/")) {
                return Err(PoidError::AppTreeMissing);
            }
        }
    }

    let manifest_json = manifest.to_json_bytes()?;

    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644);
    let deflated = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(DEFLATE_LEVEL))
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644);

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(MIMETYPE_NAME, stored)
        .map_err(zip_error)?;
    writer.write_all(MEDIA_TYPE.as_bytes())?;
    writer
        .start_file(MANIFEST_NAME, deflated)
        .map_err(zip_error)?;
    writer.write_all(&manifest_json)?;
    for (path, content) in &files {
        writer.start_file(path, deflated).map_err(zip_error)?;
        writer.write_all(content)?;
    }
    let cursor = writer.finish().map_err(zip_error)?;
    Ok(cursor.into_inner())
}

fn zip_error(e: zip::result::ZipError) -> PoidError {
    match e {
        zip::result::ZipError::Io(io) => PoidError::Io(io),
        other => PoidError::Io(std::io::Error::other(other.to_string())),
    }
}
