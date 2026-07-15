//! Canonical tree digests over `app/` and `deps/` (SPEC §3.3).
//!
//! For every file under the prefix, sorted by full container path (byte
//! order), compute `SHA-256(path ‖ 0x00 ‖ content)`; the tree digest is the
//! SHA-256 of the concatenation of those per-file digests. Hashing a digest
//! list (rather than one concatenated stream) makes file boundaries
//! unambiguous: no combination of paths and contents can collide with a
//! different combination.
//!
//! `data/`, `slots/` and `manifest.json` are deliberately outside the digest:
//! consent is keyed to the *application* hash (SECURITY §5), so saving user
//! data or assigning `instance.id` must not invalidate consent.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use crate::manifest::{ExtraFields, Integrity, IntegrityAlgo, Manifest};
use crate::signature::to_hex;

/// Recomputes the manifest's integrity block from the actual files. Called on
/// every pack and before signing, so stale digests cannot survive either.
pub(crate) fn refresh(manifest: &mut Manifest, files: &BTreeMap<String, Vec<u8>>) {
    let integrity = manifest.integrity.get_or_insert_with(|| Integrity {
        algo: IntegrityAlgo::Sha256,
        app: None,
        deps: None,
        extra: ExtraFields::new(),
    });
    integrity.algo = IntegrityAlgo::Sha256;
    integrity.app = tree_digest(files, "app/");
    integrity.deps = tree_digest(files, "deps/");
}

/// Computes the canonical digest of all files whose path starts with `prefix`.
///
/// Returns `None` when no files live under the prefix (the manifest then
/// omits the corresponding `integrity` field).
pub(crate) fn tree_digest(files: &BTreeMap<String, Vec<u8>>, prefix: &str) -> Option<String> {
    let mut outer = Sha256::new();
    let mut any = false;
    for (path, content) in files.range(prefix.to_owned()..) {
        if !path.starts_with(prefix) {
            break;
        }
        any = true;
        let mut inner = Sha256::new();
        inner.update(path.as_bytes());
        inner.update([0u8]);
        inner.update(content);
        outer.update(inner.finalize());
    }
    any.then(|| to_hex(&outer.finalize()))
}

#[cfg(test)]
mod tests {
    use super::tree_digest;
    use std::collections::BTreeMap;

    fn files(pairs: &[(&str, &[u8])]) -> BTreeMap<String, Vec<u8>> {
        pairs
            .iter()
            .map(|(p, c)| ((*p).to_owned(), c.to_vec()))
            .collect()
    }

    #[test]
    fn empty_tree_has_no_digest() {
        assert_eq!(tree_digest(&files(&[]), "app/"), None);
        assert_eq!(
            tree_digest(&files(&[("data/store.json", b"{}")]), "app/"),
            None
        );
    }

    #[test]
    fn digest_is_order_independent_and_content_sensitive() {
        let a = files(&[("app/a", b"1"), ("app/b", b"2")]);
        let b = files(&[("app/b", b"2"), ("app/a", b"1")]);
        assert_eq!(tree_digest(&a, "app/"), tree_digest(&b, "app/"));

        let c = files(&[("app/a", b"1"), ("app/b", b"3")]);
        assert_ne!(tree_digest(&a, "app/"), tree_digest(&c, "app/"));
    }

    #[test]
    fn file_boundaries_are_unambiguous() {
        // One file whose content embeds "\0app/b\0..." must not collide with
        // two separate files.
        let one = files(&[("app/a", b"1\0app/b\x002")]);
        let two = files(&[("app/a", b"1"), ("app/b", b"2")]);
        assert_ne!(tree_digest(&one, "app/"), tree_digest(&two, "app/"));
    }

    #[test]
    fn prefix_does_not_leak_into_neighbours() {
        // "app0" sorts after "app/" — the range scan must not include it.
        let f = files(&[("app/a", b"1"), ("app0", b"junk")]);
        let g = files(&[("app/a", b"1")]);
        assert_eq!(tree_digest(&f, "app/"), tree_digest(&g, "app/"));
    }
}
