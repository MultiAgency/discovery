//! NEAR AI Cloud, verified: attestation (Intel TDX quotes, NVIDIA GPU
//! verdicts), end-to-end encryption to the attested model key (E2EE v2),
//! signed-response checks, and receipts anyone can re-check.
//!
//! `attest`, `e2ee`, `sign`, `receipt` and `net` are derived from
//! MultiAgency/private-ai@dd469a5 (`app/src`). Local security patches and
//! receipt extensions are documented in `VENDORED.md`.
//! `net` speaks `wasi:http` and runs inside the OutLayer enclave.

pub mod attest;
pub mod e2ee;
pub mod net;
pub mod receipt;
pub mod sign;

/// Bytes of a hex string, with or without a `0x` prefix.
pub fn bytes(hex: &str) -> anyhow::Result<Vec<u8>> {
    let hex = hex
        .strip_prefix("0x")
        .or_else(|| hex.strip_prefix("0X"))
        .unwrap_or(hex);
    Ok(hex::decode(hex)?)
}

pub fn random<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).expect("system randomness");
    out
}

/// A JSON value as OutLayer's worker hashes an output: compact, keys sorted.
pub fn canonical(value: &serde_json::Value) -> String {
    sorted(value).to_string()
}

fn sorted(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|k| (k.clone(), sorted(&map[k])))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}
