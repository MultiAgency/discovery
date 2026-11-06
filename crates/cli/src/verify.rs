//! Verify a survey's key from the chain before encrypting to it.
//!
//! The `InitSurvey` transaction proves, through OutLayer's contract and its
//! TDX-attested workers, which build produced which key. The key is only
//! safe if, at that block, the release was the project's only version and its
//! owner was initialized, without access keys or contract code: otherwise
//! another version could have read the key before the project was locked.

use serde_json::{json, Value};

use crate::Result;
pub use rpc::Rpc;
use rpc::{events, success_value};

/// A registered release: the audited build and the organizer it runs as.
pub struct Release {
    pub contract: String,
    pub project: String,
    pub wasm_hash: String,
    pub organizer: String,
}

pub struct Verified {
    pub public_key: String,
    pub definition: discovery_protocol::definition::SurveyDefinition,
}

pub fn survey_key(
    rpc: &Rpc,
    release: &Release,
    survey_id: &str,
    init_tx: &str,
) -> Result<Verified> {
    let outcome = rpc.tx(init_tx, &release.organizer)?;
    let (input, output, block) = execution(&outcome, release)?;
    if input["operation"] != "init_survey" {
        return Err("execution was not init_survey".into());
    }
    let definition: discovery_protocol::definition::SurveyDefinition =
        serde_json::from_value(input["definition"].clone())?;
    if hex::encode(definition.hash()?) != survey_id {
        return Err("init definition does not hash to this survey id".into());
    }
    if output["survey_id"] != survey_id {
        return Err("init output is for a different survey".into());
    }
    let public_key = output["public_key"]
        .as_str()
        .ok_or("init output has no public key")?
        .to_string();

    require_locked(rpc, release, &block)?;
    Ok(Verified {
        public_key,
        definition,
    })
}

/// Check contract execution provenance before trusting the returned bytes.
pub(crate) fn execution(outcome: &Value, release: &Release) -> Result<(Value, Value, String)> {
    let tx = &outcome["transaction"];
    if tx["signer_id"] != release.organizer.as_str()
        || tx["receiver_id"] != release.contract.as_str()
    {
        return Err("transaction is not from the organizer to OutLayer".into());
    }
    let action = &tx["actions"][0]["FunctionCall"];
    if action["method_name"] != "request_execution"
        || tx["actions"].as_array().map_or(0, Vec::len) != 1
    {
        return Err("transaction is not a single request_execution".into());
    }

    let requested = events(outcome, &release.contract, "execution_requested");
    let [(event, block)] = requested.as_slice() else {
        return Err("transaction has no single execution_requested event".into());
    };
    let request: Value =
        serde_json::from_str(event["request_data"].as_str().ok_or("no request_data")?)?;
    if request["sender_id"] != release.organizer.as_str() {
        return Err("execution was not requested by the organizer account".into());
    }
    if request["project_id"] != release.project.as_str() {
        return Err(format!(
            "execution ran project {}, not {}",
            request["project_id"], release.project
        )
        .into());
    }
    if request["code_source"]["WasmUrl"]["hash"] != release.wasm_hash.as_str() {
        return Err(format!(
            "execution ran build {}, not the audited release",
            request["code_source"]["WasmUrl"]["hash"]
        )
        .into());
    }
    let input: Value =
        serde_json::from_str(request["input_data"].as_str().ok_or("no input_data")?)?;
    let completed = events(outcome, &release.contract, "execution_completed");
    if completed.len() != 1 || completed[0].0["success"] != true {
        return Err("execution did not complete successfully exactly once".into());
    }
    Ok((input, success_value(outcome)?, block.clone()))
}

/// Fetch the report from the finalized chain transaction, never the HTTPS API.
/// OutLayer's contract authenticates the worker output for the pinned build.
pub fn report(
    rpc: &Rpc,
    release: &Release,
    survey_id: &str,
    finalize_tx: &str,
) -> Result<discovery_protocol::report::Report> {
    let outcome = rpc.tx(finalize_tx, &release.organizer)?;
    let (input, output, block) = execution(&outcome, release)?;
    let report = report_output(&input, output, survey_id)?;
    require_locked(rpc, release, &block)?;
    Ok(report)
}

