//! POID broker: every privileged operation passes through here.
//!
//! The broker checks the manifest declaration and the user's grants before
//! performing any operation on behalf of a sandboxed application (SPEC §7).
//! Credentials are held by the reader and NEVER enter the sandbox (SPEC §7.1).
//!
//! # Security
//!
//! Any change to this crate requires an explicit security review note in the
//! PR (see `CONTRIBUTING.md`). Read `spec/SECURITY.md` first. Never trust a
//! scope identifier sent by the sandbox — derive scope from the window.

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
