//! POID Connections: the mechanism behind SPEC §7.2.
//!
//! `poid-broker` decides whether something may happen. This crate is where the
//! credential actually lives and where the registry is kept — the two jobs
//! that involve touching the operating system.
//!
//! # The one rule
//!
//! A credential is stored in the OS credential store and nowhere else. Not in
//! the registry file, not in a log, not in a URL, not in a struct that gets
//! serialised (SPEC §7.2.2). The API is shaped to make that structural rather
//! than careful:
//!
//! - [`ConnectionRecord`] and [`poid_broker::ConnectionRef`] have no secret
//!   field, so nothing persisted or forwarded can carry one.
//! - [`NewConnection`] — the only type that holds a credential — exists solely
//!   as an argument to [`ConnectionStore::upsert`], which splits it into
//!   "write this down" and "hand this to the keychain" and drops the rest.
//! - [`ConnectionStore::secret`] is the single way back to a credential, is
//!   documented as such, and is called only by code that performs a
//!   privileged operation in Core.
//!
//! # Platforms
//!
//! Windows Credential Manager, macOS Keychain, and Secret Service on Linux,
//! via the `keyring` crate. A platform without one of these does not get a
//! weaker fallback — it gets no credentialed connections at all, because a
//! secret kept somewhere the user cannot see is a secret they cannot revoke.

#![forbid(unsafe_code)]

pub mod bindings;
pub mod dsn;
pub mod error;
pub mod record;
pub mod secret;
pub mod store;

pub use bindings::{BindingStore, RecordedBinding};
pub use dsn::{ParsedDsn, SqlTarget};
pub use error::{ConnectionError, Result};
pub use record::{ConnectionConfig, ConnectionRecord};
pub use secret::{KeyringStore, SecretStore, SERVICE};
pub use store::{ConnectionStore, NewConnection};

#[cfg(any(test, feature = "test-store"))]
pub use secret::MemoryStore;

#[cfg(test)]
mod tests {
    use poid_broker::{ConnectionId, ConnectionKind, Origin};

    use super::*;

    const DSN: &str = "postgres://app:sup3rs3cret-pw@db.example.com:5432/appdb";
    const TOKEN: &str = "sk-live-0123456789abcdef";

    fn store(dir: &tempfile::TempDir) -> ConnectionStore<MemoryStore> {
        ConnectionStore::open(dir.path().join("connections.json"), MemoryStore::new())
            .expect("a fresh registry opens")
    }

    fn origin(value: &str) -> Origin {
        Origin::parse(value).expect("a valid origin")
    }

    #[test]
    fn the_registry_file_never_contains_a_credential() {
        // The Definition of Done, at the storage layer: configure a connection
        // with a known secret, then search everything written to disk for it.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("connections.json");
        let mut store = ConnectionStore::open(&path, MemoryStore::new()).expect("opens");

        store
            .upsert(
                ConnectionId::new("c1"),
                "my-supabase",
                NewConnection::Sql {
                    dsn: DSN.to_owned(),
                },
            )
            .expect("configures");
        store
            .upsert(
                ConnectionId::new("c2"),
                "my-api",
                NewConnection::Net {
                    origins: vec![origin("https://api.example.com")],
                    token: TOKEN.to_owned(),
                },
            )
            .expect("configures");

        let written = std::fs::read_to_string(&path).expect("the registry was written");
        assert!(
            !written.contains("sup3rs3cret-pw"),
            "the DSN password reached the file"
        );
        assert!(!written.contains(DSN), "the whole DSN reached the file");
        assert!(!written.contains(TOKEN), "the API token reached the file");

        // The parts that identify the connection are there, because the user
        // has to be able to tell one from another.
        assert!(written.contains("my-supabase"));
        assert!(written.contains("db.example.com"));
        assert!(written.contains("appdb"));
    }

    #[test]
    fn the_brokers_view_carries_no_credential() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut store = store(&dir);
        store
            .upsert(
                ConnectionId::new("c1"),
                "my-supabase",
                NewConnection::Sql {
                    dsn: DSN.to_owned(),
                },
            )
            .expect("configures");

