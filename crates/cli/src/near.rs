//! NEAR JSON-RPC: views (optionally at a past block), transactions, and
//! signing a single FunctionCall transaction with a local ed25519 key.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::Result;

pub struct Rpc {
    pub url: String,
}

impl Rpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let reply: Value = ureq::post(&self.url)
            .send_json(
                json!({ "jsonrpc": "2.0", "id": "discovery", "method": method, "params": params }),
            )?
            .into_json()?;
        if let Some(e) = reply.get("error") {
            return Err(format!("rpc {method}: {e}").into());
        }
        if let Some(e) = reply["result"].get("error") {
            return Err(format!("rpc {method}: {e}").into());
        }
        Ok(reply["result"].clone())
    }

    fn at(block_hash: Option<&str>) -> Value {
        match block_hash {
            Some(h) => json!({ "block_id": h }),
            None => json!({ "finality": "final" }),
        }
    }

    /// Call a view method; `block_hash` reads historical state (archival RPC).
    pub fn view(
        &self,
        contract: &str,
        method: &str,
        args: Value,
        block_hash: Option<&str>,
    ) -> Result<Value> {
        let mut params = json!({
            "request_type": "call_function",
            "account_id": contract,
            "method_name": method,
            "args_base64": B64.encode(serde_json::to_vec(&args)?),
        });
        merge(&mut params, Self::at(block_hash));
        let result = self.call("query", params)?;
        let bytes: Vec<u8> = serde_json::from_value(result["result"].clone())?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn access_key_count(&self, account: &str, block_hash: Option<&str>) -> Result<usize> {
        let mut params = json!({ "request_type": "view_access_key_list", "account_id": account });
        merge(&mut params, Self::at(block_hash));
        let result = self.call("query", params)?;
        Ok(result["keys"]
            .as_array()
            .ok_or("access key query has no key list")?
            .len())
    }

    pub fn tx(&self, hash: &str, sender: &str) -> Result<Value> {
        self.call(
            "tx",
            json!({ "tx_hash": hash, "sender_account_id": sender, "wait_until": "FINAL" }),
        )
    }

    /// Sign and send one FunctionCall; returns the final transaction outcome.
    pub fn function_call(
        &self,
        signer: &Signer,
        receiver: &str,
        method: &str,
        args: &Value,
        gas: u64,
        deposit: u128,
    ) -> Result<Value> {
        let public = signer.key.verifying_key().to_bytes();
        let public_str = format!("ed25519:{}", bs58::encode(public).into_string());
        let key = self.call(
            "query",
            json!({ "request_type": "view_access_key", "finality": "final", "account_id": signer.account, "public_key": public_str }),
        )?;
        let nonce = key["nonce"].as_u64().ok_or("access key has no nonce")? + 1;
        let block: [u8; 32] = bs58::decode(key["block_hash"].as_str().ok_or("no block hash")?)
            .into_vec()?
            .try_into()
            .map_err(|_| "block hash is not 32 bytes")?;
        let tx = transaction(
            &signer.account,
            &public,
            nonce,
            receiver,
            &block,
            method,
            &serde_json::to_vec(args)?,
            gas,
            deposit,
        );
        let digest = Sha256::digest(&tx);
        let hash = bs58::encode(digest).into_string();
        let signature = signer.key.sign(&digest);
        let mut signed = tx;
        signed.push(0); // ed25519
        signed.extend_from_slice(&signature.to_bytes());
        // A long OutLayer run outlasts the RPC's wait (HTTP 408), so submit
        // asynchronously and poll for the final outcome.
        self.call("broadcast_tx_async", json!([B64.encode(signed)]))?;
        self.await_final(&hash, &signer.account)
    }

    fn await_final(&self, hash: &str, sender: &str) -> Result<Value> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        loop {
            match self.tx(hash, sender) {
                Ok(outcome) => return Ok(outcome),
                Err(e) if std::time::Instant::now() < deadline => {
                    eprintln!("waiting for {hash}: {e}");
                    std::thread::sleep(std::time::Duration::from_secs(5));
                }
                Err(e) => {
                    return Err(format!("transaction {hash} not final after 5 minutes: {e}").into())
                }
            }
        }
    }
}

fn merge(target: &mut Value, extra: Value) {
    if let (Some(t), Some(e)) = (target.as_object_mut(), extra.as_object()) {
        t.extend(e.clone());
    }
}

/// An account and its full-access key, from a NEAR CLI legacy keychain file.
pub struct Signer {
    pub account: String,
    key: SigningKey,
}