fn report_output(
    input: &Value,
    output: Value,
    survey_id: &str,
) -> Result<discovery_protocol::report::Report> {
    if input["operation"] != "finalize" || input["survey_id"] != survey_id {
        return Err("transaction did not finalize this survey".into());
    }
    if let Some(error) = output["error"].as_str() {
        return Err(format!(
            "finalization did not complete: {error}; reconcile, then retry finalization"
        )
        .into());
    }
    let report: discovery_protocol::report::Report = serde_json::from_value(output)?;
    if report.survey_id != survey_id {
        return Err("finalize output is for a different survey".into());
    }
    Ok(report)
}

/// At the execution block, the release must have been the project's only
/// version, owned by an initialized account without access keys or code.
fn require_locked(rpc: &Rpc, release: &Release, block: &str) -> Result<()> {
    let mut warnings = Vec::new();
    let project = rpc.view(
        &release.contract,
        "get_project",
        json!({ "project_id": release.project }),
        Some(block),
    )?;
    let versions = rpc.view(
        &release.contract,
        "list_versions",
        json!({ "project_id": release.project, "limit": 100 }),
        Some(block),
    )?;
    let hashes: Vec<&str> = versions
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v["wasm_hash"].as_str())
        .collect();
    if project["active_version"] != release.wasm_hash.as_str() {
        warnings.push("the release was not the active version at execution".into());
    }
    if hashes != [release.wasm_hash.as_str()] {
        warnings.push(format!(
            "the project had {} versions at execution, not only the release",
            hashes.len()
        ));
    }
    let owner = project["owner"].as_str().ok_or("project has no owner")?;
    let keys = rpc.access_key_count(owner, Some(block))?;
    let account = rpc.account(owner, Some(block))?;
    if !owner_is_locked(&account, keys) {
        warnings.push(format!("project owner {owner} was not an initialized account without access keys or contract code at execution"));
    }
    if !warnings.is_empty() {
        return Err(format!("release was not locked: {}", warnings.join("; ")).into());
    }
    Ok(())
}

