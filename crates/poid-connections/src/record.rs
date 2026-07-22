//! What the registry file holds — which is everything about a connection
//! except the credential.
//!
//! [`ConnectionRecord`] has no secret field, exactly as
//! `poid_broker::ConnectionRef` has none. That is not a coincidence and not
//! belt-and-braces: it is the same invariant expressed at the two layers that
//! could otherwise persist or forward a credential by accident. If a future
//! change needs a secret in one of these structs, that change is wrong.

use poid_broker::{ConnectionId, ConnectionKind, ConnectionRef, Origin};
use serde::{Deserialize, Serialize};

use crate::dsn::SqlTarget;

/// The non-secret configuration of a connection, by kind.
///
/// Only the kinds this milestone implements are here. `ai`, `mcp`, `kv`,
/// `docs`, `files` and `sync` are vocabulary in SPEC §7.2.1 and arrive with
/// the milestones that implement them — an empty variant now would be a shape
/// invented before its use, and shapes invented that way are usually wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ConnectionConfig {
    /// A relational backend. The full DSN is the secret; this is what is safe
    /// to show.
    Sql {
        /// Host, port, database and role — never the password.
        target: SqlTarget,
    },
    /// An API credential bound to origins, backing `poid.net`.
    Net {
        /// The origins whose requests receive this credential.
        ///
        /// Matched exactly (`ConnectionRef::covers`), so an attacker cannot
        /// register `api.example.com.evil.test` and collect the key.
        origins: Vec<Origin>,
    },
}

impl ConnectionConfig {
    /// The kind this configuration belongs to.
    #[must_use]
    pub const fn kind(&self) -> ConnectionKind {
        match self {
            Self::Sql { .. } => ConnectionKind::Sql,
            Self::Net { .. } => ConnectionKind::Net,
        }
    }

    /// A one-line description for the connection manager.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::Sql { target } => target.display(),
            Self::Net { origins } => origins
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

/// One configured connection, as persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionRecord {
    /// Reader-assigned identifier; the keychain account name.
    pub id: ConnectionId,
    /// The user's own name for it.
    pub label: String,
    /// Kind-specific, non-secret configuration.
    pub config: ConnectionConfig,
}

impl ConnectionRecord {
    /// The kind of backend this is.
    #[must_use]
    pub const fn kind(&self) -> ConnectionKind {
        self.config.kind()
    }

    /// The broker's view of this connection.
    ///
    /// This is what may be handed to the policy layer, and — with the label
    /// alone — to the reader's binding prompt. It still holds no secret.
    #[must_use]
    pub fn to_ref(&self) -> ConnectionRef {
        ConnectionRef {
            id: self.id.clone(),
            kind: self.kind(),
            label: self.label.clone(),
            origins: match &self.config {
                ConnectionConfig::Net { origins } => origins.clone(),
                ConnectionConfig::Sql { .. } => Vec::new(),
            },
        }
    }
}

/// The registry file's on-disk shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RegistryFile {
    /// Schema version, so a later format change can migrate rather than guess.
    pub(crate) version: u32,
    /// The configured connections.
    pub(crate) connections: Vec<ConnectionRecord>,
}

/// The registry schema version this crate writes.
pub(crate) const REGISTRY_VERSION: u32 = 1;
