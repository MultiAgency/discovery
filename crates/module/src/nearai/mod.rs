//! Confidential inference for `Finalize` on NEAR AI Cloud, verified inside
//! the enclave.
//!
//! The first inference call fetches fresh attestation evidence for the model
//! and verifies it: an Intel TDX quote bound to our nonce and the model's
//! signing key, and NVIDIA's signed GPU verdict for that nonce. Which software runs on that
//! hardware is NEAR AI's deployment, trusted rather than independently approved
//! here: a quote proves hardware and measured configuration, not reviewed model
//! code. Each request is end-to-end encrypted to the attested model key (E2EE
//! v3, binding the reply key and request field), so the gateway relays only
//! ciphertext and is not itself checked, and a
//! reply is used only after the model enclave's signature over the exact
//! request and response bytes checks out. Errors are public on-chain output,
//! so they carry no request, reply or provider text.
//!
//! `attest`, `e2ee`, `net` and `sign` derive from
//! `MultiAgency/private-ai@dd469a5` (`app/src`), with local changes: only the
//! model requests are encrypted to is verified, and its platform must be fully
//! patched; weak Ed25519 keys and non-contributory X25519 exchanges are
//! refused; v3 request encryption authenticates the reply key and field label;
//! only text messages are supported; tool calls, overrides, offline
//! inputs and persisted receipts are removed. Tests inject a quote verifier and
//! sign synthetic NVIDIA-format JWTs; they check protocol behavior, not real
//! hardware or a deployed service.
//!
//! Reference: https://docs.near.ai/cloud/verification/cloud-api/model-attestations

mod attest;
mod e2ee;
mod net;

use std::cell::OnceCell;

use discovery_protocol::{Error, Result};
use serde_json::{json, Value};

use crate::finalize::Llm;

/// Confidential TEE model on NEAR AI's own fleet, by canonical id.
pub const MODEL: &str = "z-ai/glm-5.3-flash";
/// GLM-5.3 Flash reasons by default and reasoning counts toward the reply
/// limit; "low" keeps answers complete (docs.near.ai/cloud/reasoning-models).
pub(crate) const EFFORT: &str = "low";
pub(crate) const MAX_TOKENS: u64 = 8192;

/// An attested, end-to-end encrypted connection, opened on first use.
pub struct Confidential {
    connection: OnceCell<(net::NearAi, String)>,
}

fn fail(stage: &str) -> impl Fn(anyhow::Error) -> Error + '_ {
    move |_| Error(format!("nearai {stage} failed"))
}

impl Confidential {
    /// No key, attestation, or network is needed until the first inference.
    pub fn new() -> Self {
        Self {
            connection: OnceCell::new(),
        }
    }

    fn connection(&self) -> Result<&(net::NearAi, String)> {
        if self.connection.get().is_none() {
            let key = std::env::var("NEARAI_API_KEY")
                .map_err(|_| Error("NEARAI_API_KEY is not set".into()))?;
            let api = net::NearAi::new(key);
            let model_key = api.attest(MODEL).map_err(fail("attestation"))?;
            // Single-threaded, with no callbacks into this cell meanwhile.
            let _ = self.connection.set((api, model_key));
        }
        Ok(self.connection.get().expect("connection initialized"))
    }
}

impl Llm for Confidential {
    fn model(&self) -> &str {
        MODEL
    }

    fn json(&self, system: &str, user: &str) -> Result<Value> {
        let (api, model_key) = self.connection()?;
        let messages = messages(system, user);
        let session = e2ee::Session::new(model_key).map_err(fail("session"))?;
        let reply = api
            .signed_turn(MODEL, &session, &messages, EFFORT, MAX_TOKENS)
            .map_err(fail("turn"))?;
        Ok(parse_reply(&reply))
    }
}

/// The chat messages for one turn.
pub(crate) fn messages(system: &str, user: &str) -> [Value; 2] {
    let system = format!("{system}\n\nReply with one JSON object and nothing else.");
    [
        json!({ "role": "system", "content": system }),
        json!({ "role": "user", "content": user }),
    ]
}

/// Malformed or empty model output follows the synthesis retry/withhold path.
/// Never return a JSON parser error that could quote confidential reply text.
/// The deployed model sometimes wraps its JSON in one Markdown code fence.
pub(crate) fn parse_reply(content: &str) -> Value {
    let content = content.trim();
    let unfenced = content
        .strip_prefix("```json")
        .or_else(|| content.strip_prefix("```"))
        .and_then(|rest| rest.strip_suffix("```"))
        .unwrap_or(content);
    serde_json::from_str(unfenced).unwrap_or(Value::Null)
}

/// Bytes of a hex string, with or without a `0x` prefix.
fn bytes(hex: &str) -> anyhow::Result<Vec<u8>> {
    let hex = hex
        .strip_prefix("0x")
        .or_else(|| hex.strip_prefix("0X"))
        .unwrap_or(hex);
    Ok(hex::decode(hex)?)
}

mod sign {
    //! Response signatures from NEAR AI Cloud: the model's enclave signs
    //! "<model>:<sha256 of the exact request bytes>:<sha256 of the exact response
    //! bytes>" with the key its attestation report bound. Port of `core/sign.mjs`.
    use anyhow::{bail, Result};
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    /// Hex SHA-256 of bytes.
    pub fn sha256(data: &[u8]) -> String {
        hex::encode(Sha256::digest(data))
    }

    pub fn signed_text(model: &str, request_sha256: &str, response_sha256: &str) -> String {
        format!("{model}:{request_sha256}:{response_sha256}")
    }

    pub fn check_signature(signature: &Value, expected_text: &str, signer: &str) -> Result<()> {
        match signature["signature_kind"].as_str() {
            Some("provider_tee") => {}
            kind => bail!(
                "signed by {}, not the model enclave",
                kind.unwrap_or("nobody")
            ),
        }
        if signature["signing_algo"].as_str() != Some("ed25519") {
            bail!("unexpected signing algorithm {}", signature["signing_algo"]);
        }
        if signature["text"].as_str() != Some(expected_text) {
            bail!("signature does not cover the exact request and response");
        }
        if signature["signing_address"].as_str().map(str::to_lowercase)
            != Some(signer.to_lowercase())
        {
            bail!("signed by a key other than the attested one");
        }
        let valid = (|| {
            let key =
                VerifyingKey::from_bytes(&crate::nearai::bytes(signer).ok()?.try_into().ok()?)
                    .ok()?;
            let signature = Signature::from_slice(
                &crate::nearai::bytes(signature["signature"].as_str()?).ok()?,
            )
            .ok()?;
            Some(key.verify(expected_text.as_bytes(), &signature).is_ok())
        })();
        if valid != Some(true) {
            bail!("invalid signature");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inference_failures_never_echo_provider_text_or_decrypted_content() {
        let private = "PRIVATE-RESPONDENT-CONTEXT";
        let error = fail("turn")(anyhow::anyhow!(private));
        assert!(!error.0.contains(private));
        for content in [private, "", "```json\n{} trailing", "```private```"] {
            assert_eq!(parse_reply(content), Value::Null);
        }
        assert_eq!(parse_reply(" {} "), json!({}));
        assert_eq!(parse_reply("```json\n{}\n```\n"), json!({}));
        assert_eq!(parse_reply("```\n{}\n```"), json!({}));
    }
}
