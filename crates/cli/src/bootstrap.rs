//! Prove that the release's project has never admitted another build.
//!
//! A current version list cannot reveal removed versions. Instead, the pinned
//! bootstrap derives a new owner's name from a recent block, excludes earlier
//! account lifetimes, creates it with one key, spends that key's next nonce
//! creating the reviewed project, and spends the following nonce deleting the
//! key. Key changes at the creation and lock boundaries exclude hidden
//! same-block transactions and already-authorized, delayed receipts.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::verify::{owner_is_locked, rpc, Release, CONTRACT};
use crate::Result;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bootstrap {
    pub owner_seed_block: String,
    pub owner_init_tx: String,
    pub project_init_tx: String,
    pub lock_tx: String,
}

#[derive(Clone)]
struct Account {
    value: Value,
    keys: Value,
}

#[derive(Clone)]
struct Stage {
    tx: Value,
    header: Value,
    owner: Account,
    changes: Value,
}

#[derive(Clone)]
struct Project {
    value: Value,
    versions: Value,
}

struct Evidence {
    owner_init: Stage,
    owner_history: Vec<(Value, Value)>,
    project_init: Stage,
    lock: Stage,
    before_lock: Account,
    project_created: Project,
    project_locked: Project,
    execution_header: Value,
}

const MAX_OWNER_CREATION_BLOCKS: u64 = 32;

/// Read-only archival verification. Missing evidence always refuses the release.
pub fn verify(release: &Release, execution_block: &str, current_project: &Value) -> Result<()> {
    let proof = release
        .bootstrap
        .as_ref()
        .ok_or("release has no clean-bootstrap proof")?;
    let (owner, _) = project_name(release)?;
    let parent = owner
        .split_once('.')
        .map(|(_, parent)| parent)
        .filter(|parent| !parent.is_empty())
        .ok_or("bootstrap owner must be a named subaccount")?;
    let owner_init = load_stage(rpc::tx(&proof.owner_init_tx, parent)?, owner, owner, true)?;
    let project_init = load_stage(
        rpc::tx(&proof.project_init_tx, owner)?,
        CONTRACT,
        owner,
        false,
    )?;
    let lock = load_stage(rpc::tx(&proof.lock_tx, owner)?, owner, owner, true)?;
    let previous = string(&lock.header["prev_hash"], "lock block has no parent")?;
    let evidence = Evidence {
        owner_history: load_owner_history(owner, &owner_init.header, &proof.owner_seed_block)?,
        before_lock: load_account(previous, owner)?,
        project_created: load_project(release, root(&project_init.tx, CONTRACT)?.1)?,
        project_locked: load_project(release, root(&lock.tx, owner)?.1)?,
        execution_header: header(execution_block)?,
        owner_init,
        project_init,
        lock,
    };
    validate(release, execution_block, current_project, &evidence)
}

fn header(block: &str) -> Result<Value> {
    Ok(rpc::call("block", json!({"block_id": block}))?["header"].clone())
}

/// Keep the archival walk bounded even when the supplied seed is not an ancestor.
fn load_owner_history(owner: &str, creation: &Value, seed: &str) -> Result<Vec<(Value, Value)>> {
    let mut block = creation.clone();
    let mut history = Vec::new();
    for _ in 0..=MAX_OWNER_CREATION_BLOCKS {
        let hash = string(&block["hash"], "owner history block has no hash")?;
        let changes = rpc::call(
            "EXPERIMENTAL_changes",
            json!({"changes_type": "account_changes", "account_ids": [owner], "block_id": hash}),
        )?;
        let reached_seed = hash == seed;
        history.push((block.clone(), changes));
        if reached_seed {
            return Ok(history);
        }
        block = header(string(
            &block["prev_hash"],
            "owner history block has no parent",
        )?)?;
    }
    Err("owner creation is not within 32 blocks of its seed ancestor".into())
}

