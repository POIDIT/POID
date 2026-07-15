//! A raw, byte-level ZIP builder with no sanity checks — it exists to craft
//! containers that violate the spec in precisely controlled ways. All entries
//! are STORED, so every byte is deterministic.

use poid_core::{Manifest, MEDIA_TYPE};

/// Builds a ZIP archive byte-by-byte, entry order preserved.
#[derive(Default)]
pub struct RawZip {
    entries: Vec<RawEntry>,
}

struct RawEntry {
    name: Vec<u8>,
    method: u16,
    crc: u32,
    data: Vec<u8>,
    uncompressed_len: u32,
    external_attrs: u32,
    extra: Vec<u8>,
}

impl RawZip {
    /// An empty archive.
    pub fn new() -> Self {
        Self::default()
    }

    /// A conformant `mimetype` first entry (SPEC §2.1).
    pub fn mimetype(self) -> Self {
        self.stored("mimetype", MEDIA_TYPE.as_bytes())
    }

    /// A `manifest.json` entry serialized from the given manifest.
    // Serializing an in-memory manifest to JSON cannot fail; unwrap-in-builder
    // keeps the fixture-definition call sites readable.
    #[allow(clippy::expect_used)]
    pub fn manifest(self, m: &Manifest) -> Self {
        let json = m
            .to_json_bytes()
            .expect("in-memory manifest always serializes");
        self.stored("manifest.json", &json)
    }

    /// A STORED entry with a regular-file unix mode.
    pub fn stored(self, name: &str, content: &[u8]) -> Self {
        self.entry(name.as_bytes(), 0, content, 0o100_644 << 16, Vec::new())
    }

    /// A STORED entry with an extra field in the local header.
    pub fn stored_with_extra(self, name: &str, content: &[u8], extra: &[u8]) -> Self {
        self.entry(name.as_bytes(), 0, content, 0o100_644 << 16, extra.to_vec())
    }

    /// An entry claiming an arbitrary compression method (data left raw).
    pub fn with_method(self, name: &str, method: u16, content: &[u8]) -> Self {
        self.entry(
            name.as_bytes(),
            method,
            content,
            0o100_644 << 16,
            Vec::new(),
        )
    }

    /// A symbolic-link entry.
    pub fn symlink(self, name: &str, target: &str) -> Self {
        self.entry(
            name.as_bytes(),
            0,
            target.as_bytes(),
            0o120_777 << 16,
            Vec::new(),
        )
    }

    fn entry(
        mut self,
        name: &[u8],
        method: u16,
        data: &[u8],
        external_attrs: u32,
        extra: Vec<u8>,
    ) -> Self {
        self.entries.push(RawEntry {
            name: name.to_vec(),
            method,
            crc: crc32fast::hash(data),
            data: data.to_vec(),
            uncompressed_len: data.len() as u32,
            external_attrs,
            extra,
        });
        self
    }

    /// Serializes the archive.
    pub fn build(self) -> Vec<u8> {
        fn u16le(out: &mut Vec<u8>, v: u16) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        fn u32le(out: &mut Vec<u8>, v: u32) {
            out.extend_from_slice(&v.to_le_bytes());
        }

        let mut out = Vec::new();
        let mut offsets = Vec::new();
        for e in &self.entries {
            offsets.push(out.len() as u32);
            out.extend_from_slice(b"PK\x03\x04");
            u16le(&mut out, 20); // version needed
            u16le(&mut out, 0); // flags
            u16le(&mut out, e.method);
            u16le(&mut out, 0); // mod time
            u16le(&mut out, 0x21); // mod date: 1980-01-01
            u32le(&mut out, e.crc);
            u32le(&mut out, e.data.len() as u32);
            u32le(&mut out, e.uncompressed_len);
            u16le(&mut out, e.name.len() as u16);
            u16le(&mut out, e.extra.len() as u16);
            out.extend_from_slice(&e.name);
            out.extend_from_slice(&e.extra);
            out.extend_from_slice(&e.data);
        }
        let cd_offset = out.len() as u32;
        for (e, offset) in self.entries.iter().zip(&offsets) {
            out.extend_from_slice(b"PK\x01\x02");
            u16le(&mut out, 0x031e); // made by: unix
            u16le(&mut out, 20);
            u16le(&mut out, 0);
            u16le(&mut out, e.method);
            u16le(&mut out, 0);
            u16le(&mut out, 0x21);
            u32le(&mut out, e.crc);
            u32le(&mut out, e.data.len() as u32);
            u32le(&mut out, e.uncompressed_len);
            u16le(&mut out, e.name.len() as u16);
            u16le(&mut out, 0); // extra len (central)
            u16le(&mut out, 0); // comment len
            u16le(&mut out, 0); // disk number
            u16le(&mut out, 0); // internal attrs
            u32le(&mut out, e.external_attrs);
            u32le(&mut out, *offset);
            out.extend_from_slice(&e.name);
        }
        let cd_size = out.len() as u32 - cd_offset;
        out.extend_from_slice(b"PK\x05\x06");
        u16le(&mut out, 0);
        u16le(&mut out, 0);
        u16le(&mut out, self.entries.len() as u16);
        u16le(&mut out, self.entries.len() as u16);
        u32le(&mut out, cd_size);
        u32le(&mut out, cd_offset);
        u16le(&mut out, 0);
        out
    }
}
