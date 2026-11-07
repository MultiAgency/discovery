//! Verify a survey's key from the chain before encrypting to it.
//!
//! The `InitSurvey` transaction proves, through OutLayer's contract and its
//! TDX-attested workers, which build produced which key. The key is only
//! safe only if a clean bootstrap proves the project could never have run
//! another build, and the owner was already locked before survey creation.
//! A snapshot of the remaining versions alone cannot prove that history.

use serde_json::{json, Value};

use crate::release::Release;
use crate::rpc::{self, events, success_value, CONTRACT};
use crate::Result;

pub struct Verified {
    pub public_key: String,
    pub definition: discovery_protocol::definition::SurveyDefinition,
}

pub fn survey_key(release: &Release, survey_id: &str, init_tx: &str) -> Result<Verified> {
    eprintln!("Verifying survey creation on-chain...");
    let outcome = rpc::tx(init_tx, &release.organizer)?;
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

    require_locked(release, &block)?;
    Ok(Verified {
        public_key,
        definition,
    })
}

/// Check contract execution provenance before trusting the returned bytes.
pub(crate) fn execution(outcome: &Value, release: &Release) -> Result<(Value, Value, String)> {
    let tx = &outcome["transaction"];
    if tx["signer_id"] != release.organizer.as_str() || tx["receiver_id"] != CONTRACT {
        return Err("transaction is not from the organizer to OutLayer".into());
    }
    let action = &tx["actions"][0]["FunctionCall"];
    if action["method_name"] != "request_execution"
        || tx["actions"].as_array().map_or(0, Vec::len) != 1
    {
        return Err("transaction is not a single request_execution".into());
    }

    let requested = events(outcome, "execution_requested");
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
    let input = execution_input(action, &request, release)?;
    let completed = events(outcome, "execution_completed");
    if completed.len() != 1 || completed[0].0["success"] != true {
        return Err("execution did not complete successfully exactly once".into());
    }
    Ok((input, success_value(outcome)?, block.clone()))
}

/// The signed transaction always carries the complete input. OutLayer omits
/// large inputs from its event and removes their temporary state on completion,
/// so verification must not depend on either copy being available later.
fn execution_input(action: &Value, request: &Value, release: &Release) -> Result<Value> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};

    let args: Value = serde_json::from_slice(
        &B64.decode(
            action["args"]
                .as_str()
                .ok_or("transaction has no arguments")?,
        )?,
    )?;
    let project = &args["source"]["Project"];
    if project["project_id"] != release.project.as_str()
        || project
            .get("version_key")
            .is_some_and(|version| !version.is_null() && version != release.wasm_hash.as_str())
    {
        return Err("transaction arguments do not request the registered project and build".into());
    }
    let input = args["input_data"]
        .as_str()
        .ok_or("transaction arguments have no input_data")?;
    let event_input = request["input_data"].as_str().ok_or("no input_data")?;
    match request.get("input_data_in_state") {
        Some(Value::Bool(true)) if event_input.is_empty() => {}
        None | Some(Value::Bool(false)) if event_input == input => {}
        _ => return Err("execution event input does not match the signed transaction".into()),
    }
    Ok(serde_json::from_str(input)?)
}

