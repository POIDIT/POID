//! Binding a declared need to a configured backend (SPEC §7.2.3).
//!
//! The application says *"I need somewhere relational to put my data"*. The
//! user says *which* somewhere. This module holds the rules that connect the
//! two, and one rule matters more than the rest:
//!
//! > The application is not an input here.
//!
//! Every function takes the requirement from the **manifest** and the decision
//! from the **reader's records**. There is no parameter a message from the
//! sandbox could fill in, which is what makes "an application cannot choose
//! its own backend" a property of the type signature rather than a promise.

use poid_core::{RequireKind, StorageRequires};

use crate::connection::{require_kind_str, ConnectionId, ConnectionRef};
use crate::error::PolicyError;

/// What the manifest asked for (SPEC §3.1 `storage.requires`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingRequest {
    /// The kind of backend the application needs.
    pub require: RequireKind,
    /// The manifest's suggested provider.
    ///
    /// **Informative only.** It may order the choices the user is shown; it
    /// must never select one. A hint that could select would be a manifest
    /// field that picks the user's database.
    pub hint: Option<String>,
}

impl BindingRequest {
    /// Reads the request out of a validated manifest's `storage.requires`.
    #[must_use]
    pub fn from_manifest(requires: &StorageRequires) -> Self {
        Self {
            require: requires.kind,
            hint: requires.hint.clone(),
        }
    }
}

/// The user's recorded decision for one application instance.
///
/// Recorded per `app.id` + `instance.id` (SPEC §7.2.3) by the reader, and
/// revocable. Absence of a record means the user has not been asked yet —
/// which is different from having declined, and the reader must not conflate
/// them: the first opens a prompt, the second must not re-open it on every
/// launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// Use this configured connection.
    Connection(ConnectionId),
    /// Keep the data local, in this reader's own store.
    ///
    /// Always available, always offered. A reader that only lets the user
    /// connect-or-quit fails SPEC §7.2.3 clause 2.
    KeepLocal,
}

/// What the reader should do once a binding is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved<'a> {
    /// Serve the storage tiers from this connection.
    Connection(&'a ConnectionRef),
    /// Serve them from the local store, exactly as `embedded`/`vault` would.
    Local,
}

/// The connections that could serve this request, in the order to offer them.
///
/// Ordering is a courtesy, not a decision: a connection matching the
/// manifest's `hint` sorts first because it is probably what the author had in
/// mind, and the user still picks. Ties keep configuration order so the list
/// does not reshuffle between launches.
#[must_use]
pub fn candidates<'a>(
    request: &BindingRequest,
    available: &'a [ConnectionRef],
) -> Vec<&'a ConnectionRef> {
    let mut matching: Vec<&ConnectionRef> = available
        .iter()
        .filter(|c| c.satisfies(request.require))
        .collect();
    if let Some(hint) = request.hint.as_deref() {
        let hint = hint.to_ascii_lowercase();
        // A stable sort keeps configuration order within each group.
        matching.sort_by_key(|c| !c.label.to_ascii_lowercase().contains(&hint));
    }
    matching
}

/// Resolves a recorded decision against the connections that exist now.
///
/// Re-checked on every open rather than trusted from the record, because the
/// world moves between launches: a connection can be deleted, or edited into a
/// kind that no longer serves this application.
///
/// # Errors
///
/// - [`PolicyError::BindingMissing`] — the recorded connection is gone.
/// - [`PolicyError::BindingUnsatisfiable`] — it exists but can no longer serve
///   the requirement.
///
/// Both map to `CONNECTION_REQUIRED`, and both should send the reader back to
/// the binding prompt rather than to an error screen: the honest response to
/// "your database is no longer configured" is to ask again.
pub fn resolve<'a>(
    request: &BindingRequest,
    binding: &Binding,
    available: &'a [ConnectionRef],
) -> Result<Resolved<'a>, PolicyError> {
    let id = match binding {
        Binding::KeepLocal => return Ok(Resolved::Local),
        Binding::Connection(id) => id,
    };
    let connection =
        available
            .iter()
            .find(|c| &c.id == id)
            .ok_or_else(|| PolicyError::BindingMissing {
                connection: id.to_string(),
            })?;
    if !connection.satisfies(request.require) {
        return Err(PolicyError::BindingUnsatisfiable {
            connection: id.to_string(),
            require: require_kind_str(request.require),
        });
    }
    Ok(Resolved::Connection(connection))
}

/// The error to raise when an application calls a connection-only method with
/// nothing bound (SPEC §7.2.3 clause 4).
#[must_use]
pub fn unbound(request: &BindingRequest) -> PolicyError {
    PolicyError::NoSatisfyingConnection {
        require: require_kind_str(request.require),
    }
}
