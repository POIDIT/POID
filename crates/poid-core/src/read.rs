//! Container reader: enforce SPEC §2 on untrusted bytes, produce a [`Poid`].
//!
//! Checks run in the order mandated by the milestone spec: `mimetype` magic,
//! then manifest parse + validation, then prohibited content and zip-bomb
//! limits, then type-specific rules. Every rejection carries a stable code.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Seek};

use zip::{CompressionMethod, ZipArchive};

use crate::error::PoidError;
use crate::limits::Limits;
use crate::manifest::{ContainerType, Manifest};
use crate::paths;
use crate::poid::Poid;
use crate::MEDIA_TYPE;

/// Name of the required first entry (SPEC §2.1).
pub(crate) const MIMETYPE_NAME: &str = "mimetype";
/// Name of the manifest entry (SPEC §2.2).
pub(crate) const MANIFEST_NAME: &str = "manifest.json";

/// Executable magic byte prefixes prohibited by SPEC §2.3.
const EXEC_MAGICS: [(&[u8], &str); 6] = [
    (b"MZ", "PE/DOS executable"),
    (&[0x7f, b'E', b'L', b'F'], "ELF binary"),
    (&[0xfe, 0xed, 0xfa], "Mach-O binary"),
    (&[0xce, 0xfa, 0xed, 0xfe], "Mach-O binary"),
    (&[0xcf, 0xfa, 0xed, 0xfe], "Mach-O binary"),
    (b"!<arch>", "static library archive"),
];

/// Opens a container from bytes with [`Limits::default`].
pub fn open(bytes: &[u8]) -> Result<Poid, PoidError> {
    open_with_limits(bytes, &Limits::default())
}

/// Opens a container from a file with [`Limits::default`].
#[cfg(feature = "fs")]
pub fn open_path(p: &std::path::Path) -> Result<Poid, PoidError> {
    open_path_with_limits(p, &Limits::default())
}

/// Opens a container from a file with explicit limits.
#[cfg(feature = "fs")]
pub fn open_path_with_limits(p: &std::path::Path, limits: &Limits) -> Result<Poid, PoidError> {
    let meta = std::fs::metadata(p)?;
    // A container larger than the whole uncompressed budget cannot be valid;
    // refuse before reading it into memory.
    if meta.len() > limits.max_total_uncompressed {
        return Err(PoidError::ZipBomb {
            path: p.display().to_string(),
        });
    }
    let bytes = std::fs::read(p)?;
    open_with_limits(&bytes, limits)
}

/// Opens a container from bytes with explicit limits.
pub fn open_with_limits(bytes: &[u8], limits: &Limits) -> Result<Poid, PoidError> {
    // 1. `mimetype` first, STORED, exact content — checked on the raw local
    //    file header at offset 0, so type detection needs only ~60 bytes and
    //    prepended junk cannot fool it (SPEC §2.1).
    check_mimetype_raw(bytes)?;

    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| PoidError::NotZip)?;
    if archive.len() > limits.max_entries {
        return Err(PoidError::TooManyEntries);
    }

    // Shared decompression budget for the container and everything nested in it.
    let mut budget = limits.max_total_uncompressed;

    // 2. `manifest.json` parses and validates (SPEC §3).
    let manifest = read_manifest(&mut archive, limits, &mut budget)?;
    manifest.validate()?;

    // 3.–4. Full scan: prohibited content and zip-bomb limits (SPEC §2.3).
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut seen_fold: BTreeSet<String> = BTreeSet::new();
    for i in 0..archive.len() {
        // Metadata pass first: `by_index_raw` never touches the decompressor,
        // so an entry with a prohibited compression method is rejected by
        // name instead of failing to open at all.
        let entry = archive
            .by_index_raw(i)
            .map_err(|_| PoidError::CorruptEntry {
                path: format!("entry #{i}"),
            })?;
        let name = entry_name(&entry)?;

        if entry.is_dir() {
            // Directory entries are allowed but not stored; their segments
            // must still be well-formed (minus the trailing slash).
            let trimmed = name.trim_end_matches('/');
            if !trimmed.is_empty() {
                paths::check_container_path(trimmed)?;
            }
            continue;
        }

        paths::check_container_path(&name)?;
        check_not_special(&entry, &name)?;
        check_method(&entry, &name)?;
        let compressed = entry.compressed_size();
        drop(entry);

        let mut entry = archive
            .by_index(i)
            .map_err(|_| PoidError::CorruptEntry { path: name.clone() })?;
        let content = read_limited(&mut entry, &name, compressed, limits, &mut budget)?;
        scan_content(&name, &content, 1, limits, &mut budget)?;

        // Case-insensitive uniqueness so the container extracts identically
        // on case-preserving and case-folding filesystems alike.
        if !seen_fold.insert(name.to_ascii_lowercase()) {
            return Err(PoidError::DuplicatePath { path: name });
        }
        if name != MIMETYPE_NAME && name != MANIFEST_NAME {
            files.insert(name, content);
        }
    }

    // 5. Type-specific rules (SPEC §2.2, §4.2).
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
            // `entry` is Some — validate() guarantees it for type=app.
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

    Ok(Poid::from_parts(manifest, files))
}

