//! Respondent identity (D6).
//!
//! Every submission is signed in NEP-413 format by a local ed25519 key. The key
//! travels inside the envelope with its signature, and the module verifies it
//! without any RPC lookup, since an implicit account's id is its public key.
//!
//! Keys are derived per survey from a local secret; no external identity service
//! is involved.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::response::Response;
use crate::{random_bytes, sha256, Error, Hash, Result};

/// NEP-413 prefix tag: 2^31 + 413.
const NEP413_TAG: u32 = 2_147_484_061;
pub const RECIPIENT: &str = "discovery";

/// What a respondent encrypts into the envelope.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    pub response: Response,
    /// `ed25519:<base58>`
    pub signer: String,
    /// Base64, 32 bytes.
    pub nonce: String,
    /// Base64, 64 bytes, over `discovery:response:<hex sha256(response)>`.
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Identity {
    Key([u8; 32]),
}

impl Identity {
    /// Bytes the module MACs into a storage name. Never leaves the module.
    pub fn name(&self) -> String {
        match self {
            Identity::Key(k) => format!("key:{}", hex::encode(k)),
        }
    }
}

/// A respondent's signing key for one survey.
pub struct RespondentKey(SigningKey);

impl RespondentKey {
    /// A separate key per survey from one local secret.
    pub fn for_survey(local_secret: &[u8; 32], definition_hash: &Hash) -> Result<Self> {
        let info = format!("discovery:v1:respondent:{}", hex::encode(definition_hash));
        let mut seed = [0u8; 32];
        Hkdf::<Sha256>::new(None, local_secret)
            .expand(info.as_bytes(), &mut seed)
            .map_err(|_| Error("hkdf".into()))?;
        Ok(Self(SigningKey::from_bytes(&seed)))
    }

    pub fn public_key(&self) -> String {
        format_key(self.0.verifying_key().as_bytes())
    }

    pub fn sign(&self, response: Response) -> Result<Submission> {
        let nonce = random_bytes::<32>()?;
        let message = response_message(&response)?;
        let signature = self.0.sign(&nep413_hash(&message, &nonce, RECIPIENT));
        Ok(Submission {
            response,
            signer: self.public_key(),
            nonce: B64.encode(nonce),
            signature: B64.encode(signature.to_bytes()),
        })
    }
}

impl Submission {
    /// Verify the per-survey signature and return the private identity.
    pub fn verify(&self) -> Result<Identity> {
        let signer = parse_key(&self.signer)?;
        verify_nep413(
            &signer,
            &response_message(&self.response)?,
            &self.nonce,
            &self.signature,
        )?;
        Ok(Identity::Key(signer))
    }
}

fn response_message(response: &Response) -> Result<String> {
    Ok(format!(
        "discovery:response:{}",
        hex::encode(sha256(&serde_json::to_vec(response)?))
    ))
}

fn format_key(bytes: &[u8; 32]) -> String {
    format!("ed25519:{}", bs58::encode(bytes).into_string())
}

fn parse_key(s: &str) -> Result<[u8; 32]> {
    let b58 = s
        .strip_prefix("ed25519:")
        .ok_or_else(|| Error("key must be ed25519:<base58>".into()))?;
    bs58::decode(b58)
        .into_vec()
        .map_err(|e| Error(format!("key: {e}")))?
        .try_into()
        .map_err(|_| Error("key is not 32 bytes".into()))
}

fn borsh_string(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

/// SHA-256 of the borsh-serialized NEP-413 payload (no callback URL).
pub fn nep413_hash(message: &str, nonce: &[u8; 32], recipient: &str) -> Hash {
    let mut payload = Vec::with_capacity(4 + 4 + message.len() + 32 + 4 + recipient.len() + 1);
    payload.extend_from_slice(&NEP413_TAG.to_le_bytes());
    borsh_string(&mut payload, message);
    payload.extend_from_slice(nonce);
    borsh_string(&mut payload, recipient);
    payload.push(0);
    sha256(&payload)
}

fn verify_nep413(key: &[u8; 32], message: &str, nonce: &str, signature: &str) -> Result<()> {
    let nonce: [u8; 32] = B64
        .decode(nonce)
        .map_err(|e| Error(format!("nonce: {e}")))?
        .try_into()
        .map_err(|_| Error("nonce is not 32 bytes".into()))?;
    let sig_bytes = match signature.strip_prefix("ed25519:") {
        Some(b58) => bs58::decode(b58)
            .into_vec()
            .map_err(|e| Error(format!("signature: {e}")))?,
        None => B64
            .decode(signature)
            .map_err(|e| Error(format!("signature: {e}")))?,
    };
    let sig: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| Error("signature is not 64 bytes".into()))?;
    VerifyingKey::from_bytes(key)
        .map_err(|e| Error(format!("key: {e}")))?
        .verify(
            &nep413_hash(message, &nonce, RECIPIENT),
            &Signature::from_bytes(&sig),
        )
        .map_err(|_| Error("bad signature".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{definition, response};

    fn setup() -> (Response, Hash) {
        let d = definition::sample();
        (response::sample(&d), d.hash().unwrap())
    }

    #[test]
    fn signed_submission_round_trip() {
        let (r, h) = setup();
        let key = RespondentKey::for_survey(&[1u8; 32], &h).unwrap();
        let s = key.sign(r).unwrap();
        assert!(matches!(s.verify().unwrap(), Identity::Key(_)));
    }

    #[test]
    fn per_survey_keys_differ_but_are_stable() {
        let a = RespondentKey::for_survey(&[1u8; 32], &[1u8; 32])
            .unwrap()
            .public_key();
        let a2 = RespondentKey::for_survey(&[1u8; 32], &[1u8; 32])
            .unwrap()
            .public_key();
        let b = RespondentKey::for_survey(&[1u8; 32], &[2u8; 32])
            .unwrap()
            .public_key();
        assert_eq!(a, a2);
        assert_ne!(a, b);
    }

    #[test]
    fn tampered_response_fails() {
        let (r, h) = setup();
        let mut s = RespondentKey::for_survey(&[1u8; 32], &h)
            .unwrap()
            .sign(r)
            .unwrap();
        s.response.answers[0].text.push('!');
        assert!(s.verify().is_err());
    }

    #[test]
    fn unsupported_identity_fields_are_rejected() {
        let (r, h) = setup();
        let submission = RespondentKey::for_survey(&[1; 32], &h)
            .unwrap()
            .sign(r)
            .unwrap();
        let mut value = serde_json::to_value(submission).unwrap();
        value["anchor"] = serde_json::json!({"wallet_account_id": "legacy"});
        assert!(serde_json::from_value::<Submission>(value).is_err());
    }
}
