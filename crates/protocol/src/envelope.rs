//! Encryption to the survey key, with padding to one fixed size.
//!
//! X25519 ECDH with a fresh ephemeral key, HKDF-SHA256, XChaCha20-Poly1305:
//! the same construction NEAR AI uses for end-to-end encryption.
//!
//! Wire format: `0x01 || ephemeral_public (32) || nonce (24) || ciphertext+tag`.
//! The associated data is the definition hash, so an envelope can't be replayed
//! into another survey.
//!
//! Plaintext is padded to [`PADDED`] before sealing, so every envelope has the
//! same length. OutLayer's public job log shows each run's instruction count,
//! which would otherwise reveal response length. The size also keeps each
//! response short enough for its human to review and approve.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::{err, random_bytes, Error, Hash, Result};

pub const FORMAT: u8 = 0x01;
const INFO: &[u8] = b"discovery:v1:envelope";
const HEADER: usize = 1 + 32 + 24;

/// Padded plaintext size of every envelope.
pub const PADDED: usize = 4 * 1024;
/// Largest response, in serialized bytes; anything larger is refused.
pub const MAX_RESPONSE: usize = PADDED - 4;

/// A survey's X25519 keypair. The secret exists only inside the module.
pub struct SurveyKey(StaticSecret);

impl SurveyKey {
    pub fn generate() -> Result<Self> {
        Ok(Self(StaticSecret::from(random_bytes::<32>()?)))
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(StaticSecret::from(bytes))
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    pub fn public(&self) -> [u8; 32] {
        PublicKey::from(&self.0).to_bytes()
    }

    pub fn open(&self, envelope: &[u8], definition_hash: &Hash) -> Result<Vec<u8>> {
        if envelope.len() < HEADER + 16 || envelope[0] != FORMAT {
            return err("not an envelope");
        }
        let ephemeral: [u8; 32] = envelope[1..33].try_into().expect("32 bytes");
        let nonce = XNonce::from_slice(&envelope[33..HEADER]);
        let shared = self.0.diffie_hellman(&PublicKey::from(ephemeral));
        let cipher = cipher(shared.as_bytes(), &ephemeral, &self.public())?;
        let padded = cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &envelope[HEADER..],
                    aad: definition_hash,
                },
            )
            .map_err(|_| Error("envelope does not open".into()))?;
        unpad(&padded)
    }
}

pub fn seal(survey_public: &[u8; 32], plaintext: &[u8], definition_hash: &Hash) -> Result<Vec<u8>> {
    let ephemeral = StaticSecret::from(random_bytes::<32>()?);
    let ephemeral_public = PublicKey::from(&ephemeral).to_bytes();
    let shared = ephemeral.diffie_hellman(&PublicKey::from(*survey_public));
    let cipher = cipher(shared.as_bytes(), &ephemeral_public, survey_public)?;
    let nonce = random_bytes::<24>()?;
    let sealed = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &pad(plaintext)?,
                aad: definition_hash,
            },
        )
        .map_err(|_| Error("encryption failed".into()))?;
    let mut out = Vec::with_capacity(HEADER + sealed.len());
    out.push(FORMAT);
    out.extend_from_slice(&ephemeral_public);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    Ok(out)
}

fn cipher(
    shared: &[u8; 32],
    ephemeral_public: &[u8; 32],
    recipient: &[u8; 32],
) -> Result<XChaCha20Poly1305> {
    if shared.iter().all(|b| *b == 0) {
        return err("low-order public key");
    }
    let mut salt = [0u8; 64];
    salt[..32].copy_from_slice(ephemeral_public);
    salt[32..].copy_from_slice(recipient);
    let mut key = [0u8; 32];
    Hkdf::<Sha256>::new(Some(&salt), shared)
        .expand(INFO, &mut key)
        .map_err(|_| Error("hkdf".into()))?;
    Ok(XChaCha20Poly1305::new(&key.into()))
}

/// `u32 length (LE) || data || zeros` up to [`PADDED`].
pub fn pad(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() > MAX_RESPONSE {
        return err(format!(
            "response too large: {} bytes (max {MAX_RESPONSE}); shorten the answers",
            data.len()
        ));
    }
    let mut out = Vec::with_capacity(PADDED);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    out.resize(PADDED, 0);
    Ok(out)
}

pub fn unpad(padded: &[u8]) -> Result<Vec<u8>> {
    if padded.len() != PADDED {
        return err("padded length is wrong");
    }
    let len = u32::from_le_bytes(padded[..4].try_into().expect("4 bytes")) as usize;
    if 4 + len > padded.len() || padded[4 + len..].iter().any(|b| *b != 0) {
        return err("bad padding");
    }
    Ok(padded[4..4 + len].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: Hash = [7u8; 32];

    #[test]
    fn bound_to_definition() {
        let key = SurveyKey::generate().unwrap();
        let env = seal(&key.public(), b"hello", &H).unwrap();
        assert!(key.open(&env, &[8u8; 32]).is_err());
    }

    #[test]
    fn wrong_key_and_tampering_fail() {
        let key = SurveyKey::generate().unwrap();
        let other = SurveyKey::generate().unwrap();
        let mut env = seal(&key.public(), b"hello", &H).unwrap();
        assert_eq!(key.open(&env, &H).unwrap(), b"hello");
        assert!(other.open(&env, &H).is_err());
        let last = env.len() - 1;
        env[last] ^= 1;
        assert!(key.open(&env, &H).is_err());
    }

    #[test]
    fn every_envelope_has_the_same_length() {
        let key = SurveyKey::generate().unwrap();
        let a = seal(&key.public(), b"a", &H).unwrap();
        let b = seal(&key.public(), &[b'x'; MAX_RESPONSE], &H).unwrap();
        assert_eq!(a.len(), b.len());
    }

    #[test]
    fn padding_limits() {
        assert!(pad(&vec![0u8; MAX_RESPONSE]).is_ok());
        assert!(pad(&vec![0u8; MAX_RESPONSE + 1]).is_err());
        let mut p = pad(b"abc").unwrap();
        p[10] = 1;
        assert!(unpad(&p).is_err());
    }

    #[test]
    fn low_order_point_refused() {
        assert!(seal(&[0u8; 32], b"x", &H).is_err());
    }
}
