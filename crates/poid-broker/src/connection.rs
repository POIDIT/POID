//! Connections, as the broker sees them: identity, kind, and what a kind can
//! serve (SPEC §7.2.1).
//!
//! Note what is **not** in this module: a secret. A [`ConnectionRef`] is the
//! whole of what the policy layer needs, and it is safe to hold anywhere. The
//! credential lives in the OS keychain and is fetched by the layer that
//! performs the operation, at the moment it performs it (SPEC §7.2.2).

use std::fmt;

use poid_core::RequireKind;
use serde::{Deserialize, Serialize};

/// What a configured backend can do (SPEC §7.2.1).
///
/// Distinct from [`RequireKind`], which is what an *application* may ask for.
/// The two overlap but are not the same set: an application can declare that
/// it needs somewhere to put data, and cannot declare that it needs a model
/// provider or an MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionKind {
    /// Key-value backend.
    Kv,
    /// Relational backend.
    Sql,
    /// Document backend.
    Docs,
    /// Blob / object storage.
    Files,
    /// Model provider, backing `poid.ai`.
    Ai,
    /// MCP server, backing `poid.mcp`.
    Mcp,
    /// An API credential bound to one or more origins, backing `poid.net`.
    Net,
    /// Vault synchronisation backend.
    Sync,
}

impl ConnectionKind {
    /// The wire/config spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kv => "kv",
            Self::Sql => "sql",
            Self::Docs => "docs",
            Self::Files => "files",
            Self::Ai => "ai",
            Self::Mcp => "mcp",
            Self::Net => "net",
            Self::Sync => "sync",
        }
    }

    /// Whether this backend can serve an application's storage requirement
    /// (SPEC §7.2.1, the satisfies table).
    ///
    /// `Sql` satisfies `Kv` and `Docs` because both tiers are layerable over a
    /// relational backend — which is how the reference implementation already
    /// serves `poid.db.docs` locally. The reverse never holds: a key-value
    /// backend cannot answer `poid.db.sql`.
    #[must_use]
    pub const fn satisfies(self, require: RequireKind) -> bool {
        matches!(
            (self, require),
            (Self::Kv, RequireKind::Kv)
                | (
                    Self::Sql,
                    RequireKind::Kv | RequireKind::Sql | RequireKind::Docs
                )
                | (Self::Docs, RequireKind::Docs)
                | (Self::Files, RequireKind::Files)
        )
    }
}

impl fmt::Display for ConnectionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The reader's identifier for a configured connection.
///
/// Opaque and reader-assigned. An application never sees one and never sends
/// one: a `connectionId` arriving from the sandbox is stripped before the
/// broker looks at the message (CONVENTIONS, security rule 3).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConnectionId(String);

impl ConnectionId {
    /// Wraps a reader-assigned id.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Everything about a connection except the thing that matters to an attacker.
///
/// There is no `secret` field, and there must never be one. Code that needs
/// the credential asks the keychain for it by [`ConnectionId`] at the point of
/// use; code that only needs to decide *whether* an operation is allowed —
/// which is all of this crate — works from this struct alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionRef {
    /// Reader-assigned identifier.
    pub id: ConnectionId,
    /// What this backend can do.
    pub kind: ConnectionKind,
    /// The user's own name for it ("my-supabase", "work-postgres").
    ///
    /// Shown in the reader's binding prompt. Never derived from a credential.
    pub label: String,
    /// Origins this connection covers, for `kind = Net` and for backends
    /// reached over HTTP. Empty for backends addressed another way.
    pub origins: Vec<crate::network::Origin>,
}

impl ConnectionRef {
    /// Whether this connection can serve `require`.
    #[must_use]
    pub const fn satisfies(&self, require: RequireKind) -> bool {
        self.kind.satisfies(require)
    }

    /// Whether this connection's credential should be attached to a request
    /// for `origin`.
    ///
    /// Exact origin match only. A prefix or suffix rule here would mean
    /// `https://api.example.com.attacker.test` receives the user's key.
    #[must_use]
    pub fn covers(&self, origin: &crate::network::Origin) -> bool {
        self.origins.iter().any(|o| o == origin)
    }
}

/// The manifest spelling of a requirement kind, for diagnostics.
#[must_use]
pub const fn require_kind_str(require: RequireKind) -> &'static str {
    match require {
        RequireKind::Kv => "kv",
        RequireKind::Sql => "sql",
        RequireKind::Docs => "docs",
        RequireKind::Files => "files",
    }
}
