//! NEAR JSON-RPC submission and verification of official-CLI-signed transactions.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::{json, Value};

use crate::Result;

pub struct Rpc {
    pub url: String,
}

impl Rpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let reply: Value = ureq::post(&self.url)
            .timeout(std::time::Duration::from_secs(30))
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

    /// Broadcast exactly the CLI-produced bytes; then query those same bytes.
    /// An ambiguous acknowledgement never causes signing of a replacement.
    pub fn submit_signed(&self, signed: &str) -> Result<Value> {
        if self
            .call(
                "send_tx",
                json!({"signed_tx_base64":signed,"wait_until":"NONE"}),
            )
            .is_err()
        {
            eprintln!("RPC did not acknowledge submission; checking the same signed transaction.");
        }
        self.await_signed(signed)
    }

    pub fn signed_status(&self, signed: &str, wait_until: &str) -> Result<Value> {
        self.call(
            "tx",
            json!({"signed_tx_base64":signed,"wait_until":wait_until}),
        )
    }

    pub fn await_signed(&self, signed: &str) -> Result<Value> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        loop {
            match self.signed_status(signed, "FINAL") {
                Ok(outcome) => return Ok(outcome),
                Err(_) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_secs(5))
                }
                Err(_) => return Err("signed transaction is not final after five minutes".into()),
            }
        }
    }
}

fn merge(target: &mut Value, extra: Value) {
    if let (Some(t), Some(e)) = (target.as_object_mut(), extra.as_object()) {
        t.extend(e.clone());
    }
}

/// Bind the chain result to the exact organizer call we authorized.
pub fn check_call(outcome: &Value, hash: &str, organizer: &str, call: &Value) -> Result<()> {
    let tx = &outcome["transaction"];
    if tx["hash"] != hash
        || tx["signer_id"] != organizer
        || tx["receiver_id"] != call["receiver_id"]
    {
        return Err(
            "organizer transaction hash, signer or receiver does not match the authorized call"
                .into(),
        );
    }
    let actions = tx["actions"]
        .as_array()
        .ok_or("organizer transaction has no actions")?;
    let [action] = actions.as_slice() else {
        return Err("organizer transaction must contain one action".into());
    };
    let function = &action["FunctionCall"];
    if function["method_name"] != call["method_name"]
        || function["gas"].as_u64().map(|g| g.to_string()).as_deref() != call["gas"].as_str()
        || function["deposit"] != call["deposit"]
    {
        return Err("organizer transaction action does not match the authorized call".into());
    }
    let args: Value = serde_json::from_slice(
        &B64.decode(
            function["args"]
                .as_str()
                .ok_or("organizer transaction has no encoded arguments")?,
        )?,
    )?;
    if args != call["args"] {
        return Err("organizer transaction arguments differ from the authorized call".into());
    }
    Ok(())
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
