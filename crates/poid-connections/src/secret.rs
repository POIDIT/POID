//! Where credentials actually live (SPEC §7.2.2).
//!
//! One trait, two implementations, and a hard rule about the second:
//!
//! - [`KeyringStore`] is the real one — Windows Credential Manager, macOS
//!   Keychain, Secret Service on Linux.
//! - [`MemoryStore`] exists for tests and is behind the `test-store` feature,
//!   which no release build enables. It is not a fallback. A reader that
//!   cannot reach an OS credential store must refuse to offer credentialed
//!   connections rather than quietly holding secrets somewhere weaker — SPEC
//!   §7.2.2 says so, and the reason is that the weaker place is invisible to
//!   the user, who would go on believing their key was in the keychain.

use crate::error::{ConnectionError, Result};

/// The service name POID registers credentials under.
///
/// One service, one account per connection id. Reverse-DNS so the user can
/// recognise the entries in Keychain Access or `credwiz` and delete them by
/// hand — a credential the user cannot find is a credential they cannot
/// revoke.
pub const SERVICE: &str = "dev.poid.studio";

/// Somewhere a credential can be kept.
///
/// Implementations hold real secrets, so they exist only in the process that
/// performs privileged operations. Nothing here is reachable from a web
/// context, let alone a sandbox.
pub trait SecretStore: Send + Sync {
    /// Stores (or replaces) the secret for a connection.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::SecretStore`] if the platform store refuses.
    fn set(&self, connection: &str, secret: &str) -> Result<()>;

    /// Reads the secret for a connection.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::SecretMissing`] when nothing is stored, or
    /// [`ConnectionError::SecretStore`] if the platform store refuses.
    fn get(&self, connection: &str) -> Result<String>;

    /// Removes the secret for a connection.
    ///
    /// Removing something that is not there succeeds: deletion is idempotent,
    /// so a half-finished delete can always be finished.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::SecretStore`] if the platform store refuses.
    fn delete(&self, connection: &str) -> Result<()>;

    /// Whether a secret is stored, without reading it.
    ///
    /// The manager UI needs to show "configured" versus "not set up yet", and
    /// it must be able to do that without the secret travelling anywhere.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::SecretStore`] if the platform store refuses.
    fn has(&self, connection: &str) -> Result<bool> {
        match self.get(connection) {
            Ok(_) => Ok(true),
            Err(ConnectionError::SecretMissing { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

/// The operating system's credential store.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

impl KeyringStore {
    /// Builds a store backed by the OS keychain.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    fn entry(connection: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, connection).map_err(|_| ConnectionError::SecretStore {
            operation: "address",
            connection: connection.to_owned(),
        })
    }
}

impl SecretStore for KeyringStore {
    fn set(&self, connection: &str, secret: &str) -> Result<()> {
        if secret.is_empty() {
            return Err(ConnectionError::InvalidSecret { why: "it is empty" });
        }
        Self::entry(connection)?
            .set_password(secret)
            .map_err(|_| ConnectionError::SecretStore {
                operation: "store",
                connection: connection.to_owned(),
            })
    }

    fn get(&self, connection: &str) -> Result<String> {
        // The platform error is mapped, never forwarded: some backends
        // include the item's own contents in their message.
        Self::entry(connection)?
            .get_password()
            .map_err(|e| match e {
                keyring::Error::NoEntry => ConnectionError::SecretMissing {
                    connection: connection.to_owned(),
                },
                _ => ConnectionError::SecretStore {
                    operation: "read",
                    connection: connection.to_owned(),
                },
            })
    }

    fn delete(&self, connection: &str) -> Result<()> {
        match Self::entry(connection)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(ConnectionError::SecretStore {
                operation: "remove",
                connection: connection.to_owned(),
            }),
        }
    }
}

/// An in-memory secret store, **for tests only**.
///
/// Gated behind the `test-store` feature, which `default` does not include, so
/// this type does not exist in a release build. See the module docs for why
/// this is a test double rather than a fallback.
#[cfg(any(test, feature = "test-store"))]
#[derive(Debug, Default)]
pub struct MemoryStore {
    secrets: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
}

#[cfg(any(test, feature = "test-store"))]
impl MemoryStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every secret currently held, for assertions.
    ///
    /// A test wanting to prove a credential did **not** reach some file or
    /// message needs to know what the credential was.
    #[must_use]
    pub fn values(&self) -> Vec<String> {
        self.secrets
            .lock()
            .map(|s| s.values().cloned().collect())
            .unwrap_or_default()
    }
}

#[cfg(any(test, feature = "test-store"))]
impl SecretStore for MemoryStore {
    fn set(&self, connection: &str, secret: &str) -> Result<()> {
        if secret.is_empty() {
            return Err(ConnectionError::InvalidSecret { why: "it is empty" });
        }
        let mut secrets = self
            .secrets
            .lock()
            .map_err(|_| ConnectionError::SecretStore {
                operation: "store",
                connection: connection.to_owned(),
            })?;
        secrets.insert(connection.to_owned(), secret.to_owned());
        Ok(())
    }

    fn get(&self, connection: &str) -> Result<String> {
        let secrets = self
            .secrets
            .lock()
            .map_err(|_| ConnectionError::SecretStore {
                operation: "read",
                connection: connection.to_owned(),
            })?;
        secrets
            .get(connection)
            .cloned()
            .ok_or_else(|| ConnectionError::SecretMissing {
                connection: connection.to_owned(),
            })
    }

    fn delete(&self, connection: &str) -> Result<()> {
        let mut secrets = self
            .secrets
            .lock()
            .map_err(|_| ConnectionError::SecretStore {
                operation: "remove",
                connection: connection.to_owned(),
            })?;
        secrets.remove(connection);
        Ok(())
    }
}
