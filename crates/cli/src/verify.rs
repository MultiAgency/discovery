//! Verify a survey's key from the chain before encrypting to it.
//!
//! The `InitSurvey` transaction proves, through OutLayer's contract and its
//! TDX-attested workers, which build produced which key. The key is only
//! safe if, at that block, the release was the project's only version and
//! its owner had no access keys: otherwise another version could have read
//! the key before the project was locked (D5).

use serde_json::{json, Value};

use crate::near::{events, success_value, Rpc};
use crate::Result;

/// What a respondent pins: the audited release and the organizer account.
pub struct Release {
    pub contract: String,
    pub project: String,
    pub wasm_hash: String,
    pub organizer: String,
}

pub struct Verified {
    pub public_key: String,
    /// Lock checks that failed (only allowed with `--allow-unlocked`).
    pub warnings: Vec<String>,
}

pub fn survey_key(
    rpc: &Rpc,
    release: &Release,
    survey_id: &str,
    init_tx: &str,
    allow_unlocked: bool,
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

    let warnings = lock_warnings(rpc, release, &block)?;
    if !warnings.is_empty() && !allow_unlocked {
        return Err(format!(
            "survey key is not protected by a locked release: {}",
            warnings.join("; ")
        )
        .into());
    }
    Ok(Verified {
        public_key,
        warnings,
    })
}

/// Check contract execution provenance before trusting the returned bytes.
fn execution(outcome: &Value, release: &Release) -> Result<(Value, Value, String)> {
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
    let warnings = lock_warnings(rpc, release, &block)?;
    if !warnings.is_empty() {
        return Err(format!(
            "report is not from a locked release: {}",
            warnings.join("; ")
        )
        .into());
    }
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
    let report: discovery_protocol::report::Report = serde_json::from_value(output)?;
    if report.survey_id != survey_id {
        return Err("finalize output is for a different survey".into());
    }
    Ok(report)
}

fn lock_warnings(rpc: &Rpc, release: &Release, block: &str) -> Result<Vec<String>> {
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
    if keys != 0 {
        warnings.push(format!("the project owner {owner} still had {keys} access key(s) at execution: it could add versions"));
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD as B64, Engine};

    fn release() -> Release {
        Release {
            contract: "outlayer.testnet".into(),
            project: "owner/discovery".into(),
            wasm_hash: "audited".into(),
            organizer: "organizer.testnet".into(),
        }
    }

    fn output() -> Value {
        json!({ "survey_id": "survey", "kind": "issue_survey", "title": "t",
            "target": { "repo": "owner/repo", "number": 1 }, "status": "released",
            "counts": { "responses": 0, "merged": 0, "anchored": 0 },
            "body": { "summary": "No responses.", "findings": [], "disagreements": [],
                "open_questions": [], "acceptance_criteria": [], "suggested_tasks": [] },
            "model": "model", "limits": [] })
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
        assert!(report_output(
            &json!({ "operation": "finalize", "survey_id": "survey" }),
            json!({"error":"failed"}),
            "survey"
        )
        .is_err());
    }
}