fn load_account(block: &str, owner: &str) -> Result<Account> {
    Ok(Account {
        value: rpc::query(block, "view_account", owner)?,
        keys: rpc::query(block, "view_access_key_list", owner)?,
    })
}

fn load_stage(tx: Value, receiver: &str, owner: &str, changes: bool) -> Result<Stage> {
    let block = root(&tx, receiver)?.1;
    Ok(Stage {
        header: header(block)?,
        owner: load_account(block, owner)?,
        changes: if changes {
            rpc::call(
                "EXPERIMENTAL_changes",
                json!({
                    "changes_type": "all_access_key_changes",
                    "account_ids": [owner],
                    "block_id": block,
                }),
            )?
        } else {
            Value::Null
        },
        tx,
    })
}

fn load_project(release: &Release, block: &str) -> Result<Project> {
    Ok(Project {
        value: rpc::view(block, "get_project", json!({"project_id": release.project}))?,
        versions: rpc::view(
            block,
            "list_versions",
            json!({"project_id": release.project, "limit": 100}),
        )?,
    })
}

fn validate(
    release: &Release,
    execution_block: &str,
    current_project: &Value,
    e: &Evidence,
) -> Result<()> {
    let proof = release
        .bootstrap
        .as_ref()
        .ok_or("release has no clean-bootstrap proof")?;
    let (owner, name) = project_name(release)?;
    let parent = owner
        .split_once('.')
        .map(|(_, parent)| parent)
        .filter(|parent| !parent.is_empty())
        .ok_or("bootstrap owner must be a named subaccount")?;
    let (initial_receipt, initial_block) = root(&e.owner_init.tx, owner)?;
    let (_, project_block) = root(&e.project_init.tx, CONTRACT)?;
    let (lock_receipt, lock_block) = root(&e.lock.tx, owner)?;
    let heights = [
        stage_height(&e.owner_init, initial_block)?,
        stage_height(&e.project_init, project_block)?,
        stage_height(&e.lock, lock_block)?,
        block_height(&e.execution_header, execution_block)?,
    ];
    if !heights.windows(2).all(|h| h[0] < h[1]) {
        return Err("bootstrap and survey executions must be in strictly ordered blocks".into());
    }
    validate_owner_history(
        owner,
        parent,
        proof,
        initial_receipt,
        initial_block,
        &e.owner_history,
    )?;

    let initial = transaction(&e.owner_init.tx, &proof.owner_init_tx, parent, owner)?;
    let actions = array(&initial["actions"], "owner creation has no actions")?;
    if actions.len() != 3
        || actions[0] != "CreateAccount"
        || !single_variant(&actions[1], "Transfer")
        || !actions[1]["Transfer"]["deposit"].is_string()
        || !single_variant(&actions[2], "AddKey")
        || actions[2]["AddKey"]["access_key"]["permission"] != "FullAccess"
    {
        return Err(
            "owner must be newly created with only a transfer and one full-access key".into(),
        );
    }
    let key = string(
        &actions[2]["AddKey"]["public_key"],
        "owner creation has no key",
    )?;
    let initial_nonce = sole_key(&e.owner_init.owner, key)?;
    let project_nonce = initial_nonce
        .checked_add(1)
        .ok_or("bootstrap nonce overflow")?;
    let lock_nonce = project_nonce
        .checked_add(1)
        .ok_or("bootstrap nonce overflow")?;
    let changes = array(
        &e.owner_init.changes["changes"],
        "owner creation has no key history",
    )?;
    if changes.len() != 1
        || !key_update(&changes[0], owner, key, initial_nonce)
        || !receipt_cause(&changes[0], initial_receipt)
    {
        return Err("owner creation block contains unrelated key changes or transactions".into());
    }

    let project = transaction(&e.project_init.tx, &proof.project_init_tx, owner, CONTRACT)?;
    signer_key_and_nonce(project, key, project_nonce)?;
    let actions = array(&project["actions"], "project creation has no actions")?;
    if actions.len() != 1
        || !single_variant(&actions[0], "FunctionCall")
        || actions[0]["FunctionCall"]["method_name"] != "create_project"
    {
        return Err("bootstrap must directly create exactly one project".into());
    }
    let args: Value = serde_json::from_slice(&B64.decode(string(
        &actions[0]["FunctionCall"]["args"],
        "project creation has no arguments",
    )?)?)?;
    if args["name"] != name
        || !single_variant(&args["source"], "WasmUrl")
        || args["source"]["WasmUrl"]["hash"] != release.wasm_hash
    {
        return Err("bootstrap project source does not match the reviewed release".into());
    }
    if sole_key(&e.project_init.owner, key)? != project_nonce {
        return Err("owner performed another transaction before project creation completed".into());
    }
    project_snapshot(release, current_project, &e.project_created)?;
    project_snapshot(release, current_project, &e.project_locked)?;

    let lock = transaction(&e.lock.tx, &proof.lock_tx, owner, owner)?;
    signer_key_and_nonce(lock, key, lock_nonce)?;
    let actions = array(&lock["actions"], "lock has no actions")?;
    if actions.len() != 1
        || !single_variant(&actions[0], "DeleteKey")
        || actions[0]["DeleteKey"]["public_key"] != key
    {
        return Err("bootstrap lock must delete exactly the owner's original key".into());
    }
    let previous_nonce = sole_key(&e.before_lock, key)?;
    let changes = array(&e.lock.changes["changes"], "lock block has no key history")?;
    let deletion = |c: &Value| {
        c["type"] == "access_key_deletion"
            && c["change"]["account_id"] == owner
            && c["change"]["public_key"] == key
            && receipt_cause(c, lock_receipt)
    };
    let clean_lock = match changes.as_slice() {
        [delete] => previous_nonce == lock_nonce && deletion(delete),
        [update, delete] => {
            previous_nonce == project_nonce
                && key_update(update, owner, key, lock_nonce)
                && update["cause"]["type"] == "transaction_processing"
                && update["cause"]["tx_hash"] == proof.lock_tx
                && deletion(delete)
        }
        _ => false,
    };
    if !clean_lock {
        return Err("lock permits unrelated key changes or pending owner transactions".into());
    }
    let keys = complete_keys(&e.lock.owner.keys)?;
    if !keys.is_empty() || !owner_is_locked(&e.lock.owner.value, 0) {
        return Err("bootstrap did not leave an initialized owner without keys or code".into());
    }
    Ok(())
}

