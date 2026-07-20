//! Ed25519 signatures over the application content (SPEC §9.3).
//!
//! The signed payload is a canonical JSON projection of the manifest that
//! covers the publisher's attestation — identity, runtime, entry,
//! permissions and the integrity digests — and deliberately excludes what
//! legitimately changes after signing: `instance` (assigned per copy,
//! SPEC §6.3), `storage` (the user's choice, SPEC §6.1) and `draft`.
//! Because the integrity digests are included, a valid signature
//! transitively attests every byte of `app/` and `deps/`.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::error::PoidError;
use crate::manifest::{
    AppInfo, ContainerType, DataRef, ExtraFields, Manifest, Permissions, Runtime,
};

/// Container path of the signature file (SPEC §9.3.1).
pub const SIGNATURE_PATH: &str = "signature/signature.json";

/// The parsed `signature/signature.json` (SPEC §9.3.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignatureBlock {
    /// Signature format version. Only `1` is defined.
    pub version: u32,
    /// Signature algorithm. Only `ed25519` is defined.
    pub algo: String,
    /// Ed25519 public key, 32 bytes, lowercase hex.
    pub public_key: String,
    /// Ed25519 signature over the canonical payload, 64 bytes, lowercase hex.
    pub signature: String,
    /// Unknown fields, preserved verbatim.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// Result of checking a container's signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureStatus {
    /// No `signature/signature.json` present. Not an error — signing is
    /// optional (SPEC §9.3).
    Unsigned,
    /// The signature verifies against the manifest.
    Valid {
        /// The signer's Ed25519 public key, lowercase hex.
        public_key: String,
    },
    /// The signature file is well-formed but does not verify — the content
    /// changed after signing, or the signature is forged.
    Invalid,
}

/// Canonical signed payload (SPEC §9.3.2). Field order is normative.
#[derive(Serialize)]
struct Payload<'a> {
    poid: &'a str,
    #[serde(rename = "type")]
    container_type: ContainerType,
    #[serde(skip_serializing_if = "Option::is_none")]
    app: Option<&'a AppInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime: Option<&'a Runtime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entry: Option<&'a String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    permissions: Option<&'a Permissions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shared_scope: Option<&'a Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_ref: Option<&'a DataRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    integrity_app: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    integrity_deps: Option<&'a str>,
}

/// The canonical byte sequence the signature covers (SPEC §9.3.2).
pub fn signature_payload_bytes(manifest: &Manifest) -> Result<Vec<u8>, PoidError> {
    let (integrity_app, integrity_deps) = match &manifest.integrity {
        Some(i) => (i.app.as_deref(), i.deps.as_deref()),
        None => (None, None),
    };
    let payload = Payload {
        poid: &manifest.poid,
        container_type: manifest.container_type,
        app: manifest.app.as_ref(),
        runtime: manifest.runtime.as_ref(),
        entry: manifest.entry.as_ref(),
        permissions: manifest.permissions.as_ref(),
        shared_scope: manifest.shared_scope.as_ref(),
        data_ref: manifest.data_ref.as_ref(),
        integrity_app,
        integrity_deps,
    };
    serde_json::to_vec(&payload).map_err(|e| PoidError::SignatureMalformed {
        reason: format!("payload serialization: {e}"),
    })
}

/// Signs the manifest's canonical payload with an Ed25519 private key seed.
pub(crate) fn sign_manifest(
    manifest: &Manifest,
    private_key_seed: &[u8; 32],
) -> Result<SignatureBlock, PoidError> {
    let signing_key = SigningKey::from_bytes(private_key_seed);
    let payload = signature_payload_bytes(manifest)?;
    let signature = signing_key.sign(&payload);
    Ok(SignatureBlock {
        version: 1,
        algo: "ed25519".to_owned(),
        public_key: to_hex(signing_key.verifying_key().as_bytes()),
        signature: to_hex(&signature.to_bytes()),
        extra: ExtraFields::new(),
    })
}

