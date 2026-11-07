//! `discovery`: prepare, verify, answer and retrieve confidential surveys.
//!
//! One testnet release is pinned: the `releases.json` service compiled into this
//! binary. Announcements cannot override it. Respondents verify keys against the
//! service's creation transaction before encrypting answers.

mod verify;

use std::fs;
use std::path::PathBuf;

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use clap::{Parser, Subcommand};
use discovery_protocol::definition::{SurveyDefinition, Target};
use discovery_protocol::envelope::{seal, MAX_RESPONSE};
use discovery_protocol::response::Response;
use discovery_protocol::SurveyStatus;
use serde::Deserialize;
use serde_json::{json, Value};
use verify::Release;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The trusted release of the checkout this CLI was built from.
const REGISTRY: &str = include_str!("../../../releases.json");
const API: &str = "https://testnet-api.outlayer.ai";

#[derive(Parser)]
#[command(name = "discovery", about = "Confidential surveys for agents")]
struct Cli {
    /// The organizer's published, restricted submission payment key.
    #[arg(
        long,
        global = true,
        env = "DISCOVERY_PAYMENT_KEY",
        hide_env_values = true
    )]
    payment_key: Option<String>,
    /// Refuse a survey or report for a different GitHub issue.
    #[arg(long, global = true, value_name = "OWNER/REPO#NUMBER")]
    target: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print unsigned OutLayer arguments to sign with the official NEAR CLI.
    Prepare {
        #[command(subcommand)]
        operation: Preparation,
    },
    /// Verify a survey on-chain and show its definition and state.
    Verify {
        survey_id: String,
        #[arg(long)]
        init_tx: String,
    },
    /// Verify, encrypt and submit `answers.json`.
    Submit {
        survey_id: String,
        #[arg(long)]
        init_tx: String,
        answers: PathBuf,
    },
    /// Print a verified report as Markdown for posting.
    Report {
        survey_id: String,
        #[arg(long)]
        finalize_tx: String,
    },
}

#[derive(Subcommand)]
enum Preparation {
    /// Validate a definition for creation on the configured service.
    Init { definition: PathBuf },
    /// Finalize after the deadline, using the organizer's `nearai` secrets.
    Finalize { survey_id: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    service: Option<Release>,
}

/// Without a registered service, live use is refused. The entry needs no
/// further checks here: verification against the chain fails closed on a
/// wrong project, build or organizer, or an owner that still holds keys.
fn configured_service(registry: &str) -> Result<Release> {
    Ok(serde_json::from_str::<Registry>(registry)?
        .service
        .ok_or("this build registers no release (releases.json has no registered service)")?)
}

fn is_hex(s: &str, bytes: usize) -> bool {
    s.len() == 2 * bytes && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A command whose local inputs have all been read and validated.
enum Checked<'a> {
    /// Unsigned arguments to print: the survey id and the module input.
    Prepare(String, Value),
    Verify(&'a str, &'a str, &'a str),
    Submit(&'a str, &'a str, &'a str, Response),
    Report(&'a str, &'a str),
}

/// Every check that needs no network, reported together before any RPC or
/// paid call. Error messages never echo the inputs.
fn check<'a>(cli: &'a Cli, registry: &str) -> Result<(Release, Checked<'a>)> {
    let mut errors = Vec::new();
    let e = &mut errors;
    let release = keep(e, configured_service(registry));
    if let Some(target) = &cli.target {
        let valid = target.rsplit_once('#').is_some_and(|(repo, number)| {
            repo.split('/').filter(|p| !p.is_empty()).count() == 2
                && repo.matches('/').count() == 1
                && number.parse::<u64>().is_ok_and(|n| n > 0)
        });
        if !valid {
            e.push("--target must look like OWNER/REPO#NUMBER".into());
        }
    }
    let checked = match &cli.command {
        Command::Prepare {
            operation: Preparation::Init { definition },
        } => keep(e, read_definition(definition, cli.target.as_deref()))
            .map(|(id, input)| Checked::Prepare(id, input)),
        Command::Prepare {
            operation: Preparation::Finalize { survey_id },
        } => {
            survey(e, survey_id);
            if cli.target.is_some() {
                e.push("verify the survey's target before preparing finalization".into());
            }
            let input = json!({"operation": "finalize", "survey_id": survey_id});
            Some(Checked::Prepare(survey_id.clone(), input))
        }
        Command::Verify { survey_id, init_tx } => {
            survey(e, survey_id);
            tx(e, init_tx);
            payment_key(e, cli).map(|key| Checked::Verify(survey_id, init_tx, key))
        }
        Command::Submit {
            survey_id,
            init_tx,
            answers,
        } => {
            survey(e, survey_id);
            tx(e, init_tx);
            let key = payment_key(e, cli);
            let response = keep(e, read_answers(answers));
            key.zip(response)
                .map(|(key, response)| Checked::Submit(survey_id, init_tx, key, response))
        }
        Command::Report {
            survey_id,
            finalize_tx,
        } => {
            survey(e, survey_id);
            tx(e, finalize_tx);
            Some(Checked::Report(survey_id, finalize_tx))
        }
    };
    match (release, checked) {
        (Some(release), Some(checked)) if errors.is_empty() => Ok((release, checked)),
        _ => Err(format!("local configuration errors:\n- {}", errors.join("\n- ")).into()),
    }
}

fn keep<T>(errors: &mut Vec<String>, result: Result<T>) -> Option<T> {
    result.map_err(|e| errors.push(e.to_string())).ok()
}

fn survey(errors: &mut Vec<String>, id: &str) {
    if !is_hex(id, 32) {
        errors.push("not a survey id (64 hex characters)".into());
    }
}

fn tx(errors: &mut Vec<String>, hash: &str) {
    const BASE58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    if !(32..=44).contains(&hash.len()) || !hash.bytes().all(|b| BASE58.contains(&b)) {
        errors.push("not a NEAR transaction hash".into());
    }
}

fn payment_key<'a>(errors: &mut Vec<String>, cli: &'a Cli) -> Option<&'a str> {
    match cli.payment_key.as_deref() {
        Some(key) if key.splitn(3, ':').filter(|part| !part.is_empty()).count() == 3 => Some(key),
        Some(_) => {
            errors.push("DISCOVERY_PAYMENT_KEY must look like owner:nonce:secret".into());
            None
        }
        None => {
            errors
                .push("DISCOVERY_PAYMENT_KEY is not set; it is in the survey announcement".into());
            None
        }
    }
}

