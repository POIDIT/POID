//! `protected` storage: real encryption of embedded data (SPEC §9.2).
//!
//! AES-256-GCM with the key derived from a user passphrase via Argon2id.
//! This is not a UI lock — a UI lock is defeated by unzipping the file. The
//! plaintext `data/store.json` never touches disk once protection is on; what
//! sits in the container is the [`Envelope`] below.
//!
//! The envelope is self-describing JSON (so a future reader can still parse
//! old files) carrying the Argon2 parameters, the per-file salt, a fresh
//! per-write nonce, and the ciphertext+tag. A wrong passphrase and tampered
//! data are indistinguishable by design — AES-GCM authentication fails the
//! same way for both, and the error says only that it could not be unlocked.
//!
//! Platform-pure: randomness is injected by the caller (the desktop passes
//! the OS RNG, the Web Reader passes `crypto.getRandomValues`), so this
//! module compiles for `wasm32-unknown-unknown` without a getrandom backend.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::{Argon2, Params};
use serde::{Deserialize, Serialize};

use crate::error::{Result, VaultError};

/// The path of the encrypted blob inside a protected container. Replaces
/// `data/store.json`, which must be absent when protection is on.
pub const PROTECTED_PATH: &str = "data/store.enc";

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

/// Argon2id parameters recorded per file, so a file encrypted today still
/// opens after the defaults are raised. These are the current defaults.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KdfParams {
    /// Memory cost in KiB.
    pub m_cost: u32,
    /// Time cost (iterations).
    pub t_cost: u32,
    /// Parallelism (lanes).
    pub p_cost: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        // 19 MiB / 2 passes / 1 lane — the OWASP Argon2id baseline; a couple
        // hundred ms on a desktop, tolerable for a one-per-open unlock.
        Self {
            m_cost: 19 * 1024,
            t_cost: 2,
            p_cost: 1,
        }
    }
}

/// The on-disk protected blob (`data/store.enc`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    /// Format version (1).
    pub v: u32,
    /// AEAD identifier (`aes-256-gcm`).
    pub alg: String,
    /// KDF identifier (`argon2id`).
    pub kdf: String,
    /// Argon2id parameters used for this file.
    pub params: KdfParams,
    /// Per-file salt, lowercase hex.
    pub salt: String,
    /// Per-write nonce, lowercase hex — unique on every write (SPEC §9.2).
    pub nonce: String,
    /// Ciphertext + 16-byte GCM tag, lowercase hex.
    pub ct: String,
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn unhex(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return Err(VaultError::Crypto {
            message: "malformed envelope".to_owned(),
        });
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| VaultError::Crypto {
                message: "malformed envelope".to_owned(),
            })
        })
        .collect()
}

fn derive_key(passphrase: &[u8], salt: &[u8], params: &KdfParams) -> Result<[u8; 32]> {
    let argon = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        Params::new(params.m_cost, params.t_cost, params.p_cost, Some(32)).map_err(|e| {
            VaultError::Crypto {
                message: format!("bad kdf params: {e}"),
            }
        })?,
    );
    let mut key = [0u8; 32];
    argon
        .hash_password_into(passphrase, salt, &mut key)
        .map_err(|_| VaultError::Crypto {
            message: "key derivation failed".to_owned(),
        })?;
    Ok(key)
}

/// Encrypts `plaintext` under `passphrase`. `salt` (16 bytes) and `nonce`
/// (12 bytes) are supplied by the caller and MUST be freshly random for each
/// write — a reused nonce breaks GCM.
pub fn seal(
    plaintext: &[u8],
    passphrase: &[u8],
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
    params: KdfParams,
) -> Result<Envelope> {
    let key = derive_key(passphrase, &salt, &params)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|_| VaultError::Crypto {
            message: "encryption failed".to_owned(),
        })?;
    Ok(Envelope {
        v: 1,
        alg: "aes-256-gcm".to_owned(),
        kdf: "argon2id".to_owned(),
        params,
        salt: hex(&salt),
        nonce: hex(&nonce),
        ct: hex(&ct),
    })
}