/// A block-derived name cannot have an old lifetime before the seed was known;
/// every subsequent account change is inspected, including intermediate changes
/// from separate receipts in the same block.
fn validate_owner_history(
    owner: &str,
    parent: &str,
    proof: &Bootstrap,
    creation_receipt: &str,
    creation_block: &str,
    history: &[(Value, Value)],
) -> Result<()> {
    let label = hex::encode(discovery_protocol::sha256(
        format!(
            "discovery:bootstrap-owner:v1:{}:{parent}",
            proof.owner_seed_block
        )
        .as_bytes(),
    ));
    if owner != format!("{}.{parent}", &label[..32]) {
        return Err("bootstrap owner name does not match its seed block".into());
    }
    let Some((creation, _)) = history.first() else {
        return Err("bootstrap has no owner account history".into());
    };
    let Some((seed, _)) = history.last() else {
        return Err("bootstrap has no owner seed block".into());
    };
    let creation_height = block_height(creation, creation_block)?;
    let seed_height = block_height(seed, &proof.owner_seed_block)?;
    let distance = creation_height
        .checked_sub(seed_height)
        .filter(|distance| (1..=MAX_OWNER_CREATION_BLOCKS).contains(distance));
    if distance.is_none() || history.len() > MAX_OWNER_CREATION_BLOCKS as usize + 1 {
        return Err("owner creation is not within 32 blocks of its seed ancestor".into());
    }
    for pair in history.windows(2) {
        let current_hash = string(&pair[0].0["hash"], "owner history block has no hash")?;
        let parent_hash = string(&pair[1].0["hash"], "owner history block has no hash")?;
        if pair[0].0["prev_hash"] != parent_hash
            || block_height(&pair[0].0, current_hash)? <= block_height(&pair[1].0, parent_hash)?
        {
            return Err("owner account history does not follow its seed ancestry".into());
        }
    }
    for (i, (block, changes)) in history.iter().enumerate() {
        if changes["block_hash"] != block["hash"] {
            return Err("owner account changes do not match their block".into());
        }
        let changes = array(&changes["changes"], "owner account history is incomplete")?;
        let valid = if i == 0 {
            !changes.is_empty()
                && changes.iter().all(|change| {
                    change["type"] == "account_update"
                        && change["change"]["account_id"] == owner
                        && receipt_cause(change, creation_receipt)
                })
        } else {
            changes.is_empty()
        };
        if !valid {
            return Err(
                "bootstrap owner had another account lifetime or unrelated account changes".into(),
            );
        }
    }
    Ok(())
}

