//! The POID conformance fixtures (SPEC §14, `spec/CONFORMANCE.md`).
//!
//! Every fixture is *generated*, never hand-crafted: this crate is the single
//! source of truth, and `gen-conformance` writes the suite to
//! `spec/conformance/`. Generation is fully deterministic — fixed timestamps
//! (the writer's), a fixed pseudo-random seed, and a fixed, well-known test
//! signing key — so CI can regenerate the suite and diff it against the
//! committed files.

mod invalid;
mod rawzip;
mod valid;

pub use rawzip::RawZip;

use poid_core::PoidError;

/// A well-known Ed25519 test seed used by the `signed` fixtures.
///
/// It is public by design — **never** use it to sign anything real. Ed25519
/// signatures are deterministic (RFC 8032), which keeps the signed fixtures
/// byte-reproducible.
pub const TEST_KEY_SEED: [u8; 32] = [42u8; 32];

/// One conformance fixture: container bytes plus the expected verdict.
pub struct Fixture {
    /// File stem, e.g. `minimal-html`.
    pub name: &'static str,
    /// Whether a conformant implementation must accept it.
    pub valid: bool,
    /// The container bytes.
    pub bytes: Vec<u8>,
    /// Registry code (`spec/errors.md`) expected on rejection.
    pub expected_code: Option<&'static str>,
}

impl Fixture {
    /// Content of the sibling `<name>.expected.json` file.
    pub fn expected_json(&self) -> serde_json::Value {
        match self.expected_code {
            Some(code) => serde_json::json!({ "valid": false, "code": code }),
            None => serde_json::json!({ "valid": true }),
        }
    }
}

/// Builds the complete suite, valid fixtures first.
pub fn conformance_fixtures() -> Result<Vec<Fixture>, PoidError> {
    let mut fixtures = valid::all()?;
    fixtures.extend(invalid::all()?);
    Ok(fixtures)
}

/// Deterministic pseudo-random bytes (xorshift64) — incompressible content
/// for the `large-but-legal` and `protected` fixtures.
pub fn pseudo_random_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed | 1;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.truncate(len);
    out
}

/// A small ZIP built with the real writer, deterministically pinned options —
/// used for Python wheels in `deps/` and for the zip-bomb fixture.
pub fn mini_zip(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, PoidError> {
    use std::io::Write;
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6))
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, content) in entries {
        w.start_file(*name, options).map_err(zip_error)?;
        w.write_all(content)?;
    }
    Ok(w.finish().map_err(zip_error)?.into_inner())
}

fn zip_error(e: zip::result::ZipError) -> PoidError {
    match e {
        zip::result::ZipError::Io(io) => PoidError::Io(io),
        other => PoidError::Io(std::io::Error::other(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use poid_core::{conformance_check, Limits};

    /// The DoD gate: the reference implementation passes 100% of the suite,
    /// with exactly the registry codes the fixtures promise.
    #[test]
    fn reference_implementation_passes_the_whole_suite() {
        let fixtures = super::conformance_fixtures().expect("fixtures build");
        assert!(fixtures.len() >= 26, "suite unexpectedly small");
        for fixture in fixtures {
            match (
                fixture.valid,
                conformance_check(&fixture.bytes, &Limits::default()),
            ) {
                (true, Ok(_)) => {}
                (true, Err(e)) => panic!("{} must open cleanly, got: {e}", fixture.name),
                (false, Ok(_)) => panic!("{} must be rejected", fixture.name),
                (false, Err(e)) => assert_eq!(
                    e.conformance_code(),
                    fixture.expected_code,
                    "{}: wrong registry code (diagnostic: {})",
                    fixture.name,
                    e.code()
                ),
            }
        }
    }

    /// Generation is deterministic: two runs produce identical bytes.
    #[test]
    fn fixtures_are_deterministic() {
        let a = super::conformance_fixtures().expect("first run");
        let b = super::conformance_fixtures().expect("second run");
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.name, y.name);
            assert_eq!(x.bytes, y.bytes, "{} not reproducible", x.name);
        }
    }
}