impl Signer {
    /// The account's key from `DISCOVERY_ORGANIZER_KEY` (CI), or else from
    /// `~/.near-credentials/<network>/<account>.json`, field `private_key`.
    /// Either way the key is `ed25519:<base58 of seed||public>`.
    pub fn load(network: &str, account: &str) -> Result<Self> {
        if let Ok(key) = std::env::var("DISCOVERY_ORGANIZER_KEY") {
            return Self::from_key(account, &key);
        }
        let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
        let path = std::path::PathBuf::from(home)
            .join(".near-credentials")
            .join(network)
            .join(format!("{account}.json"));
        let file: Value = serde_json::from_slice(
            &std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
        )?;
        Self::from_key(
            account,
            file["private_key"]
                .as_str()
                .ok_or("credentials file has no private_key")?,
        )
    }

    fn from_key(account: &str, key: &str) -> Result<Self> {
        let encoded = key
            .trim()
            .strip_prefix("ed25519:")
            .ok_or("private key must be ed25519:<base58>")?;
        let bytes = bs58::decode(encoded).into_vec()?;
        let seed: [u8; 32] = bytes.get(..32).ok_or("private key too short")?.try_into()?;
        let key = SigningKey::from_bytes(&seed);
        if bytes.len() == 64 && bytes[32..] != key.verifying_key().to_bytes() {
            return Err("private key does not match its public half".into());
        }
        Ok(Self {
            account: account.into(),
            key,
        })
    }
}

fn borsh_str(out: &mut Vec<u8>, s: &str) {
    borsh_bytes(out, s.as_bytes());
}

fn borsh_bytes(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(b);
}

/// Borsh `Transaction` (V0) with a single `FunctionCall` action.
#[allow(clippy::too_many_arguments)]
fn transaction(
    signer: &str,
    public: &[u8; 32],
    nonce: u64,
    receiver: &str,
    block: &[u8; 32],
    method: &str,
    args: &[u8],
    gas: u64,
    deposit: u128,
) -> Vec<u8> {
    let mut out = Vec::new();
    borsh_str(&mut out, signer);
    out.push(0); // KeyType::ED25519
    out.extend_from_slice(public);
    out.extend_from_slice(&nonce.to_le_bytes());
    borsh_str(&mut out, receiver);
    out.extend_from_slice(block);
    out.extend_from_slice(&1u32.to_le_bytes()); // one action
    out.push(2); // Action::FunctionCall
    borsh_str(&mut out, method);
    borsh_bytes(&mut out, args);
    out.extend_from_slice(&gas.to_le_bytes());
    out.extend_from_slice(&deposit.to_le_bytes());
    out
}

/// Decode the final `SuccessValue` of a transaction outcome as JSON.
pub fn success_value(outcome: &Value) -> Result<Value> {
    let encoded = outcome["status"]["SuccessValue"]
        .as_str()
        .ok_or_else(|| format!("transaction did not succeed: {}", outcome["status"]))?;
    Ok(serde_json::from_slice(&B64.decode(encoded)?)?)
}

/// `EVENT_JSON` logs with the given event name, with the receipt block hash.
pub fn events(outcome: &Value, contract: &str, event: &str) -> Vec<(Value, String)> {
    let mut out = Vec::new();
    for r in outcome["receipts_outcome"].as_array().into_iter().flatten() {
        if r["outcome"]["executor_id"] != contract {
            continue;
        }
        for log in r["outcome"]["logs"].as_array().into_iter().flatten() {
            let Some(body) = log.as_str().and_then(|l| l.strip_prefix("EVENT_JSON:")) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(body) else {
                continue;
            };
            if v["standard"] == "near-outlayer" && v["event"] == event {
                for d in v["data"].as_array().into_iter().flatten() {
                    out.push((
                        d.clone(),
                        r["block_hash"].as_str().unwrap_or_default().to_string(),
                    ));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_layout() {
        let tx = transaction(
            "a.testnet",
            &[1; 32],
            7,
            "b.testnet",
            &[2; 32],
            "m",
            b"{}",
            3,
            5,
        );
        let mut expected = Vec::new();
        expected.extend_from_slice(&9u32.to_le_bytes());
        expected.extend_from_slice(b"a.testnet");
        expected.push(0);
        expected.extend_from_slice(&[1; 32]);
        expected.extend_from_slice(&7u64.to_le_bytes());
        expected.extend_from_slice(&9u32.to_le_bytes());
        expected.extend_from_slice(b"b.testnet");
        expected.extend_from_slice(&[2; 32]);
        expected.extend_from_slice(&1u32.to_le_bytes());
        expected.push(2);
        expected.extend_from_slice(&1u32.to_le_bytes());
        expected.extend_from_slice(b"m");
        expected.extend_from_slice(&2u32.to_le_bytes());
        expected.extend_from_slice(b"{}");
        expected.extend_from_slice(&3u64.to_le_bytes());
        expected.extend_from_slice(&5u128.to_le_bytes());
        assert_eq!(tx, expected);
    }
}
