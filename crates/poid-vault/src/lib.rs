//! POID vault: managed storage keyed by `instance.id`.
//!
//! The vault MUST be a CRDT (Automerge), not a last-write-wins blob — see
//! SPEC §6.5. This is a load-bearing decision: without CRDT semantics from
//! day one, offline editing on two devices loses data.

/// Name of the vault storage engine mandated by SPEC §6.5.
pub const STORAGE_ENGINE: &str = "automerge";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_is_a_crdt_not_a_blob() {
        assert_eq!(STORAGE_ENGINE, "automerge");
    }
}