/// Decrypts an envelope. Returns the same `Crypto` error for a wrong
/// passphrase and for tampered data — the two are not distinguishable, and
/// must not be distinguished in a message.
pub fn open(envelope: &Envelope, passphrase: &[u8]) -> Result<Vec<u8>> {
    if envelope.alg != "aes-256-gcm" || envelope.kdf != "argon2id" {
        return Err(VaultError::Crypto {
            message: "unsupported protection scheme".to_owned(),
        });
    }
    let salt = unhex(&envelope.salt)?;
    let nonce = unhex(&envelope.nonce)?;
    let ct = unhex(&envelope.ct)?;
    if nonce.len() != NONCE_LEN {
        return Err(VaultError::Crypto {
            message: "malformed envelope".to_owned(),
        });
    }
    let key = derive_key(passphrase, &salt, &envelope.params)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    cipher
        .decrypt(Nonce::from_slice(&nonce), ct.as_slice())
        .map_err(|_| VaultError::Crypto {
            message: "wrong passphrase or the data was tampered with".to_owned(),
        })
}

/// Serializes an envelope to the bytes stored at [`PROTECTED_PATH`].
pub fn to_bytes(envelope: &Envelope) -> Result<Vec<u8>> {
    serde_json::to_vec(envelope).map_err(|e| VaultError::Crypto {
        message: e.to_string(),
    })
}

/// Parses envelope bytes from [`PROTECTED_PATH`].
pub fn from_bytes(bytes: &[u8]) -> Result<Envelope> {
    serde_json::from_slice(bytes).map_err(|_| VaultError::Crypto {
        message: "the protected blob is not a valid envelope".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::{from_bytes, open, seal, to_bytes, KdfParams};

    // Test-only fast parameters: real KDF cost would make the suite crawl.
    fn fast() -> KdfParams {
        KdfParams {
            m_cost: 256,
            t_cost: 1,
            p_cost: 1,
        }
    }

    #[test]
    fn round_trips_with_the_right_passphrase() {
        let env = seal(b"{\"secret\":true}", b"hunter2", [7; 16], [3; 12], fast()).unwrap();
        assert_eq!(open(&env, b"hunter2").unwrap(), b"{\"secret\":true}");
    }

    #[test]
    fn a_wrong_passphrase_fails_without_saying_which() {
        let env = seal(b"data", b"correct horse", [1; 16], [2; 12], fast()).unwrap();
        let err = open(&env, b"wrong").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("tampered") || msg.contains("passphrase"));
        // The message never confirms the passphrase was the specific problem.
        assert!(!msg.contains("incorrect passphrase"));
    }

    #[test]
    fn tampering_with_the_ciphertext_is_rejected() {
        let mut env = seal(b"data", b"pw", [1; 16], [2; 12], fast()).unwrap();
        // Flip one ciphertext nibble.
        let mut ct: Vec<char> = env.ct.chars().collect();
        ct[0] = if ct[0] == 'a' { 'b' } else { 'a' };
        env.ct = ct.into_iter().collect();
        assert!(open(&env, b"pw").is_err());
    }

    #[test]
    fn envelope_survives_serialization() {
        let env = seal(b"payload", b"pw", [9; 16], [8; 12], fast()).unwrap();
        let bytes = to_bytes(&env).unwrap();
        let parsed = from_bytes(&bytes).unwrap();
        assert_eq!(open(&parsed, b"pw").unwrap(), b"payload");
    }

    #[test]
    fn a_fresh_nonce_changes_the_ciphertext() {
        let a = seal(b"same", b"pw", [1; 16], [1; 12], fast()).unwrap();
        let b = seal(b"same", b"pw", [1; 16], [2; 12], fast()).unwrap();
        assert_ne!(a.ct, b.ct, "distinct nonces must yield distinct ciphertext");
    }
}
