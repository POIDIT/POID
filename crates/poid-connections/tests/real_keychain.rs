//! The OS credential store, for real (SPEC §7.2.2).
//!
//! Every other test in this crate uses `MemoryStore`, which proves the logic
//! and nothing about the platform. This one talks to the actual Windows
//! Credential Manager / macOS Keychain / Secret Service, because "the secret
//! is in the keychain" is the entire security claim and a test double cannot
//! evidence it.
//!
//! `#[ignore]` by default: it writes to the developer's own credential store,
//! and CI runners frequently have no unlocked keyring at all — a failure there
//! would mean "no D-Bus session", not "POID is broken". Run it deliberately:
//!
//! ```text
//! cargo test -p poid-connections --test real_keychain -- --ignored --nocapture
//! ```
#![allow(clippy::unwrap_used, clippy::expect_used)]

use poid_connections::{KeyringStore, SecretStore, SERVICE};

/// An account name no real connection can collide with.
const TEST_ID: &str = "poid-selftest-do-not-use";

#[test]
#[ignore = "writes to the developer's real OS credential store"]
fn a_credential_round_trips_through_the_real_keychain() {
    let store = KeyringStore::new();
    let secret = "postgres://selftest:sup3rs3cret@db.invalid:5432/selftest";

    // Leave nothing behind from an interrupted earlier run.
    store.delete(TEST_ID).expect("delete is idempotent");
    assert!(
        !store.has(TEST_ID).expect("checks"),
        "stale entry from a previous run"
    );

    store
        .set(TEST_ID, secret)
        .expect("the OS accepted the secret");
    assert!(store.has(TEST_ID).expect("checks"));
    assert_eq!(store.get(TEST_ID).expect("reads back"), secret);

    println!("service `{SERVICE}`, account `{TEST_ID}` — written, read back, removing");

    store.delete(TEST_ID).expect("removes");
    assert!(
        !store.has(TEST_ID).expect("checks"),
        "the entry outlived the test"
    );
}
