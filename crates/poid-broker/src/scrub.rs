//! Redaction for the diagnostics the **user** reads (SPEC §7.2.4).
//!
//! Two different audiences, two different rules:
//!
//! - **The application** gets a [`crate::error::SafeError`] — a code and a
//!   constant string. Nothing derived from a backend reaches it, so there is
//!   nothing here to protect.
//! - **The user** gets the real diagnostic, in Studio's log, because a person
//!   debugging their own database deserves the actual message. That text came
//!   from a backend, and backends do quote connection strings in errors.
//!
//! This module serves the second audience. It is **defence in depth, not the
//! control**: the control is that a credential is never placed into text that
//! travels. Redaction catches the case where a backend put it there for us.

use std::collections::BTreeSet;

/// What redaction replaces a secret with.
pub const REDACTED: &str = "[redacted]";

/// The shortest secret worth searching for.
///
/// Below this, a "secret" is likely to occur in ordinary text by coincidence
/// (a two-character password would redact half the alphabet), and redacting
/// everywhere it appears would destroy the message without protecting
/// anything. A credential this short is a problem to reject at configuration
/// time, not to paper over here.
pub const MIN_REDACTABLE_LEN: usize = 6;

/// Removes known secrets from diagnostic text.
///
/// Holds the secret values themselves, so it lives only in the process that
/// already holds them (Core) and is never constructed in a web context.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    // Ordered longest-first so that a secret containing another secret is
    // replaced whole rather than being shredded from the inside out.
    secrets: Vec<String>,
}

impl Redactor {
    /// An empty redactor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a secret to strip from future diagnostics.
    ///
    /// Values shorter than [`MIN_REDACTABLE_LEN`] are ignored — see the
    /// constant for why that is deliberate rather than sloppy.
    pub fn insert(&mut self, secret: impl Into<String>) {
        let secret = secret.into();
        if secret.len() < MIN_REDACTABLE_LEN || self.secrets.contains(&secret) {
            return;
        }
        self.secrets.push(secret);
        self.secrets
            .sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    }

    /// Registers several secrets.
    pub fn extend<I, S>(&mut self, secrets: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for secret in secrets {
            self.insert(secret);
        }
    }

    /// How many secrets are registered. For assertions, not for display.
    #[must_use]
    pub fn len(&self) -> usize {
        self.secrets.len()
    }

    /// Whether nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }

    /// Returns `text` with every registered secret replaced.
    ///
    /// Also strips the obvious encodings a secret picks up in transit — a
    /// connection string that has been through a URL builder arrives
    /// percent-encoded, and a literal-only search would sail straight past it.
    #[must_use]
    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for secret in &self.secrets {
            for form in encodings(secret) {
                if out.contains(&form) {
                    out = out.replace(&form, REDACTED);
                }
            }
        }
        out
    }

    /// Whether `text` still contains any registered secret in any known form.
    ///
    /// The assertion a test wants: `redact` should make this false, and if it
    /// does not, that is a bug worth failing on rather than shipping.
    #[must_use]
    pub fn leaks(&self, text: &str) -> bool {
        self.secrets
            .iter()
            .flat_map(|s| encodings(s))
            .any(|form| text.contains(&form))
    }
}

/// The forms one secret can take in text a backend hands back.
fn encodings(secret: &str) -> BTreeSet<String> {
    let mut forms = BTreeSet::new();
    forms.insert(secret.to_owned());
    forms.insert(percent_encode(secret));
    forms
}

/// Percent-encodes the characters a URL builder would, and only those.
///
/// Not a general-purpose encoder: it exists to recognise our own secret after
/// a URL library has been at it, so it must match what such a library emits
/// for the RFC 3986 unreserved set.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
