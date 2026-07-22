//! Errors from the connection registry and the secret store.
//!
//! Every variant here is for the **reader's** eyes — the user's log, Studio's
//! own diagnostics. None of it crosses to an application: the boundary
//! converts to a `poid_broker::SafeError` first (SPEC §7.2.4).
//!
//! Note what the variants carry: a connection id, a path, a store name. Never
//! a secret, and never the text a keychain returned, because on some platforms
//! that text quotes the item it failed on.

use thiserror::Error;

/// A failure in the connection registry or the secret store.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConnectionError {
    /// The OS credential store refused or failed.
    ///
    /// Deliberately coarse. The underlying error is summarised to a
    /// `&'static str` rather than forwarded, so a platform backend cannot
    /// smuggle the credential it was handling into our diagnostics.
    #[error("the system credential store could not {operation} the secret for `{connection}`")]
    SecretStore {
        /// What was being attempted: `store`, `read`, or `remove`.
        operation: &'static str,
        /// The connection whose secret was involved.
        connection: String,
    },
    /// No secret is stored for this connection.
    ///
    /// Distinct from a store failure: this is the ordinary "you configured a
    /// connection and then the user deleted the keychain item" case, and the
    /// honest response is to ask for the credential again.
    #[error("no secret is stored for connection `{connection}`")]
    SecretMissing {
        /// The connection with no secret.
        connection: String,
    },
    /// No connection with this id is configured.
    #[error("connection `{connection}` is not configured")]
    NotFound {
        /// The id that was looked up.
        connection: String,
    },
    /// A connection with this id already exists.
    #[error("connection `{connection}` already exists")]
    Duplicate {
        /// The colliding id.
        connection: String,
    },
    /// The registry file could not be read or written.
    #[error("the connection registry at `{path}` could not be {operation}")]
    Registry {
        /// What was being attempted: `read`, `written`, or `parsed`.
        operation: &'static str,
        /// The registry path.
        path: String,
    },
    /// The credential offered is not usable.
    #[error("that credential cannot be used: {why}")]
    InvalidSecret {
        /// Why it was refused.
        why: &'static str,
    },
    /// The connection's configuration is not valid for its kind.
    #[error("that connection is not configured correctly: {why}")]
    InvalidConfig {
        /// Why it was refused.
        why: &'static str,
    },
    /// A brokered network request was refused or failed (SPEC §7.2.5).
    ///
    /// The reason is free text because the **user's** log should say what
    /// actually happened — which origin, which address, which transport
    /// error. It is never forwarded to an application: the boundary maps it to
    /// a `RUNTIME-API.md` §9 code first, and the text goes to the diagnostics
    /// the user reads.
    #[error("that request could not be made: {reason}")]
    Network {
        /// What went wrong, for the user's log.
        reason: String,
    },
    /// A SQL connection could not be opened, or refused a statement.
    ///
    /// Like [`ConnectionError::Network`], the reason is free text for the
    /// user's log and never for an application. Postgres error messages quote
    /// the statement and sometimes the connection string, which is exactly why
    /// this must be scrubbed before it is written and mapped to a §9 code
    /// before it crosses the boundary (SPEC §7.2.4).
    #[error("the database could not do that: {reason}")]
    Sql {
        /// What went wrong, for the user's log.
        reason: String,
    },
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, ConnectionError>;