/// Raw check of the first local file header (SPEC §2.1).
///
/// Layout: signature(4) version(2) flags(2) method(2) time(2) date(2) crc(4)
/// compressed-size(4) uncompressed-size(4) name-len(2) extra-len(2) name extra data.
fn check_mimetype_raw(bytes: &[u8]) -> Result<(), PoidError> {
    if bytes.len() < 30 || &bytes[0..4] != b"PK\x03\x04" {
        return Err(PoidError::NotZip);
    }
    let u16_at = |off: usize| u16::from_le_bytes([bytes[off], bytes[off + 1]]) as usize;
    let flags = u16_at(6);
    let method = u16_at(8);
    let name_len = u16_at(26);
    let extra_len = u16_at(28);

    let name = bytes.get(30..30 + name_len).ok_or(PoidError::NotZip)?;
    if name != MIMETYPE_NAME.as_bytes() {
        return Err(PoidError::MimetypeNotFirst {
            found: String::from_utf8_lossy(name).into_owned(),
        });
    }
    if method != 0 || extra_len != 0 || flags & 0x0008 != 0 {
        // Compressed, carrying an extra field, or streamed with a data
        // descriptor (size unknown up front) — all defeat magic-byte
        // detection by a plain byte scan.
        return Err(PoidError::MimetypeNotStored);
    }
    let size = u16_at(18) | (u16_at(20) << 16); // compressed size, little-endian u32
    let content = bytes
        .get(30 + name_len..30 + name_len + size)
        .ok_or(PoidError::NotZip)?;
    if content != MEDIA_TYPE.as_bytes() {
        return Err(PoidError::MimetypeMismatch);
    }
    Ok(())
}

fn read_manifest<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    limits: &Limits,
    budget: &mut u64,
) -> Result<Manifest, PoidError> {
    let corrupt = || PoidError::CorruptEntry {
        path: MANIFEST_NAME.to_owned(),
    };
    let index = archive
        .index_for_name(MANIFEST_NAME)
        .ok_or(PoidError::ManifestMissing)?;
    let entry = archive.by_index_raw(index).map_err(|_| corrupt())?;
    check_method(&entry, MANIFEST_NAME)?;
    let compressed = entry.compressed_size();
    drop(entry);
    let mut entry = archive.by_index(index).map_err(|_| corrupt())?;
    let bytes = read_limited(&mut entry, MANIFEST_NAME, compressed, limits, budget)?;
    Ok(Manifest::parse(&bytes)?)
}

fn entry_name<R: Read>(entry: &zip::read::ZipFile<'_, R>) -> Result<String, PoidError> {
    let raw = entry.name_raw();
    String::from_utf8(raw.to_vec()).map_err(|_| PoidError::InvalidPath {
        path: String::from_utf8_lossy(raw).into_owned(),
        why: "non-UTF-8 name",
    })
}

fn check_not_special<R: Read>(
    entry: &zip::read::ZipFile<'_, R>,
    name: &str,
) -> Result<(), PoidError> {
    if entry.is_symlink() {
        return Err(PoidError::Link {
            path: name.to_owned(),
        });
    }
    if let Some(mode) = entry.unix_mode() {
        // Anything that is not a plain file or directory (symlink, fifo,
        // socket, device, …) is prohibited (SPEC §2.3).
        let file_type = mode & 0o170_000;
        if file_type != 0 && file_type != 0o100_000 && file_type != 0o040_000 {
            return Err(PoidError::Link {
                path: name.to_owned(),
            });
        }
    }
    Ok(())
}

