//! The error codes that cross the boundary, and nothing else.
//!
//! `RUNTIME-API.md` §9 defines six codes. They are the *entire* vocabulary an
//! application receives when something goes wrong: a backend's own message is
//! dropped rather than forwarded (SPEC §7.2.4), because a backend that quotes
//! its connection string inside an error is a realistic occurrence and the
//! boundary is where it has to stop.

use thiserror::Error;

/// A `RUNTIME-API.md` §9 error code.
///
/// This enum is deliberately closed and small. Adding a variant changes the
/// public conformance contract, so it is a spec decision before it is a code
/// decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// Capability not declared, or not granted by the user.
    PermissionDenied,
    /// Storage quota exhausted.
    QuotaExceeded,
    /// Capability not supported by this reader or platform.
    NotAvailable,
    /// The call needs a Connection and none is bound (SPEC §7.2.3).
    ConnectionRequired,
    /// Bad input from the application.
    InvalidArgument,
    /// Reader-side failure.
    Internal,
}

impl ErrorCode {
    /// The wire form, exactly as `RUNTIME-API.md` §9 spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::QuotaExceeded => "QUOTA_EXCEEDED",
            Self::NotAvailable => "NOT_AVAILABLE",
            Self::ConnectionRequired => "CONNECTION_REQUIRED",
            Self::InvalidArgument => "INVALID_ARGUMENT",
            Self::Internal => "INTERNAL",
        }
    }

    /// Reader-authored text for this code, written for a person (CONVENTIONS,
    /// UX rule 4).
    ///
    /// Every string returned here is a compile-time constant. That is the
    /// point: text assembled from a backend's response could carry a
    /// credential, and text that cannot vary cannot carry anything.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::PermissionDenied => "this application has not been granted that",
            Self::QuotaExceeded => "this application has used all the space it is allowed",
            Self::NotAvailable => "this reader cannot do that",
            Self::ConnectionRequired => "this application needs a connection that is not set up",
            Self::InvalidArgument => "the request was not valid",
            Self::Internal => "something went wrong inside the reader",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A refusal that is safe to hand to a sandboxed application.
///
/// Constructing one is the only way to produce something the boundary will
/// forward, and it carries no free-form text — see [`ErrorCode::message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("{code}: {}", code.message())]
pub struct SafeError {
    /// The §9 code.
    pub code: ErrorCode,
}

impl SafeError {
    /// Builds a safe refusal.
    #[must_use]
    pub const fn new(code: ErrorCode) -> Self {
        Self { code }
    }
}

impl From<ErrorCode> for SafeError {
    fn from(code: ErrorCode) -> Self {
        Self::new(code)
    }
}

/// Errors this crate's policy checks produce internally.
///
/// These are richer than [`SafeError`] because they are for the reader's own
/// diagnostics — the Studio log the *user* can read. Convert with
/// [`PolicyError::safe`] before anything crosses to an application.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PolicyError {
    /// The origin is absent from the manifest allowlist or not user-approved.
    #[error("origin `{origin}` is not in this application's approved allowlist")]
    OriginNotAllowed {
        /// The origin that was refused.
        origin: String,
    },
    /// The destination resolved to an address the broker will not connect to.
    #[error("`{host}` resolves to {address}, which is {reason}")]
    AddressBlocked {
        /// The hostname that was being resolved.
        host: String,
        /// The address it resolved to.
        address: std::net::IpAddr,
        /// Why the address is refused.
        reason: &'static str,
    },
    /// The hostname resolved to nothing usable.
    #[error("`{host}` did not resolve to any address the broker may use")]
    NoUsableAddress {
        /// The hostname that was being resolved.
        host: String,
    },
    /// The string is not a usable origin.
    #[error("`{value}` is not a usable origin ({why})")]
    MalformedOrigin {
        /// The offending value.
        value: String,
        /// Why it was refused.
        why: &'static str,
    },
    /// No configured connection satisfies the application's requirement.
    #[error("no configured connection can serve a `{require}` requirement")]
    NoSatisfyingConnection {
        /// The requirement kind, as spelled in the manifest.
        require: &'static str,
    },
    /// The recorded binding names a connection that cannot serve the need.
    #[error("connection `{connection}` cannot serve a `{require}` requirement")]
    BindingUnsatisfiable {
        /// The bound connection's id.
        connection: String,
        /// The requirement kind, as spelled in the manifest.
        require: &'static str,
    },
    /// The recorded binding names a connection that no longer exists.
    #[error("connection `{connection}` is no longer configured")]
    BindingMissing {
        /// The bound connection's id.
        connection: String,
    },
}

impl PolicyError {
    /// The code an application is allowed to see for this failure.
    ///
    /// Note how much is thrown away: which origin, which address, which
    /// connection. An application learns *that* it was refused, never enough
    /// to map the user's network or enumerate their connections.
    #[must_use]
    pub const fn safe(&self) -> SafeError {
        SafeError::new(match self {
            Self::OriginNotAllowed { .. } | Self::AddressBlocked { .. } => {
                ErrorCode::PermissionDenied
            }
            Self::NoUsableAddress { .. } => ErrorCode::NotAvailable,
            Self::MalformedOrigin { .. } => ErrorCode::InvalidArgument,
            Self::NoSatisfyingConnection { .. }
            | Self::BindingUnsatisfiable { .. }
            | Self::BindingMissing { .. } => ErrorCode::ConnectionRequired,
        })
    }
}
