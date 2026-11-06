//! Confidential inference for `Finalize`, verified from inside the enclave.
//!
//! The first inference call fetches a fresh attestation report and verifies it
//! (Intel TDX quotes for the gateway and model, NVIDIA's signed GPU verdict,
//! all bound to our nonce). Each request is end-to-end encrypted to the
//! attested model key, so NEAR AI's gateway relays only ciphertext, and a reply
//! is used only after the model enclave's signature over the exact request and
//! response bytes checks out.

use std::cell::{OnceCell, RefCell};

use discovery_protocol::{Error, Result};
use serde_json::{json, Value};

use crate::finalize::Llm;
use discovery_nearai::e2ee::Session;
use discovery_nearai::net::NearAi;
use discovery_nearai::receipt::{receipt_v2, subject_commitment, Evidence};

/// Confidential TEE model on NEAR AI's own fleet, by canonical id.
pub const MODEL: &str = "z-ai/glm-5.3-flash";
/// GLM-5.3 Flash reasons by default and reasoning counts toward the reply
/// limit; "low" keeps answers complete (docs.near.ai/cloud/reasoning-models).
const EFFORT: &str = "low";
const MAX_TOKENS: u64 = 8192;

/// An attested, end-to-end encrypted session with the model. Errors are
/// public on-chain output, so they carry no request or reply text.
pub struct Confidential {
    connection: OnceCell<VerifiedSession>,
}

struct VerifiedSession {
    api: NearAi,
    public_key: String,
    evidence: Evidence,
    /// One record per signed turn (ids, hashes, signatures), for the receipt.
    turns: RefCell<Vec<Value>>,
}

fn fail(stage: &str) -> impl Fn(anyhow::Error) -> Error + '_ {
    move |e| Error(format!("nearai {stage}: {e}"))
}

impl Confidential {
    /// No key, attestation, or network is needed until the first inference.
    pub fn new() -> Self {
        Self {
            connection: OnceCell::new(),
        }
    }

    fn session(&self) -> Result<&VerifiedSession> {
        if self.connection.get().is_none() {
            let session = VerifiedSession::connect()?;
            // Single-threaded, with no callbacks into this cell during connect.
            let _ = self.connection.set(session);
        }
        Ok(self.connection.get().expect("session initialized"))
    }
}

impl VerifiedSession {
    fn connect() -> Result<Self> {
        let policy = crate::workload::Policy::release(MODEL)?;
        let key = std::env::var("NEARAI_API_KEY")
            .map_err(|_| Error("NEARAI_API_KEY is not set".into()))?;
        let api = NearAi::new(key);
        let (evidence, attestation) = api.attest(MODEL, false).map_err(fail("attestation"))?;
        policy.check(&attestation)?;
        Ok(Self {
            api,
            public_key: attestation.model.public_key,
            evidence,
            turns: RefCell::default(),
        })
    }
}

impl Llm for Confidential {
    fn model(&self) -> &str {
        MODEL
    }

    fn json(&self, system: &str, user: &str, _name: &str, schema: &Value) -> Result<Value> {
        let connection = self.session()?;
        let system = format!("{system}\n\nReply with one JSON object and nothing else. It must match this JSON schema:\n{schema}");
        let messages = [
            json!({ "role": "system", "content": system }),
            json!({ "role": "user", "content": user }),
        ];
        let session = Session::new(&connection.public_key).map_err(fail("session"))?;
        let turn = connection
            .api
            .signed_turn(
                MODEL,
                &connection.public_key,
                &session,
                &messages,
                EFFORT,
                MAX_TOKENS,
            )
            .map_err(fail("turn"))?;
        connection.turns.borrow_mut().push(turn.record);
        let Some(content) = turn.reply["content"]
            .as_str()
            .filter(|c| !c.trim().is_empty())
        else {
            return Err(Error(format!(
                "nearai reply has no content (finish_reason {:?}, usage {})",
                turn.finish_reason,
                turn.usage.unwrap_or(Value::Null)
            )));
        };
        // Unparseable output is `Null`: synthesis retries it as a schema
        // mismatch, and the audit treats it as no verdict (fail closed).
        Ok(serde_json::from_str(strip_fences(content)).unwrap_or(Value::Null))
    }

    fn receipt(&self, subject: &Value, block_ns: u64) -> Option<(String, String)> {
        let connection = self.connection.get()?;
        let survey = subject["survey_id"].as_str().unwrap_or_default();
        let outlayer = json!({
            "project": outlayer::env::var("OUTLAYER_PROJECT_ID"),
            "request_id": outlayer::env::request_id(),
            "transaction_hash": outlayer::env::transaction_hash(),
            "block_timestamp_ns": block_ns.to_string(),
        });
        let created_at = discovery_protocol::iso_time(
            block_ns / 1_000_000_000,
            ((block_ns / 1_000_000) % 1000) as u32,
        );
        Some(receipt_v2(
            &subject_commitment(survey, subject),
            MODEL,
            &connection.evidence,
            &connection.turns.borrow(),
            &created_at,
            &outlayer,
        ))
    }
}

fn strip_fences(s: &str) -> &str {
    let s = s.trim();
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .unwrap_or(s);
    s.strip_suffix("```").unwrap_or(s).trim()
}
