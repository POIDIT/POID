//! The process-wide connection registry (SPEC §7.2).
//!
//! One store for the whole application, exactly like the vault: connections
//! belong to the *user*, not to a document, so every window sees the same set.
//!
//! The secrets inside never leave this process. Windows ask for operations;
//! they never ask for credentials, and there is no command that returns one.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use poid_broker::ConnectionId;
use poid_connections::{
    BindingStore, ConnectionError, ConnectionStore, KeyringStore, PostgresConnection,
};

/// The user's configured connections, behind one lock.
pub struct ConnectionsState {
    store: Mutex<ConnectionStore<KeyringStore>>,
    bindings: Mutex<BindingStore>,
    /// Open database connections, one per Reader window.
    ///
    /// Keyed by window label, so a window's statements always run on its own
    /// session — which is what makes a transaction mean anything. Opening a
    /// socket per statement would be a TLS handshake per query; sharing one
    /// across windows would let one document see another's open transaction.
    sql: tokio::sync::Mutex<HashMap<String, Arc<PostgresConnection>>>,
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
            sql: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    /// The window's database connection, opening it on first use.
    ///
    /// The credential is read from the keychain here and dropped as soon as
    /// the socket is up: it lives for the length of a connect, in Core, and
    /// never travels (SPEC §7.1).
    ///
    /// # Errors
    ///
    /// Propagates a missing connection, a missing credential, or a server that
    /// refuses.
    pub async fn sql_for_window(
        &self,
        label: &str,
        connection: &ConnectionId,
    ) -> Result<Arc<PostgresConnection>, ConnectionError> {
        let mut open = self.sql.lock().await;
        // A dropped connection (server restart, network blip) is reopened
        // rather than handed back dead — the application would otherwise see
        // an unexplained failure it cannot act on.
        if let Some(existing) = open.get(label) {
            if !existing.is_closed() {
                return Ok(Arc::clone(existing));
            }
            open.remove(label);
        }

        let secret = self.with(|store| store.secret(connection))?;
        let client = Arc::new(PostgresConnection::open(&secret).await?);
        drop(secret);
        open.insert(label.to_owned(), Arc::clone(&client));
        Ok(client)
    }

    /// Closes and forgets a window's database connection (window teardown).
    pub async fn close_sql_for_window(&self, label: &str) {
        self.sql.lock().await.remove(label);
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
