//! The connection registry: metadata in a file, credentials in the keychain,
//! and one API that keeps them from meeting.
//!
//! The split is the whole design. [`ConnectionStore::upsert`] takes the secret
//! as a separate argument, derives the displayable parts from it, writes those
//! to the file and hands the secret straight to the [`SecretStore`]. The
//! secret is never a field of anything persisted, so "the registry leaked a
//! credential" is not a bug that can be introduced by editing a struct.

use std::path::{Path, PathBuf};

use poid_broker::{ConnectionId, ConnectionKind, ConnectionRef, Origin, Redactor};

use crate::dsn;
use crate::error::{ConnectionError, Result};
use crate::record::{ConnectionConfig, ConnectionRecord, RegistryFile, REGISTRY_VERSION};
use crate::secret::SecretStore;

/// What the user typed when configuring a connection.
///
/// Deliberately not `ConnectionConfig`: this is the *input*, and it carries
/// the credential. It exists only as a function argument and is never stored,
/// serialised, or returned.
#[derive(Debug, Clone)]
pub enum NewConnection {
    /// A relational backend, configured by connection string.
    Sql {
        /// The full DSN, e.g. `postgres://user:pass@host:5432/db`.
        dsn: String,
    },
    /// An API credential bound to origins.
    Net {
        /// The origins whose requests receive this credential.
        origins: Vec<Origin>,
        /// The bearer token or API key.
        token: String,
    },
}

impl NewConnection {
    /// The kind this input configures.
    #[must_use]
    pub const fn kind(&self) -> ConnectionKind {
        match self {
            Self::Sql { .. } => ConnectionKind::Sql,
            Self::Net { .. } => ConnectionKind::Net,
        }
    }

    /// Splits the input into what may be written down and what may not.
    fn split(self) -> Result<(ConnectionConfig, String)> {
        match self {
            Self::Sql { dsn } => {
                // Parsed for validation and for the display form; the *whole*
                // DSN remains the secret, because host and database are not
                // worth a second storage path and a partial secret is a
                // secret somebody eventually reassembles.
                let parsed = dsn::parse(&dsn)?;
                Ok((
                    ConnectionConfig::Sql {
                        target: parsed.target,
                    },
                    dsn,
                ))
            }
            Self::Net { origins, token } => {
                if origins.is_empty() {
                    return Err(ConnectionError::InvalidConfig {
                        why: "it covers no origins, so nothing would ever use it",
                    });
                }
                if token.trim().is_empty() {
                    return Err(ConnectionError::InvalidSecret { why: "it is empty" });
                }
                Ok((ConnectionConfig::Net { origins }, token))
            }
        }
    }
}

/// The user's configured connections.
///
/// Generic over the secret store so tests can run without a keychain, and so
/// the type system records that this struct never holds a credential itself —
/// it only knows where to ask for one.
#[derive(Debug)]
pub struct ConnectionStore<S: SecretStore> {
    path: PathBuf,
    secrets: S,
    records: Vec<ConnectionRecord>,
}

impl<S: SecretStore> ConnectionStore<S> {
    /// Opens the registry at `path`, creating an empty one if it is absent.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::Registry`] if the file exists but cannot be read or
    /// parsed. A corrupt registry is not silently replaced with an empty one:
    /// that would present as "all your connections vanished" and invite the
    /// user to retype credentials they had already stored.
    pub fn open(path: impl Into<PathBuf>, secrets: S) -> Result<Self> {
        let path = path.into();
        let records = if path.exists() {
            let text = std::fs::read_to_string(&path).map_err(|_| ConnectionError::Registry {
                operation: "read",
                path: path.display().to_string(),
            })?;
            let file: RegistryFile =
                serde_json::from_str(&text).map_err(|_| ConnectionError::Registry {
                    operation: "parsed",
                    path: path.display().to_string(),
                })?;
            file.connections
        } else {
            Vec::new()
        };
        Ok(Self {
            path,
            secrets,
            records,
        })
    }

    /// Every configured connection.
    #[must_use]
    pub fn records(&self) -> &[ConnectionRecord] {
        &self.records
    }

    /// The broker's view of every connection: no secrets, safe to pass to the
    /// policy layer and to name in a binding prompt.
    #[must_use]
    pub fn refs(&self) -> Vec<ConnectionRef> {
        self.records.iter().map(ConnectionRecord::to_ref).collect()
    }

    /// One connection by id.
    #[must_use]
    pub fn get(&self, id: &ConnectionId) -> Option<&ConnectionRecord> {
        self.records.iter().find(|r| &r.id == id)
    }