fn owner_is_locked(account: &Value, keys: usize) -> bool {
    keys == 0
        && account["code_hash"] == "11111111111111111111111111111111"
        && account["global_contract_hash"].is_null()
        && account["global_contract_account_id"].is_null()
        // Initialized accounts omit these fields in the canonical RPC view.
        // An uninitialized universal account can acquire code/keys later.
        && account["state"].is_null()
        && account["bootstrap_nonce"].is_null()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD as B64, Engine};

    #[test]
    fn keyless_owners_with_code_or_pending_initialization_are_not_locked() {
        let empty = json!({"code_hash":"11111111111111111111111111111111"});
        assert!(owner_is_locked(&empty, 0));
        assert!(!owner_is_locked(&empty, 1));
        assert!(!owner_is_locked(&json!({}), 0));
        for (field, value) in [
            ("code_hash", json!("deployed-code")),
            ("global_contract_hash", json!("global-code")),
            ("global_contract_account_id", json!("factory.near")),
            ("state", json!("Uninitialized")),
            ("bootstrap_nonce", json!(1)),
        ] {
            let mut account = empty.clone();
            account[field] = value;
            assert!(!owner_is_locked(&account, 0), "accepted {field}");
        }
    }

    fn release() -> Release {
        Release {
            contract: "outlayer.testnet".into(),
            project: "owner/discovery".into(),
            wasm_hash: "audited".into(),
            organizer: "organizer.testnet".into(),
        }
    }

    fn output() -> Value {
        json!({ "survey_id": "survey", "title": "t",
            "target": { "repo": "owner/repo", "number": 1 }, "status": "released",
            "body": { "summary": "No responses.", "findings": [], "disagreements": [],
                "open_questions": [], "acceptance_criteria": [] },
            "model": "model" })
    }

    fn outcome() -> Value {
        let request = json!({ "sender_id": "organizer.testnet", "project_id": "owner/discovery",
            "code_source": { "WasmUrl": { "hash": "audited" } },
            "input_data": json!({ "operation": "finalize", "survey_id": "survey" }).to_string() });
        let logs = vec![
            format!(
                "EVENT_JSON:{}",
                json!({ "standard": "near-outlayer", "event": "execution_requested", "data": [{ "request_data": request.to_string() }] })
            ),
            format!(
                "EVENT_JSON:{}",
                json!({ "standard": "near-outlayer", "event": "execution_completed", "data": [{ "success": true }] })
            ),
        ];
        json!({ "transaction": { "signer_id": "organizer.testnet", "receiver_id": "outlayer.testnet",
            "actions": [{ "FunctionCall": { "method_name": "request_execution" } }] },
            "receipts_outcome": [{ "block_hash": "block", "outcome": { "executor_id": "outlayer.testnet", "logs": logs } }],
            "status": { "SuccessValue": B64.encode(output().to_string()) } })
    }

    #[test]
    fn report_provenance_rejects_wrong_signer_contract_project_build_and_events() {
        let valid = outcome();
        let (input, output, block) = execution(&valid, &release()).unwrap();
        assert_eq!(block, "block");
        assert_eq!(
            report_output(&input, output, "survey")
                .unwrap()
                .body
                .summary,
            "No responses."
        );
        for field in ["signer_id", "receiver_id"] {
            let mut bad = valid.clone();
            bad["transaction"][field] = json!("attacker");
            assert!(execution(&bad, &release()).is_err());
        }
        for field in ["project", "wasm_hash", "organizer"] {
            let mut pin = release();
            match field {
                "project" => pin.project = "other".into(),
                "wasm_hash" => pin.wasm_hash = "other".into(),
                _ => pin.organizer = "other".into(),
            }
            assert!(execution(&valid, &pin).is_err());
        }
        let mut bad = valid.clone();
        bad["receipts_outcome"][0]["outcome"]["executor_id"] = json!("attacker");
        assert!(execution(&bad, &release()).is_err());
        let mut bad = valid.clone();
        bad["receipts_outcome"][0]["outcome"]["logs"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(execution(&bad, &release()).is_err());
        let mut bad = valid;
        let duplicate = bad["receipts_outcome"][0]["outcome"]["logs"][1].clone();
        bad["receipts_outcome"][0]["outcome"]["logs"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(execution(&bad, &release()).is_err());
    }

    #[test]
    fn report_rejects_wrong_operation_survey_and_output() {
        assert!(report_output(
            &json!({ "operation": "status", "survey_id": "survey" }),
            output(),
            "survey"
        )
        .is_err());
        assert!(report_output(
            &json!({ "operation": "finalize", "survey_id": "other" }),
            output(),
            "survey"
        )
        .is_err());
        assert!(report_output(
            &json!({ "operation": "finalize", "survey_id": "other" }),
            output(),
            "other"
        )
        .is_err());
        let error = report_output(
            &json!({ "operation": "finalize", "survey_id": "survey" }),
            json!({"error":"cleanup interrupted"}),
            "survey",
        )
        .unwrap_err()
        .to_string();
        assert_eq!(error, "finalization did not complete: cleanup interrupted; reconcile, then retry finalization");
    }
}

mod rpc {
    //! Read-only NEAR JSON-RPC for release and report verification.

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
            let mut params =
                json!({ "request_type": "view_access_key_list", "account_id": account });
            merge(&mut params, Self::at(block_hash));
            let result = self.call("query", params)?;
            Ok(result["keys"]
                .as_array()
                .ok_or("access key query has no key list")?
                .len())
        }

        pub fn account(&self, account: &str, block_hash: Option<&str>) -> Result<Value> {
            let mut params = json!({ "request_type": "view_account", "account_id": account });
            merge(&mut params, Self::at(block_hash));
            self.call("query", params)
        }

        pub fn tx(&self, hash: &str, sender: &str) -> Result<Value> {
            self.call(
                "tx",
                json!({ "tx_hash": hash, "sender_account_id": sender, "wait_until": "FINAL" }),
            )
        }
    }

    fn merge(target: &mut Value, extra: Value) {
        if let (Some(t), Some(e)) = (target.as_object_mut(), extra.as_object()) {
            t.extend(e.clone());
        }
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
}