fn check_method<R: Read>(entry: &zip::read::ZipFile<'_, R>, name: &str) -> Result<(), PoidError> {
    match entry.compression() {
        CompressionMethod::Stored | CompressionMethod::Deflated => Ok(()),
        _ => Err(PoidError::UnsupportedCompression {
            path: name.to_owned(),
        }),
    }
}

/// Streams an entry with the zip-bomb defence: the declared size is never
/// trusted; bytes are counted as they decompress, against both the shared
/// budget and the per-entry ratio cap (SPEC §2.3).
fn read_limited<R: Read>(
    reader: &mut R,
    name: &str,
    compressed_size: u64,
    limits: &Limits,
    budget: &mut u64,
) -> Result<Vec<u8>, PoidError> {
    let ratio_cap = compressed_size
        .saturating_mul(limits.max_compression_ratio)
        .max(limits.ratio_grace_bytes);
    let mut out = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = reader.read(&mut buf).map_err(|_| PoidError::CorruptEntry {
            path: name.to_owned(),
        })?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > *budget || total > ratio_cap {
            return Err(PoidError::ZipBomb {
                path: name.to_owned(),
            });
        }
        out.extend_from_slice(&buf[..n]);
    }
    *budget -= total;
    Ok(out)
}

/// Scans decompressed content for prohibited payloads (SPEC §2.3): executable
/// magic bytes, and nested archives which are scanned recursively with the
/// shared budget. Python wheels in `deps/` are ZIPs, so nested archives are
/// legitimate and must be inspected, not rejected.
pub(crate) fn scan_content(
    name: &str,
    content: &[u8],
    depth: u8,
    limits: &Limits,
    budget: &mut u64,
) -> Result<(), PoidError> {
    for (magic, kind) in EXEC_MAGICS {
        if content.starts_with(magic) {
            return Err(PoidError::NativeCode {
                path: name.to_owned(),
                kind,
            });
        }
    }
    if content.starts_with(b"PK\x03\x04") {
        if depth >= limits.max_nesting_depth {
            return Err(PoidError::NestedTooDeep {
                path: name.to_owned(),
            });
        }
        scan_nested_zip(name, content, depth + 1, limits, budget)?;
    }
    Ok(())
}

fn scan_nested_zip(
    name: &str,
    content: &[u8],
    depth: u8,
    limits: &Limits,
    budget: &mut u64,
) -> Result<(), PoidError> {
    // Data that merely starts with the ZIP magic but is not a parseable
    // archive is opaque bytes; it already passed the executable-magic scan.
    let Ok(mut archive) = ZipArchive::new(Cursor::new(content)) else {
        return Ok(());
    };
    if archive.len() > limits.max_entries {
        return Err(PoidError::TooManyEntries);
    }
    for i in 0..archive.len() {
        let inner_label = |suffix: &str| format!("{name}!{suffix}");
        // A parseable archive with unreadable entries is content we cannot
        // verify: fail closed. Metadata pass first, as in the outer reader.
        let Ok(entry) = archive.by_index_raw(i) else {
            return Err(PoidError::CorruptEntry {
                path: inner_label(&format!("entry #{i}")),
            });
        };
        let raw_name = String::from_utf8_lossy(entry.name_raw()).into_owned();
        let label = inner_label(&raw_name);
        if entry.is_dir() {
            continue;
        }
        if entry.is_symlink() {
            return Err(PoidError::Link { path: label });
        }
        match entry.compression() {
            CompressionMethod::Stored | CompressionMethod::Deflated => {}
            _ => return Err(PoidError::UnsupportedCompression { path: label }),
        }
        paths::check_container_path(&raw_name).map_err(|_| PoidError::PathTraversal {
            path: label.clone(),
            why: "invalid path inside nested archive",
        })?;
        let compressed = entry.compressed_size();
        drop(entry);
        let Ok(mut entry) = archive.by_index(i) else {
            return Err(PoidError::CorruptEntry { path: label });
        };
        let inner = read_limited(&mut entry, &label, compressed, limits, budget)?;
        scan_content(&label, &inner, depth, limits, budget)?;
    }
    Ok(())
}