    /// Whether a credential is stored for this connection, without reading it.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::SecretStore`] if the platform store refuses.
    pub fn has_secret(&self, id: &ConnectionId) -> Result<bool> {
        self.secrets.has(id.as_str())
    }

    /// Adds a connection, or replaces an existing one with the same id.
    ///
    /// The credential goes to the keychain; the file gets only what
    /// [`NewConnection::split`] judged safe to write.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::InvalidConfig`] / [`ConnectionError::InvalidSecret`]
    /// for unusable input, [`ConnectionError::SecretStore`] if the keychain
    /// refuses, [`ConnectionError::Registry`] if the file cannot be written.
    pub fn upsert(
        &mut self,
        id: ConnectionId,
        label: impl Into<String>,
        input: NewConnection,
    ) -> Result<()> {
        let (config, secret) = input.split()?;

        // Keychain first. If the file write fails afterwards we have an
        // orphaned secret, which is inert; the reverse order would leave a
        // connection the user can see and select but that cannot authenticate.
        self.secrets.set(id.as_str(), &secret)?;

        let record = ConnectionRecord {
            id: id.clone(),
            label: label.into(),
            config,
        };
        match self.records.iter_mut().find(|r| r.id == id) {
            Some(existing) => *existing = record,
            None => self.records.push(record),
        }
        self.persist()
    }

    /// Renames a connection. The credential is untouched.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::NotFound`] if no such connection exists.
    pub fn rename(&mut self, id: &ConnectionId, label: impl Into<String>) -> Result<()> {
        let record = self
            .records
            .iter_mut()
            .find(|r| &r.id == id)
            .ok_or_else(|| ConnectionError::NotFound {
                connection: id.to_string(),
            })?;
        record.label = label.into();
        self.persist()
    }

    /// Removes a connection and its credential.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::NotFound`] if no such connection exists.
    pub fn remove(&mut self, id: &ConnectionId) -> Result<()> {
        let before = self.records.len();
        self.records.retain(|r| &r.id != id);
        if self.records.len() == before {
            return Err(ConnectionError::NotFound {
                connection: id.to_string(),
            });
        }
        // Secret first, and idempotently: a delete that removes the record but
        // leaves the credential in the keychain is exactly the outcome a user
        // clicking "delete" is trying to avoid.
        self.secrets.delete(id.as_str())?;
        self.persist()
    }

    /// Reads a connection's credential.
    ///
    /// **Callers: this returns a real secret.** It exists for the code that
    /// opens a database connection or signs a request, in the process that
    /// performs the operation. Nothing that returns a value to a web context
    /// may call it (SPEC §7.1).
    ///
    /// # Errors
    ///
    /// [`ConnectionError::NotFound`], [`ConnectionError::SecretMissing`], or
    /// [`ConnectionError::SecretStore`].
    pub fn secret(&self, id: &ConnectionId) -> Result<String> {
        if self.get(id).is_none() {
            return Err(ConnectionError::NotFound {
                connection: id.to_string(),
            });
        }
        self.secrets.get(id.as_str())
    }

    /// A redactor primed with every stored credential.
    ///
    /// For scrubbing the diagnostics the user reads. Building it reads every
    /// secret, so it belongs in Core and nowhere else — and it is built on
    /// demand rather than cached, so a deleted credential stops being held
    /// the moment it is deleted.
    ///
    /// Connections whose secret is missing are skipped rather than failing the
    /// whole operation: a half-configured connection must not stop the others'
    /// diagnostics from being scrubbed.
    #[must_use]
    pub fn redactor(&self) -> Redactor {
        let mut redactor = Redactor::new();
        for record in &self.records {
            if let Ok(secret) = self.secrets.get(record.id.as_str()) {
                redactor.insert(secret);
            }
        }
        redactor
    }

    fn persist(&self) -> Result<()> {
        let file = RegistryFile {
            version: REGISTRY_VERSION,
            connections: self.records.clone(),
        };
        let text = serde_json::to_string_pretty(&file).map_err(|_| ConnectionError::Registry {
            operation: "written",
            path: self.path.display().to_string(),
        })?;
        write_atomically(&self.path, text.as_bytes()).map_err(|_| ConnectionError::Registry {
            operation: "written",
            path: self.path.display().to_string(),
        })
    }
}

/// Writes via a temporary file and a rename, so an interrupted write cannot
/// leave a truncated registry — which `open` would refuse to parse, locking
/// the user out of every connection at once.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}
