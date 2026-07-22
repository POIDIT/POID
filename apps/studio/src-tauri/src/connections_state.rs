//! The process-wide connection registry (SPEC §7.2).
//!
//! One store for the whole application, exactly like the vault: connections
//! belong to the *user*, not to a document, so every window sees the same set.
//!
//! The secrets inside never leave this process. Windows ask for operations;
//! they never ask for credentials, and there is no command that returns one.

use std::path::PathBuf;
use std::sync::Mutex;

use poid_connections::{BindingStore, ConnectionError, ConnectionStore, KeyringStore};

/// The user's configured connections, behind one lock.
pub struct ConnectionsState {
    store: Mutex<ConnectionStore<KeyringStore>>,
    bindings: Mutex<BindingStore>,
}

impl ConnectionsState {
    /// Opens the registry under `root` (the app data dir), with credentials in
    /// the OS keychain.
    ///
    /// # Errors
    ///
    /// Propagates a registry that exists but cannot be read or parsed. A
    /// corrupt registry is surfaced rather than replaced: silently starting
    /// empty would read as "all my connections disappeared" and invite the
    /// user to retype credentials they had already stored.
    pub fn open(root: PathBuf) -> Result<Self, ConnectionError> {
        let store = ConnectionStore::open(root.join("connections.json"), KeyringStore::new())?;
        let bindings = BindingStore::open(root.join("bindings.json"))?;
        Ok(Self {
            store: Mutex::new(store),
            bindings: Mutex::new(bindings),
        })
    }

    /// Runs `f` against the recorded bindings.
    ///
    /// # Errors
    ///
    /// Propagates whatever `f` returns; a poisoned lock surfaces as a registry
    /// read failure rather than a panic.
    pub fn with_bindings<T>(
        &self,
        f: impl FnOnce(&mut BindingStore) -> Result<T, ConnectionError>,
    ) -> Result<T, ConnectionError> {
        let mut bindings = self
            .bindings
            .lock()
            .map_err(|_| ConnectionError::Registry {
                operation: "read",
                path: "<in-memory bindings>".to_owned(),
            })?;
        f(&mut bindings)
    }

    /// Runs `f` against the registry.
    ///
    /// # Errors
    ///
    /// Propagates whatever `f` returns; a poisoned lock surfaces as a registry
    /// read failure rather than a panic.
    pub fn with<T>(
        &self,
        f: impl FnOnce(&mut ConnectionStore<KeyringStore>) -> Result<T, ConnectionError>,
    ) -> Result<T, ConnectionError> {
        let mut store = self.store.lock().map_err(|_| ConnectionError::Registry {
            operation: "read",
            path: "<in-memory registry>".to_owned(),
        })?;
        f(&mut store)
    }
}