fn project_name(release: &Release) -> Result<(&str, &str)> {
    release
        .project
        .split_once('/')
        .filter(|(owner, name)| !owner.is_empty() && !name.is_empty() && !name.contains('/'))
        .ok_or_else(|| "bootstrap project must be owner/name".into())
}

fn transaction<'a>(
    outcome: &'a Value,
    hash: &str,
    signer: &str,
    receiver: &str,
) -> Result<&'a Value> {
    let tx = &outcome["transaction"];
    if outcome["final_execution_status"] != "FINAL"
        || !outcome["status"]["SuccessValue"].is_string()
        || outcome["transaction_outcome"]["id"] != hash
        || tx["hash"] != hash
        || tx["signer_id"] != signer
        || tx["receiver_id"] != receiver
    {
        return Err(
            "bootstrap transaction has incorrect provenance or did not finish successfully".into(),
        );
    }
    Ok(tx)
}

/// The transaction's own action receipt, rather than a refund or a later callback.
fn root<'a>(outcome: &'a Value, receiver: &str) -> Result<(&'a str, &'a str)> {
    let id = string(
        &outcome["transaction_outcome"]["outcome"]["status"]["SuccessReceiptId"],
        "bootstrap transaction has no root receipt",
    )?;
    let receipts = array(
        &outcome["receipts_outcome"],
        "bootstrap has no receipt outcomes",
    )?;
    let matches: Vec<&Value> = receipts.iter().filter(|r| r["id"] == id).collect();
    let [receipt] = matches.as_slice() else {
        return Err("bootstrap has no unique root receipt outcome".into());
    };
    if receipt["outcome"]["executor_id"] != receiver
        || !receipt["outcome"]["status"]["SuccessValue"].is_string()
    {
        return Err("bootstrap action receipt failed or ran on another account".into());
    }
    Ok((
        id,
        string(&receipt["block_hash"], "bootstrap receipt has no block")?,
    ))
}

fn stage_height(stage: &Stage, block: &str) -> Result<u64> {
    block_height(&stage.header, block)
}

fn block_height(header: &Value, hash: &str) -> Result<u64> {
    if header["hash"] != hash {
        return Err("bootstrap block header does not match the receipt".into());
    }
    header["height"]
        .as_u64()
        .ok_or_else(|| "bootstrap block has no height".into())
}

fn sole_key(account: &Account, key: &str) -> Result<u64> {
    let keys = complete_keys(&account.keys)?;
    let [only] = keys.as_slice() else {
        return Err("bootstrap owner must hold exactly its original key".into());
    };
    if only["public_key"] != key
        || only["access_key"]["permission"] != "FullAccess"
        || !owner_is_locked(&account.value, 0)
    {
        return Err(
            "bootstrap owner has another key, restricted key, code or uninitialized state".into(),
        );
    }
    only["access_key"]["nonce"]
        .as_u64()
        .ok_or_else(|| "bootstrap key has no nonce".into())
}