/// The survey id and creation input of a valid definition for `expected`.
fn read_definition(path: &PathBuf, expected: Option<&str>) -> Result<(String, Value)> {
    let bytes = fs::read(path).map_err(|_| "cannot read definition file")?;
    let definition: SurveyDefinition =
        serde_json::from_slice(&bytes).map_err(|_| "invalid survey definition JSON")?;
    definition
        .validate()
        .map_err(|_| "invalid survey definition")?;
    check_target(&definition.target, expected).map_err(|_| "definition does not match --target")?;
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    if u128::from(definition.deadline_ns) <= now_ns {
        return Err("deadline_ns has passed; creation would fail on-chain".into());
    }
    let survey_id = hex::encode(definition.hash()?);
    Ok((
        survey_id,
        json!({"operation": "init_survey", "definition": definition}),
    ))
}

/// Parse answers and check their shape and size; question ids need the
/// verified definition.
fn read_answers(path: &PathBuf) -> Result<Response> {
    let bytes = fs::read(path).map_err(|_| "cannot read answers file")?;
    let response: Response = serde_json::from_slice(&bytes).map_err(|_| "invalid answers JSON")?;
    response
        .validate_answers()
        .map_err(|_| "answers need at least one answer, and no blank text or phrasing")?;
    let size = serde_json::to_vec(&response)?.len();
    if size > MAX_RESPONSE {
        return Err(format!(
            "{size} bytes serialized, over the {MAX_RESPONSE} byte limit; shorten the answers"
        )
        .into());
    }
    Ok(response)
}

/// Unsigned `request_execution` arguments for the pinned release.
fn request(release: &Release, input: Value) -> Value {
    let finalize = input["operation"] == "finalize";
    let mut request = json!({
        "source": {"Project": {"project_id": release.project, "version_key": release.wasm_hash}},
        "input_data": input.to_string(),
        "resource_limits": {"max_instructions": 50000000000u64,
            "max_memory_mb": 256, "max_execution_seconds": if finalize { 180 } else { 60 }},
        "response_format": "Json",
    });
    if finalize {
        request["secrets_ref"] = json!({"profile": "nearai", "account_id": release.organizer});
    }
    request
}

