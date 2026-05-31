//! ed25519 primitives + serde helpers for signed writs.
//!
//! Public keys and signatures are serialized as lowercase hex strings in
//! canonical JSON so that the digest over a `WritBody` is deterministic and
//! doesn't depend on serde_json's byte-array encoding.

pub use ed25519_dalek::{SigningKey, VerifyingKey};

use ed25519_dalek::Signer;
use rand::rngs::OsRng;
use serde::{Deserialize, Deserializer, Serializer};

use crate::error::{Error, Result};

pub type PublicKey = [u8; 32];
pub type SignatureBytes = [u8; 64];

/// Generate a fresh ed25519 keypair using the OS RNG.
pub fn generate_signing_key() -> SigningKey {
    SigningKey::generate(&mut OsRng)
}

/// 16 random bytes for a writ nonce — makes each issued writ uniquely
/// identified (and signed), so two otherwise-identical writs do not collide on
/// the same content-addressed `WritId` and each can be revoked independently.
pub fn random_nonce() -> [u8; 16] {
    use rand::RngCore;
    let mut nonce = [0u8; 16];
    OsRng.fill_bytes(&mut nonce);
    nonce
}

/// Extract the verifying (public) key bytes from a signing key.
pub fn public_key_of(sk: &SigningKey) -> PublicKey {
    sk.verifying_key().to_bytes()
}

/// Sign `message` with `sk` and return the raw 64-byte signature.
pub fn sign(sk: &SigningKey, message: &[u8]) -> SignatureBytes {
    sk.sign(message).to_bytes()
}

/// Verify a detached ed25519 signature. Returns `AuthorityVoid` on failure.
pub fn verify(pk: &PublicKey, message: &[u8], sig: &SignatureBytes) -> Result<()> {
    let vk = VerifyingKey::from_bytes(pk)
        .map_err(|e| Error::AuthorityVoid(format!("bad pubkey: {e}")))?;
    let signature = ed25519_dalek::Signature::from_bytes(sig);
    vk.verify_strict(message, &signature)
        .map_err(|e| Error::AuthorityVoid(format!("bad signature: {e}")))
}

// ---------- serde helpers --------------------------------------------------

pub mod hex32 {
    use super::*;

    pub fn serialize<S: Serializer>(
        bytes: &[u8; 32],
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        let v = hex::decode(&s).map_err(serde::de::Error::custom)?;
        v.try_into()
            .map_err(|_| serde::de::Error::custom("expected 32 bytes"))
    }
}

pub mod hex64 {
    use super::*;

    pub fn serialize<S: Serializer>(
        bytes: &[u8; 64],
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<[u8; 64], D::Error> {
        let s = String::deserialize(d)?;
        let v = hex::decode(&s).map_err(serde::de::Error::custom)?;
        v.try_into()
            .map_err(|_| serde::de::Error::custom("expected 64 bytes"))
    }
}
