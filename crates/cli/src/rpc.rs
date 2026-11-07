//! Read-only NEAR JSON-RPC for release and report verification.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::{json, Value};
use std::io::Write;
use std::time::Duration;

use crate::Result;

/// The OutLayer contract that runs the service.
pub const CONTRACT: &str = "outlayer.testnet";
const RPC: &str = "https://archival-rpc.testnet.fastnear.com";
const ATTEMPTS: u64 = 6;

pub fn call(method: &str, params: Value) -> Result<Value> {
    let body = json!({ "jsonrpc": "2.0", "id": "discovery", "method": method, "params": params });
    // The public archival RPC rate-limits bursts and occasionally drops a
    // request; every call is a read, so back off briefly and retry. An
    // optional FastNEAR key raises the limit; it never changes the endpoint.
    let key = std::env::var("FASTNEAR_API_KEY")
        .ok()
        .filter(|k| !k.is_empty());
    let reply: Value = send_with_retry(
        || {
            let mut request = ureq::post(RPC).timeout(Duration::from_secs(30));
            if let Some(key) = &key {
                request = request.set("Authorization", &format!("Bearer {key}"));
            }
            request.send_json(&body).map_err(Box::new)
        },
        std::thread::sleep,
        &mut std::io::stderr().lock(),
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

/// Only fixed classifications and attempt numbers reach diagnostics, never
/// request arguments, credentials, response bodies or transport error text.
/// Injectable sending and sleeping keep retry coverage entirely offline.
fn send_with_retry(
    mut send: impl FnMut() -> std::result::Result<ureq::Response, Box<ureq::Error>>,
    mut sleep: impl FnMut(Duration),
    progress: &mut impl Write,
) -> Result<ureq::Response> {
    for attempt in 1..=ATTEMPTS {
        let error = match send() {
            Ok(response) => return Ok(response),
            Err(error) => error,
        };
        let reason = match error.as_ref() {
            ureq::Error::Status(429, _) => "rate limited",
            ureq::Error::Status(500..=599, _) => "temporarily unavailable",
            ureq::Error::Transport(_) => "connection failed",
            _ => return Err(error),
        };
        if attempt == ATTEMPTS {
            return match *error {
                ureq::Error::Status(429, _) => Err(
                    "the archival RPC is rate-limiting this machine; wait a minute \
                    and retry, or set FASTNEAR_API_KEY"
                        .into(),
                ),
                error => Err(error.into()),
            };
        }
        let delay = Duration::from_secs(5 * attempt);
        // A failed diagnostic write must not change verification behavior.
        let _ = writeln!(
            progress,
            "Archival RPC {reason}; retry {}/{ATTEMPTS} in {}s.",
            attempt + 1,
            delay.as_secs()
        );
        sleep(delay);
    }
    unreachable!("the final attempt always returns")
}

/// Account state at a historical block (archival RPC).
pub fn query(block: &str, request_type: &str, account: &str) -> Result<Value> {
    call(
        "query",
        json!({ "request_type": request_type, "account_id": account, "block_id": block }),
    )
}

/// An OutLayer contract view at a historical block.
pub fn view(block: &str, method: &str, args: Value) -> Result<Value> {
    let result = call(
        "query",
        json!({ "request_type": "call_function", "account_id": CONTRACT,
            "method_name": method, "args_base64": B64.encode(serde_json::to_vec(&args)?),
            "block_id": block }),
    )?;
    let bytes: Vec<u8> = serde_json::from_value(result["result"].clone())?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn tx(hash: &str, sender: &str) -> Result<Value> {
    call(
        "tx",
        json!({ "tx_hash": hash, "sender_account_id": sender, "wait_until": "FINAL" }),
    )
}

/// Decode the final `SuccessValue` of a transaction outcome as JSON.
pub fn success_value(outcome: &Value) -> Result<Value> {
    let encoded = outcome["status"]["SuccessValue"]
        .as_str()
        .ok_or_else(|| format!("transaction did not succeed: {}", outcome["status"]))?;
    Ok(serde_json::from_slice(&B64.decode(encoded)?)?)
}

/// OutLayer `EVENT_JSON` logs with the given event name, with the receipt block hash.
pub fn events(outcome: &Value, event: &str) -> Vec<(Value, String)> {
    let mut out = Vec::new();
    for r in outcome["receipts_outcome"].as_array().into_iter().flatten() {
        if r["outcome"]["executor_id"] != CONTRACT {
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
    use std::collections::VecDeque;

    fn status(code: u16) -> Box<ureq::Error> {
        Box::new(
            ureq::Response::new(code, "SECRET reason", "SECRET response body")
                .unwrap()
                .into(),
        )
    }

    fn transport() -> Box<ureq::Error> {
        Box::new(std::io::Error::new(std::io::ErrorKind::TimedOut, "SECRET transport text").into())
    }

    #[test]
    fn transient_failures_retry_without_exposing_error_text_or_changing_the_reply() {
        let mut replies = VecDeque::from([
            Err(status(429)),
            Err(status(503)),
            Err(transport()),
            Ok(ureq::Response::new(200, "OK", r#"{"result":{"verified":true}}"#).unwrap()),
        ]);
        let mut delays = Vec::new();
        let mut progress = Vec::new();
        let response = send_with_retry(
            || replies.pop_front().expect("unexpected extra request"),
            |delay| delays.push(delay.as_secs()),
            &mut progress,
        )
        .unwrap();
        assert!(replies.is_empty());
        assert_eq!(delays, [5, 10, 15]);
        assert_eq!(
            response.into_json::<Value>().unwrap(),
            json!({"result": {"verified": true}})
        );
        let progress = String::from_utf8(progress).unwrap();
        assert!(!progress.contains("SECRET"));
        for expected in [
            "rate limited",
            "temporarily unavailable",
            "connection failed",
        ] {
            assert!(progress.contains(expected));
        }
        assert!(progress.contains("retry 4/6 in 15s"));
    }

    #[test]
    fn exhausted_rate_limits_stop_at_six_attempts_with_actionable_safe_guidance() {
        let mut attempts = 0;
        let mut delays = Vec::new();
        let mut progress = Vec::new();
        let error = send_with_retry(
            || {
                attempts += 1;
                Err(status(429))
            },
            |delay| delays.push(delay.as_secs()),
            &mut progress,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(attempts, 6);
        assert_eq!(delays, [5, 10, 15, 20, 25]);
        assert!(error.contains("FASTNEAR_API_KEY"));
        assert!(!error.contains("SECRET"));
        let progress = String::from_utf8(progress).unwrap();
        assert_eq!(progress.lines().count(), 5);
        assert!(!progress.contains("SECRET") && !progress.contains("7/6"));
    }

    #[test]
    fn nonretryable_status_returns_immediately_without_a_retry_message() {
        let mut attempts = 0;
        let mut progress = Vec::new();
        let error = send_with_retry(
            || {
                attempts += 1;
                Err(status(403))
            },
            |_| panic!("a nonretryable status must not sleep"),
            &mut progress,
        )
        .unwrap_err();
        assert_eq!(attempts, 1);
        assert!(progress.is_empty());
        assert!(matches!(
            error.downcast_ref::<ureq::Error>(),
            Some(ureq::Error::Status(403, _))
        ));
    }

    #[test]
    fn server_and_transport_failures_keep_their_terminal_error_after_six_attempts() {
        for server_error in [true, false] {
            let mut attempts = 0;
            let mut sleeps = 0;
            let mut progress = Vec::new();
            let error = send_with_retry(
                || {
                    attempts += 1;
                    Err(if server_error {
                        status(503)
                    } else {
                        transport()
                    })
                },
                |_| sleeps += 1,
                &mut progress,
            )
            .unwrap_err();
            assert_eq!((attempts, sleeps), (6, 5));
            match error.downcast_ref::<ureq::Error>().unwrap() {
                ureq::Error::Status(503, _) => assert!(server_error),
                ureq::Error::Transport(_) => assert!(!server_error),
                error => panic!("unexpected error: {error}"),
            }
            assert!(!String::from_utf8(progress).unwrap().contains("SECRET"));
        }
    }
}