        // Everything the policy layer and the binding prompt ever see.
        let refs = store.refs();
        let rendered = format!("{refs:?}");
        assert!(!rendered.contains("sup3rs3cret-pw"));
        assert_eq!(refs[0].kind, ConnectionKind::Sql);
        assert_eq!(refs[0].label, "my-supabase");
    }

    #[test]
    fn the_secret_survives_a_reopen_and_the_metadata_round_trips() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("connections.json");
        let secrets = MemoryStore::new();
        {
            let mut store = ConnectionStore::open(&path, MemoryStore::new()).expect("opens");
            store
                .upsert(
                    ConnectionId::new("c1"),
                    "my-supabase",
                    NewConnection::Sql {
                        dsn: DSN.to_owned(),
                    },
                )
                .expect("configures");
            // Mirror the write into the store we will reopen with, standing in
            // for a keychain that outlives the process.
            secrets.set("c1", DSN).expect("stores");
        }

        let store = ConnectionStore::open(&path, secrets).expect("reopens");
        let record = store.get(&ConnectionId::new("c1")).expect("still there");
        assert_eq!(record.label, "my-supabase");
        assert_eq!(record.kind(), ConnectionKind::Sql);
        assert_eq!(
            store.secret(&ConnectionId::new("c1")).expect("readable"),
            DSN
        );
    }

    #[test]
    fn removing_a_connection_removes_its_credential() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut store = store(&dir);
        let id = ConnectionId::new("c1");
        store
            .upsert(
                id.clone(),
                "my-api",
                NewConnection::Net {
                    origins: vec![origin("https://api.example.com")],
                    token: TOKEN.to_owned(),
                },
            )
            .expect("configures");
        assert!(store.has_secret(&id).expect("checks"));

        store.remove(&id).expect("removes");
        assert!(store.get(&id).is_none());
        // The point of the test: clicking delete must not leave the key behind
        // in the keychain, where the user would never think to look for it.
        assert!(!store.has_secret(&id).expect("checks"));
    }

    #[test]
    fn a_missing_secret_reads_as_missing_rather_than_as_a_failure() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = store(&dir);
        let id = ConnectionId::new("never-configured");
        assert!(matches!(
            store.secret(&id),
            Err(ConnectionError::NotFound { .. })
        ));
        assert!(!store.has_secret(&id).expect("checks"));
    }

    #[test]
    fn unusable_input_is_refused_before_anything_is_stored() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut store = store(&dir);
        let id = ConnectionId::new("c1");

        // A DSN that cannot connect.
        assert!(store
            .upsert(
                id.clone(),
                "broken",
                NewConnection::Sql {
                    dsn: "mysql://app:p@host/db".to_owned()
                },
            )
            .is_err());

        // A net credential covering nothing would never be used.
        assert!(store
            .upsert(
                id.clone(),
                "pointless",
                NewConnection::Net {
                    origins: Vec::new(),
                    token: TOKEN.to_owned()
                },
            )
            .is_err());

        // An empty credential.
        assert!(store
            .upsert(
                id.clone(),
                "empty",
                NewConnection::Net {
                    origins: vec![origin("https://api.example.com")],
                    token: "   ".to_owned()
                },
            )
            .is_err());

        // Nothing was recorded and nothing was stored.
        assert!(store.get(&id).is_none());
        assert!(!store.has_secret(&id).expect("checks"));
    }

    #[test]
    fn the_redactor_is_primed_with_every_stored_credential() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut store = store(&dir);
        store
            .upsert(
                ConnectionId::new("c1"),
                "my-supabase",
                NewConnection::Sql {
                    dsn: DSN.to_owned(),
                },
            )
            .expect("configures");

        let redactor = store.redactor();
        // The shape of a real driver error: the backend quotes what it was
        // given, password and all.
        let raw = format!("FATAL: password authentication failed for \"{DSN}\"");
        assert!(redactor.leaks(&raw));

        let cleaned = redactor.redact(&raw);
        assert!(!redactor.leaks(&cleaned));
        assert!(!cleaned.contains("sup3rs3cret-pw"));
        // Still useful to the person reading their own log.
        assert!(cleaned.contains("password authentication failed"));
    }

    #[test]
    fn upsert_replaces_rather_than_duplicating() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut store = store(&dir);
        let id = ConnectionId::new("c1");
        for label in ["first", "second"] {
            store
                .upsert(
                    id.clone(),
                    label,
                    NewConnection::Sql {
                        dsn: DSN.to_owned(),
                    },
                )
                .expect("configures");
        }
        assert_eq!(store.records().len(), 1);
        assert_eq!(store.get(&id).expect("present").label, "second");
    }
}