fn complete_keys(value: &Value) -> Result<&Vec<Value>> {
    if !value["last_key"].is_null() {
        return Err("bootstrap requires the complete access-key list".into());
    }
    array(&value["keys"], "bootstrap owner has no complete key list")
}

fn signer_key_and_nonce(tx: &Value, key: &str, nonce: u64) -> Result<()> {
    if tx["public_key"] != key || tx["nonce"].as_u64() != Some(nonce) {
        return Err(
            "bootstrap transactions do not use consecutive nonces of the original key".into(),
        );
    }
    Ok(())
}

fn project_snapshot(release: &Release, current: &Value, historical: &Project) -> Result<()> {
    let (owner, name) = project_name(release)?;
    let uuid = string(&current["uuid"], "release project has no UUID")?;
    if current["owner"] != owner
        || historical.value["uuid"] != uuid
        || historical.value["owner"] != owner
        || historical.value["name"] != name
        || historical.value["active_version"] != release.wasm_hash
    {
        return Err("bootstrap project identity or active build changed".into());
    }
    let versions = array(
        &historical.versions,
        "bootstrap project has no version list",
    )?;
    if versions.len() != 1 || versions[0]["wasm_hash"] != release.wasm_hash {
        return Err("bootstrap project did not contain only the reviewed build".into());
    }
    Ok(())
}

fn key_update(change: &Value, owner: &str, key: &str, nonce: u64) -> bool {
    change["type"] == "access_key_update"
        && change["change"]["account_id"] == owner
        && change["change"]["public_key"] == key
        && change["change"]["access_key"]["permission"] == "FullAccess"
        && change["change"]["access_key"]["nonce"].as_u64() == Some(nonce)
}

fn receipt_cause(change: &Value, receipt: &str) -> bool {
    change["cause"]["type"] == "receipt_processing" && change["cause"]["receipt_hash"] == receipt
}

fn single_variant(value: &Value, variant: &str) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.len() == 1 && o.get(variant).is_some_and(Value::is_object))
}

fn array<'a>(value: &'a Value, error: &'static str) -> Result<&'a Vec<Value>> {
    value.as_array().ok_or_else(|| error.into())
}