/// Verifies a signature block's bytes against the manifest.
pub(crate) fn verify_block(
    manifest: &Manifest,
    block_bytes: &[u8],
) -> Result<SignatureStatus, PoidError> {
    let malformed = |reason: &str| PoidError::SignatureMalformed {
        reason: reason.to_owned(),
    };
    let block: SignatureBlock =
        serde_json::from_slice(block_bytes).map_err(|e| malformed(&e.to_string()))?;
    if block.version != 1 {
        return Err(malformed("unsupported signature version"));
    }
    if block.algo != "ed25519" {
        return Err(malformed("unsupported signature algorithm"));
    }
    let key_bytes: [u8; 32] = decode_hex(&block.public_key)
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| malformed("public_key must be 32 bytes of lowercase hex"))?;
    let sig_bytes: [u8; 64] = decode_hex(&block.signature)
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| malformed("signature must be 64 bytes of lowercase hex"))?;
    let Ok(verifying_key) = VerifyingKey::from_bytes(&key_bytes) else {
        return Err(malformed("public_key is not a valid Ed25519 key"));
    };
    let signature = Signature::from_bytes(&sig_bytes);
    let payload = signature_payload_bytes(manifest)?;
    // `verify_strict` additionally rejects malleable and small-order keys.
    match verifying_key.verify_strict(&payload, &signature) {
        Ok(()) => Ok(SignatureStatus::Valid {
            public_key: block.public_key,
        }),
        Err(_) => Ok(SignatureStatus::Invalid),
    }
}

/// Lowercase hex encoding.
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// Lowercase hex decoding; `None` on any non-hex character or odd length.
fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let nibble = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        }
    };
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        out.push(nibble(pair[0])? << 4 | nibble(pair[1])?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::StorageMode;
    use crate::poid::Poid;
    use std::collections::BTreeMap;

    const SEED: [u8; 32] = [7u8; 32];

    fn sample_poid() -> Poid {
        let manifest = Manifest::new_app("com.example.x", "X", "1.0.0", "app/index.html");
        let mut files = BTreeMap::new();
        files.insert("app/index.html".to_owned(), b"<html>".to_vec());
        Poid::from_parts(manifest, files)
    }

    #[test]
    fn sign_then_verify_is_valid() {
        let mut poid = sample_poid();
        assert_eq!(
            poid.signature_status().expect("status"),
            SignatureStatus::Unsigned
        );
        poid.sign(&SEED).expect("sign");
        let status = poid.signature_status().expect("status");
        let expected_key = to_hex(SigningKey::from_bytes(&SEED).verifying_key().as_bytes());
        assert_eq!(
            status,
            SignatureStatus::Valid {
                public_key: expected_key
            }
        );
    }

    #[test]
    fn content_change_invalidates_signature() {
        let mut poid = sample_poid();
        poid.sign(&SEED).expect("sign");
        // Simulate tampering: change app content without re-signing.
        let manifest = poid.manifest().clone();
        let mut files: BTreeMap<String, Vec<u8>> = poid
            .files()
            .map(|(p, c)| (p.to_owned(), c.to_vec()))
            .collect();
        files.insert("app/index.html".to_owned(), b"<html>EVIL".to_vec());
        // Refresh digests the way a re-pack would; the signature still covers
        // the old ones.
        let mut manifest = manifest;
        crate::integrity::refresh(&mut manifest, &files);
        let tampered = Poid::from_parts(manifest, files);
        assert_eq!(
            tampered.signature_status().expect("status"),
            SignatureStatus::Invalid
        );
    }

    #[test]
    fn instance_and_storage_changes_keep_signature_valid() {
        let mut poid = sample_poid();
        poid.sign(&SEED).expect("sign");
        poid.set_instance_id(crate::Uuid::from_u128(42));
        poid.set_data(b"{\"user\":\"data\"}");
        poid.convert_storage_mode(StorageMode::Vault);
        assert!(matches!(
            poid.signature_status().expect("status"),
            SignatureStatus::Valid { .. }
        ));
    }

    #[test]
    fn malformed_blocks_are_rejected() {
        let manifest = Manifest::new_app("com.example.x", "X", "1.0.0", "app/index.html");
        for bad in [
            &b"not json"[..],
            br#"{"version":2,"algo":"ed25519","public_key":"","signature":""}"#,
            br#"{"version":1,"algo":"rsa","public_key":"","signature":""}"#,
            br#"{"version":1,"algo":"ed25519","public_key":"zz","signature":"00"}"#,
        ] {
            let err = verify_block(&manifest, bad).expect_err("must be malformed");
            assert_eq!(err.code(), "signature-malformed");
        }
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(
            decode_hex(&to_hex(&[0x00, 0xff, 0x1a])),
            Some(vec![0x00, 0xff, 0x1a])
        );
        assert_eq!(decode_hex("0"), None);
        assert_eq!(decode_hex("ZZ"), None);
    }
}
