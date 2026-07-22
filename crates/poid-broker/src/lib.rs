//! POID broker: every privileged operation passes through here.
//!
//! The broker checks the manifest declaration and the user's grants before
//! performing any operation on behalf of a sandboxed application (SPEC §7).
//! Credentials are held by the reader and NEVER enter the sandbox — nor any
//! other browser-engine context, including the reader's own UI (SPEC §7.1).
//!
//! # What this crate is, and is not
//!
//! This crate is the **policy**: given a manifest, a user's grants, a set of
//! configured connections and a destination address, may this happen? It
//! performs no IO, opens no sockets, reads no keychain, and holds no secret.
//! Everything here is a pure function over data, which is what makes it
//! exhaustively testable — including with `proptest`, over inputs an attacker
//! chooses.
//!
//! The **mechanism** — the keychain, the sockets, the database drivers — lives
//! in `poid-connections` and in the Studio host. That split is deliberate: the
//! decisions worth reviewing carefully are all here, in a crate that cannot
//! accidentally perform an effect.
//!
//! # Security
//!
//! Any change to this crate requires an explicit security review note in the
//! PR (see `CONTRIBUTING.md`). Read `spec/SECURITY.md` first. Never trust a
//! scope identifier sent by the sandbox — derive scope from the window.

pub mod binding;
pub mod connection;
pub mod error;
pub mod network;
pub mod scrub;

pub use binding::{Binding, BindingRequest, Resolved};
pub use connection::{ConnectionId, ConnectionKind, ConnectionRef};
pub use error::{ErrorCode, PolicyError, SafeError};
pub use network::{classify, NetworkPolicy, Origin};
pub use scrub::Redactor;

/// Default posture for every permission not explicitly granted (SPEC §9.1).
pub const DEFAULT_POSTURE: &str = "deny";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_posture_is_deny() {
        assert_eq!(DEFAULT_POSTURE, "deny");
    }
}
