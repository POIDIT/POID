//! Vault error types.

use thiserror::Error;

/// Everything that can go wrong inside the vault.
#[derive(Debug, Error)]
pub enum VaultError {
    /// The store or the index could not be read or written.
    #[error("vault I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Stored bytes are not a loadable Automerge document.
    #[error("vault entry is corrupt: {message}")]
    Corrupt {
        /// Detail from the CRDT engine.
        message: String,
    },

    /// A write would push the instance past its quota (SPEC storage.quota_mb).
    ///
    /// Maps to the broker's `QUOTA_EXCEEDED`: a misbehaving application must
    /// not be able to fill the disk.
    #[error("quota exceeded: {projected} bytes would exceed the {limit}-byte quota")]
    QuotaExceeded {
        /// Bytes the store would hold after the rejected write.
        projected: u64,
        /// The configured limit in bytes.
        limit: u64,
    },

    /// A value is not valid JSON (the engine stores atomic JSON values).
    #[error("value is not valid JSON: {message}")]
    InvalidValue {
        /// Serialization detail.
        message: String,
    },

    /// Encryption or decryption failed (wrong passphrase or tampered data).
    #[error("protected data could not be unlocked: {message}")]
    Crypto {
        /// Deliberately vague: never leaks which of passphrase/integrity failed
        /// beyond what AES-GCM authentication inherently reveals.
        message: String,
    },
}

impl From<automerge::AutomergeError> for VaultError {
    fn from(e: automerge::AutomergeError) -> Self {
        VaultError::Corrupt {
            message: e.to_string(),
        }
    }
}

/// Convenience alias used across the crate.
pub type Result<T> = std::result::Result<T, VaultError>;