fn string<'a>(value: &'a Value, error: &'static str) -> Result<&'a str> {
    value
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| error.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "93ae2c02ee9215bcdfea98147f4819d9.parent.testnet";
    const KEY: &str = "ed25519:original";
    const N: u64 = 100_000_000;

    fn release() -> Release {
        Release {
            project: format!("{OWNER}/discovery"),
            wasm_hash: "reviewed".into(),
            organizer: "organizer.testnet".into(),
            bootstrap: Some(Bootstrap {
                owner_seed_block: "block-seed".into(),
                owner_init_tx: "create-owner".into(),
                project_init_tx: "create-project".into(),
                lock_tx: "lock".into(),
            }),
        }
    }

    fn account(nonce: Option<u64>) -> Account {
        Account {
            value: json!({"code_hash":"11111111111111111111111111111111"}),
            keys: json!({"keys": nonce.into_iter().map(|n| json!({
                "public_key": KEY, "access_key":{"nonce":n,"permission":"FullAccess"}
            })).collect::<Vec<_>>()}),
        }
    }

    fn update(nonce: u64, cause: Value) -> Value {
        json!({"type":"access_key_update", "cause":cause,
            "change":{"account_id":OWNER,"public_key":KEY,
                "access_key":{"nonce":nonce,"permission":"FullAccess"}}})
    }

    fn receipt(id: &str) -> Value {
        json!({"type":"receipt_processing","receipt_hash":id})
    }

    fn tx(hash: &str, signer: &str, receiver: &str, nonce: u64, actions: Value) -> Value {
        json!({
            "final_execution_status":"FINAL", "status":{"SuccessValue":""},
            "transaction":{"hash":hash,"signer_id":signer,"receiver_id":receiver,
                "public_key":KEY,"nonce":nonce,"actions":actions},
            "transaction_outcome":{"id":hash,"outcome":{"status":{"SuccessReceiptId":format!("receipt-{hash}")}}},
            "receipts_outcome":[{"id":format!("receipt-{hash}"),"block_hash":format!("block-{hash}"),
                "outcome":{"executor_id":receiver,"status":{"SuccessValue":""}}}]
        })
    }

    fn evidence() -> Evidence {
        let project = Project {
            value: json!({"uuid":"project-uuid","owner":OWNER,"name":"discovery","active_version":"reviewed"}),
            versions: json!([{"wasm_hash":"reviewed"}]),
        };
        Evidence {
            owner_init: Stage {
                tx: tx(
                    "create-owner",
                    "parent.testnet",
                    OWNER,
                    9,
                    json!([
                        "CreateAccount", {"Transfer":{"deposit":"1000000000000000000000000"}},
                        {"AddKey":{"public_key":KEY,"access_key":{"nonce":0,"permission":"FullAccess"}}}
                    ]),
                ),
                header: json!({"hash":"block-create-owner","height":101,"prev_hash":"block-before-owner"}),
                owner: account(Some(N)),
                changes: json!({"changes":[update(N,receipt("receipt-create-owner"))]}),
            },
            owner_history: vec![
                (
                    json!({"hash":"block-create-owner","height":101,"prev_hash":"block-before-owner"}),
                    json!({"block_hash":"block-create-owner","changes":[{
                        "type":"account_update", "cause":receipt("receipt-create-owner"),
                        "change":{"account_id":OWNER,"code_hash":"11111111111111111111111111111111"}
                    }]}),
                ),
                (
                    json!({"hash":"block-before-owner","height":100,"prev_hash":"block-seed"}),
                    json!({"block_hash":"block-before-owner","changes":[]}),
                ),
                (
                    json!({"hash":"block-seed","height":99,"prev_hash":"before-seed"}),
                    json!({"block_hash":"block-seed","changes":[]}),
                ),
            ],
            project_init: Stage {
                tx: tx(
                    "create-project",
                    OWNER,
                    CONTRACT,
                    N + 1,
                    json!([{"FunctionCall":{
                        "method_name":"create_project", "args":B64.encode(json!({"name":"discovery",
                            "source":{"WasmUrl":{"url":"https://example.invalid/reviewed.wasm","hash":"reviewed"}}}).to_string())
                    }}]),
                ),
                header: json!({"hash":"block-create-project","height":103}),
                owner: account(Some(N + 1)),
                changes: Value::Null,
            },
            lock: Stage {
                tx: tx(
                    "lock",
                    OWNER,
                    OWNER,
                    N + 2,
                    json!([{"DeleteKey":{"public_key":KEY}}]),
                ),
                header: json!({"hash":"block-lock","height":105,"prev_hash":"block-before-lock"}),
                owner: account(None),
                changes: json!({"changes":[
                    update(N+2,json!({"type":"transaction_processing","tx_hash":"lock"})),
                    {"type":"access_key_deletion", "cause":receipt("receipt-lock"),
                        "change":{"account_id":OWNER,"public_key":KEY}}
                ]}),
            },
            before_lock: account(Some(N + 1)),
            project_created: project.clone(),
            project_locked: project,
            execution_header: json!({"hash":"block-survey","height":107}),
        }
    }

    fn check(e: &Evidence) -> Result<()> {
        validate(&release(), "block-survey", &e.project_created.value, e)
    }

    #[test]
    fn fresh_owner_project_and_lock_with_only_two_owner_transactions_verify() {
        let mut e = evidence();
        check(&e).unwrap();
        // A local lock receipt can execute in a later block than validation.
        e.before_lock = account(Some(N + 2));
        e.lock.changes["changes"].as_array_mut().unwrap().remove(0);
        check(&e).unwrap();
    }

    #[test]
    fn bootstrap_rejects_hidden_transactions_and_key_changes() {
        let mut e = evidence();
        e.owner_init.changes["changes"]
            .as_array_mut()
            .unwrap()
            .push(update(
                N + 1,
                json!({"type":"transaction_processing","tx_hash":"hidden"}),
            ));
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.owner_init.changes["changes"][0]["cause"] = receipt("unrelated-receipt");
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.project_init.tx["transaction"]["nonce"] = json!(N + 3);
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.project_init.owner = account(Some(N + 3));
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.lock.tx["transaction"]["nonce"] = json!(N + 4);
        assert!(check(&e).is_err());
    }

    #[test]
    fn pending_higher_nonce_receipts_are_rejected_even_after_key_deletion() {
        let mut e = evidence();
        e.before_lock = account(Some(N + 3));
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.lock.changes["changes"].as_array_mut().unwrap().insert(
            1,
            update(
                N + 3,
                json!({"type":"transaction_processing","tx_hash":"queued-add-version"}),
            ),
        );
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.lock.changes["changes"][0]["cause"] = receipt("relayed-delegate");
        assert!(check(&e).is_err());
    }

    #[test]
    fn bootstrap_rejects_other_sources_versions_and_project_recreation() {
        let mut e = evidence();
        e.project_locked.value["uuid"] = json!("another-project");
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.project_created.versions = json!([{"wasm_hash":"reviewed"},{"wasm_hash":"malicious"}]);
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.project_locked.value["active_version"] = json!("malicious");
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.project_init.tx["transaction"]["actions"][0]["FunctionCall"]["args"] = json!(B64.encode(
            json!({"name":"discovery","source":{"WasmUrl":{"url":"u","hash":"malicious"}}})
                .to_string()
        ));
        assert!(check(&e).is_err());
    }

    #[test]
    fn bootstrap_rejects_owner_code_extra_keys_and_uninitialized_accounts() {
        for field in [
            "global_contract_hash",
            "global_contract_account_id",
            "state",
            "bootstrap_nonce",
        ] {
            let mut e = evidence();
            e.owner_init.owner.value[field] = json!("unexpected");
            assert!(check(&e).is_err(), "{field}");
        }
        let mut e = evidence();
        e.project_init.owner.value["code_hash"] = json!("owner-contract");
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.owner_init.owner.keys["keys"]
            .as_array_mut()
            .unwrap()
            .push(json!({"public_key":"second-key"}));
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.lock.owner = account(Some(N + 2));
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.owner_init.tx["transaction"]["actions"][2]["AddKey"]["access_key"]["permission"] =
            json!({"GasKeyFullAccess":{"balance":"0","num_nonces":4}});
        assert!(check(&e).is_err());
    }

    #[test]
    fn bootstrap_requires_exact_successful_direct_actions_and_receipt_order() {
        let mut e = evidence();
        e.owner_init.tx["transaction"]["signer_id"] = json!("other-parent.testnet");
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.owner_init.tx["transaction"]["actions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"DeployContract":{"code":""}}));
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.lock.tx["final_execution_status"] = json!("EXECUTED_OPTIMISTIC");
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.project_init.tx["receipts_outcome"][0]["outcome"]["status"] = json!({"Failure":{}});
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.project_init.header["height"] = json!(101);
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.execution_header["height"] = json!(105);
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.lock.tx["transaction_outcome"]["outcome"]["status"]["SuccessReceiptId"] = json!("refund");
        assert!(check(&e).is_err());
    }

    #[test]
    fn missing_or_partial_key_history_is_never_a_clean_bootstrap() {
        let mut e = evidence();
        e.owner_init.changes = Value::Null;
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.lock.changes = json!({"changes": []});
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.owner_init.owner.keys["last_key"] = json!("another-page");
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.lock.owner.keys["last_key"] = json!("hidden-key");
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.owner_init.changes["changes"]
            .as_array_mut()
            .unwrap()
            .extend([
                update(N, receipt("temporary-key-install")),
                json!({"type":"access_key_deletion", "cause":receipt("temporary-key-removal"),
                "change":{"account_id":OWNER,"public_key":"another-key"}}),
            ]);
        assert!(check(&e).is_err());
    }

    #[test]
    fn owner_seed_requires_the_derived_name_and_complete_ancestor_history() {
        let mut e = evidence();
        check(&e).unwrap();
        // Heights may skip, but every actual ancestor must remain present.
        e.owner_history[2].0["height"] = json!(98);
        check(&e).unwrap();
        e.owner_history[2].0["height"] = json!(69);
        check(&e).unwrap();
        e.owner_history[2].0["height"] = json!(68);
        assert!(check(&e).is_err());

        let mut wrong_seed = release();
        wrong_seed.bootstrap.as_mut().unwrap().owner_seed_block = "other-seed".into();
        let e = evidence();
        assert!(validate(&wrong_seed, "block-survey", &e.project_created.value, &e).is_err());

        for missing in 0..3 {
            let mut e = evidence();
            e.owner_history.remove(missing);
            assert!(check(&e).is_err(), "missing ancestor {missing}");
        }
        let mut e = evidence();
        e.owner_history.clear();
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.owner_history[1].0["height"] = json!(101);
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.owner_history[1].0["height"] = Value::Null;
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.owner_history[1].1["block_hash"] = json!("another-block");
        assert!(check(&e).is_err());
        let mut e = evidence();
        e.owner_history[1].1["changes"] = Value::Null;
        assert!(check(&e).is_err());
    }

    #[test]
    fn owner_seed_rejects_prior_lifetimes_even_when_recreated_in_one_block() {
        let account_update = |cause| {
            json!({
                "type":"account_update", "cause":receipt(cause), "change":{"account_id":OWNER}
            })
        };
        let account_deletion = json!({"type":"account_deletion",
            "cause":receipt("old-account-deletion"), "change":{"account_id":OWNER}});
        // The seed and all following ancestors are checked, not just their
        // final account snapshots. A prior incarnation can leave delayed self
        // receipts even though its account was deleted before the pinned init.
        for earlier_block in [1, 2] {
            let mut e = evidence();
            e.owner_history[earlier_block].1["changes"] =
                json!([account_update("old-account-creation"), account_deletion]);
            assert!(
                check(&e).is_err(),
                "prior lifetime at ancestor {earlier_block}"
            );
        }
        let mut e = evidence();
        e.owner_history[0].1["changes"] = json!([
            account_update("old-account-creation"),
            account_deletion,
            account_update("receipt-create-owner")
        ]);
        assert!(check(&e).is_err());
    }

    #[test]
    fn owner_creation_history_allows_only_updates_from_the_pinned_receipt() {
        let mut e = evidence();
        let update = e.owner_history[0].1["changes"][0].clone();
        e.owner_history[0].1["changes"]
            .as_array_mut()
            .unwrap()
            .push(update);
        check(&e).unwrap();

        for bad in [
            json!({"type":"account_deletion", "cause":receipt("receipt-create-owner"),
                "change":{"account_id":OWNER}}),
            json!({"type":"account_update", "cause":receipt("hidden-receipt"),
                "change":{"account_id":OWNER}}),
            json!({"type":"account_update", "cause":receipt("receipt-create-owner"),
                "change":{"account_id":"another-owner.testnet"}}),
            json!({"type":"unknown_change", "cause":receipt("receipt-create-owner"),
                "change":{"account_id":OWNER}}),
        ] {
            let mut e = evidence();
            e.owner_history[0].1["changes"]
                .as_array_mut()
                .unwrap()
                .push(bad);
            assert!(check(&e).is_err());
        }
        let mut e = evidence();
        e.owner_history[0].1["changes"] = json!([]);
        assert!(check(&e).is_err());
    }
}