/// HTTPS call to the module, run as the payment key's owner.
fn call(release: &Release, key: &str, input: Value) -> Result<Value> {
    let reply: Value = ureq::post(&format!("{API}/call/{}", release.project))
        .set("X-Payment-Key", key)
        .send_json(json!({ "input": input, "version_key": release.wasm_hash }))
        .map_err(|_| "service request failed; retry the identical answers if submitting")?
        .into_json()
        .map_err(|_| "invalid service response")?;
    if reply["status"] != "completed" {
        return Err("service call did not complete".into());
    }
    if let Some(error) = reply["output"].get("error") {
        return Err(safe_module_error(error).into());
    }
    Ok(reply["output"].clone())
}

fn verified(
    release: &Release,
    survey_id: &str,
    init_tx: &str,
    expected: Option<&str>,
) -> Result<verify::Verified> {
    let survey = verify::survey_key(release, survey_id, init_tx)?;
    check_target(&survey.definition.target, expected)?;
    Ok(survey)
}

fn check_target(actual: &Target, expected: Option<&str>) -> Result<()> {
    let actual = format!("{}#{}", actual.repo, actual.number);
    if let Some(expected) = expected.filter(|e| !e.eq_ignore_ascii_case(&actual)) {
        return Err(format!("survey targets {actual}, expected {expected}").into());
    }
    Ok(())
}

/// Encrypt only against the definition and key authenticated by the creation transaction.
fn seal_answers(survey: &verify::Verified, response: Response) -> Result<String> {
    let hash = survey.definition.hash()?;
    response
        .validate(&survey.definition)
        .map_err(|_| "answers do not match the verified survey")?;
    let public: [u8; 32] = hex::decode(&survey.public_key)?
        .try_into()
        .map_err(|_| "bad survey key")?;
    Ok(B64.encode(seal(&public, &serde_json::to_vec(&response)?, &hash)?))
}

fn safe_module_error(error: &Value) -> &'static str {
    match error.as_str() {
        Some("survey is closed") => "survey is closed",
        Some("survey is full") => "survey is full",
        _ => "module refused the request",
    }
}

fn require_accepted(output: &Value) -> Result<()> {
    if output == &json!({"accepted": true}) {
        Ok(())
    } else {
        Err("submission acceptance was not confirmed; retry the identical answers".into())
    }
}

