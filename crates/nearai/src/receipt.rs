// Derived from MultiAgency/private-ai@dd469a5 app/src/receipt.rs. Local changes: VENDORED.md.
//! Version-2 inference evidence retained alongside a released survey report.
//! Contains attestation evidence and signed turn hashes, never response content.
//! The product CLI checks on-chain report provenance, not these receipts.
use serde_json::{json, Value};

use crate::sign::sha256;

pub const RECEIPT_VERSION: u64 = 2;

/// What a live attestation leaves for the receipt.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Evidence {
    pub nonce: String,
    pub report: Value,
    pub gpu_token: String,
    /// GPU verdicts and signing keys for all candidates, in preflight order.
    #[serde(default)]
    pub model_gpu_tokens: Vec<String>,
    #[serde(default)]
    pub model_gpu_keys: Vec<Value>,
    /// The key that signed NVIDIA's verdict, with its certificate chain: NVIDIA
    /// lists only current keys, so the receipt keeps the one it needs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_key: Option<Value>,
    pub allow_unpatched_model: bool,
}

/// Commit to the released survey/body and retain the inference evidence and
/// the OutLayer execution context. No plaintext request or response is stored.
pub fn receipt_v2(
    subject_sha256: &str,
    model: &str,
    evidence: &Evidence,
    turns: &[Value],
    created_at: &str,
    outlayer: &Value,
) -> (String, String) {
    let mut receipt = serde_json::Map::new();
    receipt.insert("version".into(), json!(RECEIPT_VERSION));
    receipt.insert("subject_sha256".into(), json!(subject_sha256));
    receipt.insert("model".into(), json!(model));
    receipt.insert("created_at".into(), json!(created_at));
    receipt.insert("nonce".into(), json!(evidence.nonce));
    receipt.insert("attestation".into(), evidence.report.clone());
    receipt.insert("gpu_token".into(), json!(evidence.gpu_token));
    receipt.insert("model_gpu_tokens".into(), json!(evidence.model_gpu_tokens));
    receipt.insert("model_gpu_keys".into(), json!(evidence.model_gpu_keys));
    if let Some(key) = &evidence.gpu_key {
        receipt.insert("gpu_key".into(), key.clone());
    }
    receipt.insert(
        "policy".into(),
        json!({ "allow_unpatched_model": evidence.allow_unpatched_model }),
    );
    receipt.insert("turns".into(), json!(turns));
    receipt.insert("outlayer".into(), outlayer.clone());
    let text = serde_json::to_string_pretty(&receipt).expect("receipt serializes");
    let hash = sha256(text.as_bytes());
    (text, hash)
}

/// The commitment a receipt makes to its survey and released report body.
pub fn subject_commitment(salt: &str, subject: &Value) -> String {
    sha256(format!("{salt}:{}", crate::canonical(subject)).as_bytes())
}