/// Fetch the report from the finalized chain transaction, never the HTTPS API.
/// OutLayer's contract authenticates the worker output for the pinned build.
pub fn report(
    release: &Release,
    survey_id: &str,
    finalize_tx: &str,
) -> Result<discovery_protocol::report::Report> {
    eprintln!("Verifying report finalization on-chain...");
    let outcome = rpc::tx(finalize_tx, &release.organizer)?;
    let (input, output, block) = execution(&outcome, release)?;
    let report = report_output(&input, output, survey_id)?;
    require_locked(release, &block)?;
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
fn require_locked(release: &Release, block: &str) -> Result<()> {
    eprintln!("Checking the release build and owner lock...");
    let mut warnings = Vec::new();
    let project = rpc::view(
        block,
        "get_project",
        json!({ "project_id": release.project }),
    )?;
    let versions = rpc::view(
        block,
        "list_versions",
        json!({ "project_id": release.project, "limit": 100 }),
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
    let keys = rpc::query(block, "view_access_key_list", owner)?["keys"]
        .as_array()
        .ok_or("access key query has no key list")?
        .len();
    let account = rpc::query(block, "view_account", owner)?;
    if !owner_is_locked(&account, keys) {
        warnings.push(format!("project owner {owner} was not an initialized account without access keys or contract code at execution"));
    }
    if !warnings.is_empty() {
        return Err(format!("release was not locked: {}", warnings.join("; ")).into());
    }
    crate::bootstrap::verify(release, block, &project)
}

pub(crate) fn owner_is_locked(account: &Value, keys: usize) -> bool {
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
        serde_json::from_value(json!({"project": "owner/discovery", "release": "audited",
            "organizer": "organizer.testnet", "bootstrap": {"owner_seed_block": "",
            "owner_init_tx": "", "project_init_tx": "", "lock_tx": ""}}))
        .unwrap()
    }

    fn output() -> Value {
        json!({ "survey_id": "survey", "title": "t",
            "target": { "repo": "owner/repo", "number": 1 }, "status": "released",
            "body": { "summary": "No responses.", "findings": [], "disagreements": [],
                "open_questions": [], "acceptance_criteria": [] },
            "model": "model" })
    }

    fn outcome() -> Value {
        let input = json!({ "operation": "finalize", "survey_id": "survey" }).to_string();
        let args = json!({
            "source": {"Project": {"project_id": "owner/discovery", "version_key": "audited"}},
            "input_data": input,
        });
        let request = json!({ "sender_id": "organizer.testnet", "project_id": "owner/discovery",
            "code_source": { "WasmUrl": { "hash": "audited" } },
            "input_data": input });
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
            "actions": [{ "FunctionCall": { "method_name": "request_execution", "args": B64.encode(args.to_string()) } }] },
            "receipts_outcome": [{ "block_hash": "block", "outcome": { "executor_id": "outlayer.testnet", "logs": logs } }],
            "status": { "SuccessValue": B64.encode(output().to_string()) } })
    }

    #[test]
    fn large_creation_input_is_verified_from_signed_transaction_arguments() {
        // OutLayer's event limit applies to the complete serialized module
        // input, not just a question's text. Both sides of the boundary work.
        for size in [9_999, 10_000, 20_000] {
            let mut definition = json!({
                "version": 2, "title": "Long survey", "deadline_ns": 1_800_000_000_000_000_000u64,
                "target": {"repo": "owner/repo", "number": 1},
                "questions": [{"id": "q", "prompt": ""}],
                "rules": {"no_direct_quotes": false, "withheld_terms": []},
            });
            let base = json!({"operation": "init_survey", "definition": definition})
                .to_string()
                .len();
            definition["questions"][0]["prompt"] = json!("q".repeat(size - base));
            let input = json!({"operation": "init_survey", "definition": definition});
            assert_eq!(input.to_string().len(), size);
            let mut valid = outcome();
            let mut args: Value = serde_json::from_slice(
                &B64.decode(
                    valid["transaction"]["actions"][0]["FunctionCall"]["args"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
            args["input_data"] = json!(input.to_string());
            valid["transaction"]["actions"][0]["FunctionCall"]["args"] =
                json!(B64.encode(args.to_string()));
            let log = &mut valid["receipts_outcome"][0]["outcome"]["logs"][0];
            let mut event: Value =
                serde_json::from_str(log.as_str().unwrap().strip_prefix("EVENT_JSON:").unwrap())
                    .unwrap();
            let mut request: Value =
                serde_json::from_str(event["data"][0]["request_data"].as_str().unwrap()).unwrap();
            request["input_data"] = json!(if size >= 10_000 {
                String::new()
            } else {
                input.to_string()
            });
            request["input_data_in_state"] = json!(size >= 10_000);
            event["data"][0]["request_data"] = json!(request.to_string());
            *log = json!(format!("EVENT_JSON:{event}"));
            assert_eq!(execution(&valid, &release()).unwrap().0, input);
        }
    }

    #[test]
    fn execution_input_rejects_missing_malformed_or_conflicting_arguments() {
        let valid = outcome();
        let action = &valid["transaction"]["actions"][0]["FunctionCall"];
        let input = json!({"operation": "finalize", "survey_id": "survey"}).to_string();
        let event = json!({"input_data": input});
        let args: Value =
            serde_json::from_slice(&B64.decode(action["args"].as_str().unwrap()).unwrap()).unwrap();
        for bad in [
            Value::Null,
            json!("not base64"),
            json!(B64.encode("not JSON")),
            json!(B64.encode("{}")),
        ] {
            let mut bad_action = action.clone();
            bad_action["args"] = bad;
            assert!(execution_input(&bad_action, &event, &release()).is_err());
        }
        for field in ["project_id", "version_key"] {
            let mut bad_args = args.clone();
            bad_args["source"]["Project"][field] = json!("other");
            let mut bad_action = action.clone();
            bad_action["args"] = json!(B64.encode(bad_args.to_string()));
            assert!(execution_input(&bad_action, &event, &release()).is_err());
        }
        for bad_event in [
            json!({"input_data": "different"}),
            json!({"input_data": "", "input_data_in_state": false}),
            json!({"input_data": input, "input_data_in_state": true}),
            json!({"input_data": input, "input_data_in_state": "true"}),
        ] {
            assert!(execution_input(action, &bad_event, &release()).is_err());
        }
        // Unpinned source versions remain safe only because execution() also
        // authenticates the actual resolved build in OutLayer's event.
        let mut unpinned = args;
        unpinned["source"]["Project"]
            .as_object_mut()
            .unwrap()
            .remove("version_key");
        let mut unpinned_action = action.clone();
        unpinned_action["args"] = json!(B64.encode(unpinned.to_string()));
        assert!(execution_input(&unpinned_action, &event, &release()).is_ok());
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