fn print(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run(&Cli::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("discovery: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<()> {
    let (release, checked) = check(cli, REGISTRY)?;
    let expected = cli.target.as_deref();
    match checked {
        Checked::Prepare(survey_id, input) => {
            eprintln!("survey_id: {survey_id}");
            print(&request(&release, input))?;
        }
        Checked::Verify(survey_id, init_tx, key) => {
            let survey = verified(&release, survey_id, init_tx, expected)?;
            let state: SurveyStatus = serde_json::from_value(call(
                &release,
                key,
                json!({ "operation": "status", "survey_id": survey_id }),
            )?)?;
            if state.definition != survey.definition {
                return Err("OutLayer reports a different definition than the chain".into());
            }
            print(&json!({
                "survey_id": survey_id,
                "release": release.wasm_hash,
                "definition": survey.definition,
                "accepting_responses": state.key_present && !state.closed && !state.full,
                "closed": state.closed,
                "full": state.full,
                "key_present": state.key_present,
                "report_released": state.report_released,
            }))?;
        }
        Checked::Submit(survey_id, init_tx, key, response) => {
            let envelope =
                seal_answers(&verified(&release, survey_id, init_tx, expected)?, response)?;
            require_accepted(&call(
                &release,
                key,
                json!({ "operation": "submit", "survey_id": survey_id, "envelope": envelope }),
            )?)?;
            println!("Submitted. Responses are immutable; identical retries are counted once.");
        }
        Checked::Report(survey_id, finalize_tx) => {
            let report = verify::report(&release, survey_id, finalize_tx)?;
            check_target(&report.target, expected)?;
            eprintln!("Verified against the release's on-chain finalize output.");
            print!(
                "{}\n---\nReleased by survey `{survey_id}` (finalize tx `{finalize_tx}`, release `{}`). \
                 Verify with this CLI: `discovery report {survey_id} --finalize-tx {finalize_tx}`.\n",
                report.markdown(),
                release.wasm_hash,
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE: &str = "5f1e3c2a9b8d7e6f5f1e3c2a9b8d7e6f5f1e3c2a9b8d7e6f5f1e3c2a9b8d7e6f";

    fn registry(release: &str) -> String {
        json!({"service": {"project": "owner.testnet/discovery", "release": release,
            "organizer": "organizer.testnet"}})
        .to_string()
    }

    /// Whatever this checkout registers must be usable or explicitly absent.
    #[test]
    fn the_compiled_registry_is_absent_or_valid() {
        if let Err(error) = configured_service(REGISTRY) {
            assert_eq!(
                error.to_string(),
                "this build registers no release (releases.json has no registered service)"
            );
        }
    }

    #[test]
    fn registry_entries_parse_strictly() {
        let error = configured_service(r#"{"service": null}"#).err().unwrap();
        assert_eq!(
            error.to_string(),
            "this build registers no release (releases.json has no registered service)"
        );
        for registry in [
            r#"{"service": null, "historical": []}"#,
            r#"{"service": null, "unexpected": true}"#,
        ] {
            let error = configured_service(registry).err().unwrap().to_string();
            assert!(error.contains("unknown field"), "{error}");
        }
        assert_eq!(
            configured_service(&registry(RELEASE)).unwrap().wasm_hash,
            RELEASE
        );
        let mut extra: Value = serde_json::from_str(&registry(RELEASE)).unwrap();
        extra["service"]["network"] = json!("mainnet");
        assert!(configured_service(&extra.to_string()).is_err());
    }

    #[test]
    fn submission_requires_exact_acceptance_without_echoing_output() {
        assert!(require_accepted(&json!({"accepted": true})).is_ok());
        for output in [
            Value::Null,
            json!({}),
            json!({"accepted": false}),
            json!({"accepted": "true"}),
            json!({"accepted": true, "private": "SECRET"}),
            json!({"error": "SECRET"}),
        ] {
            let error = require_accepted(&output).unwrap_err().to_string();
            assert!(!error.contains("SECRET"));
            assert!(error.contains("not confirmed"));
            assert_eq!(safe_module_error(&output), "module refused the request");
        }
    }

    #[test]
    fn module_errors_only_reveal_fixed_public_states() {
        for state in ["survey is closed", "survey is full"] {
            assert_eq!(safe_module_error(&json!(state)), state);
        }
        assert_eq!(
            safe_module_error(&json!("SECRET private answer")),
            "module refused the request"
        );
    }

    #[test]
    fn preflight_collects_all_local_errors_without_echoing_inputs() {
        let mut cli = Cli::try_parse_from([
            "discovery",
            "--target",
            "SECRET",
            "submit",
            "SECRET",
            "--init-tx",
            "SECRET",
            "/missing/SECRET",
        ])
        .unwrap();
        cli.payment_key = None;
        let error = check(&cli, r#"{"service": null}"#)
            .err()
            .unwrap()
            .to_string();
        for expected in [
            "this build registers no release",
            "OWNER/REPO#NUMBER",
            "not a survey id",
            "not a NEAR transaction hash",
            "DISCOVERY_PAYMENT_KEY is not set",
            "cannot read answers file",
        ] {
            assert!(error.contains(expected), "{error}");
        }
        assert!(!error.contains("SECRET"));
        assert!(error.find("registers no release") < error.find("DISCOVERY_PAYMENT_KEY"));
    }

    #[test]
    fn local_input_errors_come_before_any_network_call() {
        let answers =
            std::env::temp_dir().join(format!("discovery-answers-{}.json", std::process::id()));
        let survey = "c".repeat(64);
        let tx = "8hT4w1nDgUUcV2Z4ztUn5JAtM4uWr6xkJyvQHbTTvAcK";
        let run = |args: &[&str], key: Option<&str>| {
            let mut cli = Cli::try_parse_from(args).unwrap();
            cli.payment_key = key.map(String::from);
            check(&cli, &registry(RELEASE))
                .map(|_| ())
                .map_err(|e| e.to_string())
        };
        fs::write(
            &answers,
            r#"{"answers": [{"question_id": "q", "basis": "observed", "text": "x"}]}"#,
        )
        .unwrap();
        let file = answers.to_str().unwrap();
        let key = Some("organizer.testnet:1:secret");
        assert!(run(
            &["discovery", "submit", &survey, "--init-tx", tx, file],
            key
        )
        .is_ok());
        assert!(run(
            &["discovery", "submit", &survey, "--init-tx", tx, file],
            None
        )
        .unwrap_err()
        .contains("DISCOVERY_PAYMENT_KEY is not set"));
        assert!(run(
            &["discovery", "verify", &survey, "--init-tx", tx],
            Some("secret")
        )
        .unwrap_err()
        .contains("owner:nonce:secret"));
        assert!(run(
            &["discovery", "verify", "not-a-survey", "--init-tx", tx],
            key
        )
        .unwrap_err()
        .contains("not a survey id"));
        assert!(run(
            &[
                "discovery",
                "report",
                &survey,
                "--finalize-tx",
                "0xdeadbeef"
            ],
            None
        )
        .unwrap_err()
        .contains("not a NEAR transaction hash"));
        assert!(run(&["discovery", "prepare", "finalize", "not-a-survey"], None).is_err());
        for target in [
            "owner/repo",
            "owner#1",
            "owner/repo#0",
            "a/b/c#1",
            "/repo#1",
        ] {
            assert!(
                run(
                    &[
                        "discovery",
                        "--target",
                        target,
                        "report",
                        &survey,
                        "--finalize-tx",
                        tx
                    ],
                    None
                )
                .unwrap_err()
                .contains("OWNER/REPO#NUMBER"),
                "{target}"
            );
        }
        assert!(run(
            &[
                "discovery",
                "--target",
                "owner/repo#7",
                "report",
                &survey,
                "--finalize-tx",
                tx
            ],
            None
        )
        .is_ok());
        let long = format!(
            r#"{{"answers": [{{"question_id": "q", "basis": "observed", "text": "{}"}}]}}"#,
            "x".repeat(discovery_protocol::envelope::MAX_RESPONSE)
        );
        fs::write(&answers, long).unwrap();
        assert!(run(
            &["discovery", "submit", &survey, "--init-tx", tx, file],
            key
        )
        .unwrap_err()
        .contains("byte limit"));
        for invalid in [
            r#"{"answers": []}"#,
            r#"{"answers": [{"question_id": "q", "basis": "observed", "text": " "}]}"#,
            r#"{"answers": [{"question_id": "q", "basis": "observed", "text": "x",
                "disclosure": {"tier": "context_only", "phrasing": " "}}]}"#,
        ] {
            fs::write(&answers, invalid).unwrap();
            assert!(run(
                &["discovery", "submit", &survey, "--init-tx", tx, file],
                key
            )
            .unwrap_err()
            .contains("no blank text or phrasing"));
        }
        fs::write(&answers, "{").unwrap();
        assert!(run(
            &["discovery", "submit", &survey, "--init-tx", tx, file],
            key
        )
        .is_err());
        fs::remove_file(answers).unwrap();
    }

    #[test]
    fn preparation_validates_inputs_and_uses_the_registered_release() {
        let file =
            std::env::temp_dir().join(format!("discovery-prepare-{}.json", std::process::id()));
        let path = file.to_str().unwrap();
        let prepare = |args: &[&str]| -> Result<Value> {
            let cli = Cli::try_parse_from(args).unwrap();
            match check(&cli, &registry(RELEASE))? {
                (release, Checked::Prepare(_, input)) => Ok(request(&release, input)),
                _ => unreachable!(),
            }
        };
        let definition = json!({"version": discovery_protocol::definition::DEFINITION_VERSION,
            "title": "Onboarding", "target": {"repo": "another/repo", "number": 42},
            "deadline_ns": 1900000000000000000u64,
            "questions": [{"id": "workflow", "prompt": "What did you try?"}],
            "rules": {"no_direct_quotes": false, "withheld_terms": []}});
        fs::write(&file, definition.to_string()).unwrap();
        let init =
            |target: &str| prepare(&["discovery", "--target", target, "prepare", "init", path]);
        let creation = init("another/repo#42").unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(creation["input_data"].as_str().unwrap()).unwrap(),
            json!({"operation": "init_survey", "definition": definition})
        );
        assert_eq!(
            creation["source"]["Project"],
            json!({"project_id": "owner.testnet/discovery", "version_key": RELEASE})
        );
        assert_eq!(creation["resource_limits"]["max_execution_seconds"], 60);
        assert!(creation.get("secrets_ref").is_none());
        assert!(init("wrong/repo#42").is_err());
        let mut past = definition.clone();
        past["deadline_ns"] = json!(1_000_000_000_000_000_000u64);
        fs::write(&file, past.to_string()).unwrap();
        assert!(prepare(&["discovery", "prepare", "init", path])
            .unwrap_err()
            .to_string()
            .contains("has passed"));
        fs::write(&file, "{}").unwrap();
        assert!(prepare(&["discovery", "prepare", "init", path]).is_err());
        fs::remove_file(file).unwrap();

        let survey_id = "c".repeat(64);
        let finalization = prepare(&["discovery", "prepare", "finalize", &survey_id]).unwrap();
        assert_eq!(finalization["source"], creation["source"]);
        assert_eq!(
            serde_json::from_str::<Value>(finalization["input_data"].as_str().unwrap()).unwrap(),
            json!({"operation": "finalize", "survey_id": survey_id})
        );
        assert_eq!(
            finalization["resource_limits"]["max_execution_seconds"],
            180
        );
        assert_eq!(
            finalization["secrets_ref"],
            json!({"profile": "nearai", "account_id": "organizer.testnet"})
        );
        assert!(prepare(&[
            "discovery",
            "--target",
            "another/repo#42",
            "prepare",
            "finalize",
            &survey_id
        ])
        .is_err());
    }

    #[test]
    fn submission_uses_only_the_verified_definition_and_key() {
        use discovery_protocol::definition::{DisclosureRules, Question, DEFINITION_VERSION};
        use discovery_protocol::envelope::SurveyKey;
        use discovery_protocol::response::{Answer, Basis, Disclosure};
        let key = SurveyKey::generate().unwrap();
        let mut survey = verify::Verified {
            public_key: hex::encode(key.public()),
            definition: SurveyDefinition {
                version: DEFINITION_VERSION,
                title: "Setup".into(),
                target: Target {
                    repo: "another/repo".into(),
                    number: 42,
                },
                deadline_ns: 1900000000000000000,
                questions: vec![Question {
                    id: "workflow".into(),
                    prompt: "What happened?".into(),
                }],
                rules: DisclosureRules::default(),
            },
        };
        let response = Response {
            answers: vec![Answer {
                question_id: "workflow".into(),
                basis: Basis::Observed,
                disclosure: Disclosure::Publishable,
                text: "Setup failed".into(),
            }],
        };
        let ciphertext = B64
            .decode(seal_answers(&survey, response.clone()).unwrap())
            .unwrap();
        let hash = survey.definition.hash().unwrap();
        assert_eq!(
            serde_json::from_slice::<Response>(&key.open(&ciphertext, &hash).unwrap()).unwrap(),
            response
        );
        survey.definition.questions[0].id = "different".into();
        assert!(seal_answers(&survey, response).is_err());
        assert!(key
            .open(&ciphertext, &survey.definition.hash().unwrap())
            .is_err());
    }

    #[test]
    fn target_checks_distinguish_repositories_and_issue_numbers() {
        for repo in ["MultiAgency/discovery", "another-org/another-project"] {
            let target = Target {
                repo: repo.into(),
                number: 42,
            };
            assert!(check_target(&target, Some(&format!("{repo}#42"))).is_ok());
            assert!(check_target(&target, Some(&format!("{}#42", repo.to_uppercase()))).is_ok());
            assert!(check_target(&target, Some(&format!("{repo}#43"))).is_err());
            assert!(check_target(&target, Some("unrelated/repo#42")).is_err());
            assert!(check_target(&target, None).is_ok());
        }
    }

    #[test]
    fn no_option_overrides_the_pinned_release() {
        for removed in ["--release", "--rpc", "--network", "--project", "--contract"] {
            assert!(Cli::try_parse_from([
                "discovery",
                removed,
                "arbitrary",
                "report",
                "s",
                "--finalize-tx",
                "t"
            ])
            .is_err());
        }
        let error = Cli::try_parse_from(["discovery", "report", "survey"])
            .err()
            .unwrap();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
        assert!(Cli::try_parse_from(["discovery", "report", "s", "--finalize-tx", "t"]).is_ok());
    }
}
