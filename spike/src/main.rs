//! Throwaway OutLayer module for verifying platform assumptions on testnet.
//!
//! Operations:
//! - `probe`: report the run's mode, caller, project and block timestamp, and
//!   check that a record sealed in one mode (HTTPS or on-chain) opens in the
//!   other. That settles D5 checks 3 and 4.
//! - `init_keypair`: create a survey signing keypair once with
//!   `set-if-absent`. Later calls return the same public key (D5 check 5).
//! - `verify_nep413`: verify a NEP-413 signature from an implicit account (D6).

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use ed25519_dalek::{Signature, SigningKey, Verifier, VerifyingKey};
use outlayer::{encryption_keys, env, storage::sealed};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

// The manifest lives in the `outlayer.manifest` custom section, so it is
// covered by the wasm's sha256.
#[cfg(target_arch = "wasm32")]
#[used]
#[link_section = "outlayer.manifest"]
static OUTLAYER_MANIFEST: [u8; include_bytes!("../manifest.json").len()] =
    *include_bytes!("../manifest.json");

const KEY: &str = "survey";

/// NEP-413 prefix tag: 2^31 + 413.
const NEP413_TAG: u32 = 2_147_484_061;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum Input {
    Probe,
    InitKeypair { survey_id: String },
    VerifyNep413 {
        account_id: String,
        public_key: String,
        signature: String,
        message: String,
        nonce: String,
        recipient: String,
    },
}

fn var(name: &str) -> Value {
    std::env::var(name).map(Value::String).unwrap_or(Value::Null)
}

fn probe() -> Result<Value, String> {
    let mode = std::env::var("OUTLAYER_EXECUTION_TYPE").unwrap_or_default();
    let tag = encryption_keys::mac(KEY, None, b"probe").map_err(|e| e.0)?;
    let note = json!({ "mode": mode, "block_timestamp": var("NEAR_BLOCK_TIMESTAMP") }).to_string();
    let inserted = sealed::set_if_absent(KEY, None, "probe", note.as_bytes()).map_err(|e| e.0)?;
    let stored = sealed::get(KEY, None, "probe")
        .map_err(|e| e.0)?
        .map(|s| String::from_utf8_lossy(s.plaintext()).into_owned());
    Ok(json!({
        "mode": mode,
        "variant": option_env!("SPIKE_VARIANT").unwrap_or("v1"),
        "signer": env::signer_account_id(),
        "predecessor": env::predecessor_account_id(),
        "project_id": var("OUTLAYER_PROJECT_ID"),
        "project_uuid": var("OUTLAYER_PROJECT_UUID"),
        "block_timestamp": var("NEAR_BLOCK_TIMESTAMP"),
        "block_height": var("NEAR_BLOCK_HEIGHT"),
        "mac_probe": hex::encode(tag),
        "probe_record_inserted_now": inserted,
        "probe_record": stored,
    }))
}

fn init_keypair(survey_id: &str) -> Result<Value, String> {
    let name = format!("keypair:{survey_id}");
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| e.to_string())?;
    let created = sealed::set_if_absent(KEY, None, &name, &seed).map_err(|e| e.0)?;
    let stored = sealed::get(KEY, None, &name)
        .map_err(|e| e.0)?
        .ok_or("keypair missing after set_if_absent")?;
    let secret: [u8; 32] = stored
        .plaintext()
        .try_into()
        .map_err(|_| "stored seed is not 32 bytes")?;
    let public = SigningKey::from_bytes(&secret).verifying_key();
    Ok(json!({ "created": created, "public_key": hex::encode(public.as_bytes()) }))
}

/// Borsh string: u32 little-endian length, then the bytes.
fn borsh_string(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn nep413_hash(message: &str, nonce: &[u8; 32], recipient: &str) -> [u8; 32] {
    let mut payload = Vec::new();
    payload.extend_from_slice(&NEP413_TAG.to_le_bytes());
    borsh_string(&mut payload, message);
    payload.extend_from_slice(nonce);
    borsh_string(&mut payload, recipient);
    payload.push(0); // callback_url: None
    Sha256::digest(&payload).into()
}

fn decode_signature(s: &str) -> Result<Vec<u8>, String> {
    match s.strip_prefix("ed25519:") {
        Some(b58) => bs58::decode(b58).into_vec().map_err(|e| e.to_string()),
        None => B64.decode(s).map_err(|e| e.to_string()),
    }
}

fn verify_nep413(
    account_id: &str,
    public_key: &str,
    signature: &str,
    message: &str,
    nonce: &str,
    recipient: &str,
) -> Result<Value, String> {
    let pk_bytes: [u8; 32] = bs58::decode(public_key.strip_prefix("ed25519:").ok_or("public_key must be ed25519:<base58>")?)
        .into_vec()
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "public key is not 32 bytes")?;
    let nonce: [u8; 32] = B64
        .decode(nonce)
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "nonce is not 32 bytes")?;
    let sig_bytes: [u8; 64] = decode_signature(signature)?
        .try_into()
        .map_err(|_| "signature is not 64 bytes")?;
    let key = VerifyingKey::from_bytes(&pk_bytes).map_err(|e| e.to_string())?;
    let valid = key
        .verify(&nep413_hash(message, &nonce, recipient), &Signature::from_bytes(&sig_bytes))
        .is_ok();
    let implicit_match = account_id == hex::encode(pk_bytes);
    Ok(json!({ "signature_valid": valid, "implicit_account_matches_key": implicit_match }))
}

fn run() -> Result<Value, String> {
    let input: Input = env::input_json()
        .map_err(|e| e.to_string())?
        .ok_or("no input")?;
    match input {
        Input::Probe => probe(),
        Input::InitKeypair { survey_id } => init_keypair(&survey_id),
        Input::VerifyNep413 { account_id, public_key, signature, message, nonce, recipient } => {
            verify_nep413(&account_id, &public_key, &signature, &message, &nonce, &recipient)
        }
    }
}

fn main() {
    let answer = run().unwrap_or_else(|error| json!({ "error": error }));
    env::output_json(&answer).expect("serialize output");
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer;

    #[test]
    fn nep413_round_trip() {
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let nonce = [9u8; 32];
        let sig = key.sign(&nep413_hash("hello", &nonce, "discovery"));
        let pk = format!("ed25519:{}", bs58::encode(key.verifying_key().as_bytes()).into_string());
        let account = hex::encode(key.verifying_key().as_bytes());
        let out = verify_nep413(&account, &pk, &B64.encode(sig.to_bytes()), "hello", &B64.encode(nonce), "discovery").unwrap();
        assert_eq!(out["signature_valid"], true);
        assert_eq!(out["implicit_account_matches_key"], true);
        let bad = verify_nep413(&account, &pk, &B64.encode(sig.to_bytes()), "hellO", &B64.encode(nonce), "discovery").unwrap();
        assert_eq!(bad["signature_valid"], false);
    }
}
