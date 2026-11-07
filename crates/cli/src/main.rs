//! `discovery`: prepare, verify, answer and retrieve confidential surveys.
//!
//! One testnet release is pinned: the `releases.json` service compiled into this
//! binary. Announcements cannot override it. Respondents verify keys against the
//! service's creation transaction before encrypting answers.

mod bootstrap;
mod invitation;
mod release;
mod rpc;
mod verify;

use std::fs;
use std::path::Path;

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use clap::{Parser, Subcommand};
use discovery_protocol::definition::{SurveyDefinition, Target};
use discovery_protocol::envelope::{seal, MAX_RESPONSE};
use discovery_protocol::response::Response;
use discovery_protocol::SurveyStatus;
use release::{is_hex, tx_hash, Release};
use serde_json::{json, Value};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The trusted release of the checkout this CLI was built from.
const REGISTRY: &str = include_str!("../../../releases.json");
const API: &str = "https://testnet-api.outlayer.ai";

#[derive(Parser)]
#[command(name = "discovery", about = "Confidential surveys for agents")]
struct Cli {
    /// The organizer's published submission payment key.
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
    ///
    /// Normally your agent drafts the file with
    /// `skills/discovery-respondent/SKILL.md` and shows it to you before
    /// submitting. The format, one entry per answer:
    ///
    /// {"answers": [{"question_id": "<id from verify>",
    ///   "basis": "confirmed" | "observed" | "interpretation",
    ///   "disclosure": {"tier": "publishable"}
    ///              | {"tier": "context_only", "phrasing": "<optional safe wording>"},
    ///   "text": "<your answer>"}]}
    ///
    /// At most 4,092 bytes. Submissions are immutable; an identical retry counts once.
    #[command(verbatim_doc_comment)]
    Submit {
        survey_id: String,
        #[arg(long)]
        init_tx: String,
        answers: std::path::PathBuf,
    },
    /// Print a verified report as Markdown for posting.
    Report {
        survey_id: String,
        #[arg(long)]
        finalize_tx: String,
    },
    /// Organizer: verify an accepting survey and print its invitation page.
    ///
    /// Run through `scripts/invitation.sh`, which checks a clean checkout and
    /// commit reachability from cached canonical origin refs. Requires `--target`.
    Invitation {
        survey_id: String,
        #[arg(long)]
        init_tx: String,
        /// The trusted checkout's commit that respondents install.
        #[arg(long)]
        commit: String,
        /// The https:// URL where the invitation will be published.
        #[arg(long)]
        page: String,
    },
}

#[derive(Subcommand)]
enum Preparation {
    /// Validate a definition for creation on the configured service.
    Init { definition: std::path::PathBuf },
    /// Finalize after the deadline, using the organizer's `nearai` secrets.
    Finalize { survey_id: String },
}

fn main() -> std::process::ExitCode {
    match run(Cli::parse(), REGISTRY) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("discovery: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Each command checks its local inputs before any network or paid call and
/// stops at the first problem. Error messages never echo the inputs.
fn run(cli: Cli, registry: &str) -> Result<()> {
    let Cli {
        payment_key,
        target,
        command,
    } = cli;
    let release = Release::registered(registry)?;
    let target = target.as_deref();
    if let Some(target) = target {
        check_target_format(target)?;
    }
    match command {
        Command::Prepare { operation } => {
            let (survey_id, input) = match operation {
                Preparation::Init { definition } => read_definition(&definition, target)?,
                Preparation::Finalize { survey_id } => {
                    check_survey_id(&survey_id)?;
                    if target.is_some() {
                        return Err(
                            "verify the survey's target before preparing finalization".into()
                        );
                    }
                    let input = json!({"operation": "finalize", "survey_id": survey_id});
                    (survey_id, input)
                }
            };
            eprintln!("survey_id: {survey_id}");
            print(&request(&release, input))
        }
        Command::Verify { survey_id, init_tx } => {
            check_survey_id(&survey_id)?;
            check_tx(&init_tx)?;
            let key = check_payment_key(payment_key.as_deref(), &release.organizer)?;
            let (survey, state, accepting) =
                verified_state(&release, &survey_id, &init_tx, key, target)?;
            if accepting {
                eprintln!("{}", verify_next_hint(&survey_id, &init_tx, target));
            }
            print(&json!({
                "survey_id": survey_id,
                "release": release.wasm_hash,
                "definition": survey.definition,
                "accepting_responses": accepting,
                "closed": state.closed,
                "full": state.full,
                "key_present": state.key_present,
                "report_released": state.report_released,
            }))
        }
        Command::Submit {
            survey_id,
            init_tx,
            answers,
        } => {
            check_survey_id(&survey_id)?;
            check_tx(&init_tx)?;
            let key = check_payment_key(payment_key.as_deref(), &release.organizer)?;
            let response = read_answers(&answers)?;
            let survey = verified(&release, &survey_id, &init_tx, target)?;
            let envelope = seal_answers(&survey, response)?;
            require_accepted(&call(
                &release,
                key,
                json!({ "operation": "submit", "survey_id": survey_id, "envelope": envelope }),
            )?)?;
            println!("Submitted. Responses are immutable; identical retries are counted once.");
            Ok(())
        }
        Command::Report {
            survey_id,
            finalize_tx,
        } => {
            check_survey_id(&survey_id)?;
            check_tx(&finalize_tx)?;
            let report = verify::report(&release, &survey_id, &finalize_tx)?;
            check_target(&report.target, target)?;
            eprintln!("Verified against the release's on-chain finalize output.");
            print!(
                "{}\n---\nReleased by survey `{survey_id}` (finalize tx `{finalize_tx}`, release `{}`). \
                 Verify with this CLI: `discovery report {survey_id} --finalize-tx {finalize_tx}`.\n",
                report.markdown(),
                release.wasm_hash,
            );
            Ok(())
        }
        Command::Invitation {
            survey_id,
            init_tx,
            commit,
            page,
        } => {
            check_survey_id(&survey_id)?;
            check_tx(&init_tx)?;
            if target.is_none() {
                return Err("--target is required: the issue the invitation is for".into());
            }
            invitation::check_details(&commit, &page)?;
            let key = check_payment_key(payment_key.as_deref(), &release.organizer)?;
            invitation::check_payment_key(key)?;
            let (survey, _, accepting) =
                verified_state(&release, &survey_id, &init_tx, key, target)?;
            if !accepting {
                return Err("the survey is not accepting responses; invite to a new survey".into());
            }
            print!(
                "{}",
                invitation::render(&invitation::Invitation {
                    definition: &survey.definition,
                    survey_id: &survey_id,
                    init_tx: &init_tx,
                    release: &release.wasm_hash,
                    payment_key: key,
                    commit: &commit,
                    page: &page,
                })?
            );
            Ok(())
        }
    }
}

/// Keep the issue constraint on the suggested submission, with shell quoting
/// for any unusual target characters accepted by the existing target format.
fn verify_next_hint(survey_id: &str, init_tx: &str, target: Option<&str>) -> String {
    let target = target
        .map(|target| {
            let argument = if target
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-/#".contains(&b))
            {
                target.to_string()
            } else {
                format!("'{}'", target.replace('\'', "'\\''"))
            };
            format!(" --target {argument}")
        })
        .unwrap_or_default();
    format!(
        "Next: have your agent draft answers.json with \
         skills/discovery-respondent/SKILL.md (format: `discovery submit --help`), \
         approve the exact answers, then run `discovery submit {survey_id} \
         --init-tx {init_tx}{target} answers.json`. The deadline is the earliest \
         finalization time; answers count until finalization starts."
    )
}

/// `verify`'s checks: the chain-authenticated survey, the service's state for
/// that same definition, and whether it accepts new responses.
fn verified_state(
    release: &Release,
    survey_id: &str,
    init_tx: &str,
    key: &str,
    expected: Option<&str>,
) -> Result<(verify::Verified, SurveyStatus, bool)> {
    let survey = verified(release, survey_id, init_tx, expected)?;
    eprintln!("Checking current collection state...");
    let state: SurveyStatus = serde_json::from_value(call(
        release,
        key,
        json!({ "operation": "status", "survey_id": survey_id }),
    )?)?;
    if state.definition != survey.definition {
        return Err("OutLayer reports a different definition than the chain".into());
    }
    let accepting = state.key_present && !state.closed && !state.full;
    Ok((survey, state, accepting))
}

fn check_target_format(target: &str) -> Result<()> {
    let valid = target.rsplit_once('#').is_some_and(|(repo, number)| {
        repo.split('/').filter(|p| !p.is_empty()).count() == 2
            && repo.matches('/').count() == 1
            && number.parse::<u64>().is_ok_and(|n| n > 0)
    });
    if !valid {
        return Err("--target must look like OWNER/REPO#NUMBER".into());
    }
    Ok(())
}

fn check_survey_id(id: &str) -> Result<()> {
    if !is_hex(id, 32) {
        return Err("not a survey id (64 hex characters)".into());
    }
    Ok(())
}

fn check_tx(hash: &str) -> Result<()> {
    if !tx_hash(hash) {
        return Err("not a NEAR transaction hash".into());
    }
    Ok(())
}

fn check_payment_key<'a>(key: Option<&'a str>, organizer: &str) -> Result<&'a str> {
    let key = key.ok_or("DISCOVERY_PAYMENT_KEY is not set; it is in the survey announcement")?;
    let parts: Vec<_> = key.splitn(3, ':').collect();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
        return Err("DISCOVERY_PAYMENT_KEY must look like owner:nonce:secret".into());
    }
    // The caller selects OutLayer's storage namespace. A well-formed key for
    // another account cannot access this organizer's survey; refuse it before
    // archival verification or a paid service call, without printing the key.
    if parts[0] != organizer {
        return Err(
            "DISCOVERY_PAYMENT_KEY belongs to a different account than the registered organizer; use the key from this survey's announcement".into(),
        );
    }
    Ok(key)
}

/// The survey id and creation input of a valid definition for `expected`.
fn read_definition(path: &Path, expected: Option<&str>) -> Result<(String, Value)> {
    let bytes = fs::read(path).map_err(|_| "cannot read definition file")?;
    let definition: SurveyDefinition =
        serde_json::from_slice(&bytes).map_err(|_| "invalid survey definition JSON")?;
    definition
        .validate()
        .map_err(|error| safe_definition_error(&error))?;
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

/// Protocol validation can quote organizer-written values, including question
/// IDs. Retain fixed field names that help repair the file, never those values.
fn safe_definition_error(error: &discovery_protocol::Error) -> &'static str {
    match error.0.as_str() {
        message if message.starts_with("unsupported definition version ") => {
            "invalid survey definition: version is unsupported; use the format in the organizer skill"
        }
        "target.number must be a positive issue number" => {
            "invalid survey definition: target.number must be a positive issue number"
        }
        "title is empty" => "invalid survey definition: title must not be blank",
        "target.repo must be owner/name" => {
            "invalid survey definition: target.repo must look like OWNER/REPO"
        }
        "no questions" => "invalid survey definition: questions must contain at least one question",
        "every question needs an id and a prompt" => {
            "invalid survey definition: every questions[] entry needs a nonempty id and prompt"
        }
        message if message.starts_with("duplicate question id ") => {
            "invalid survey definition: questions[].id values must be unique"
        }
        "withheld_terms must not be blank" => {
            "invalid survey definition: rules.withheld_terms must not contain blank terms"
        }
        "title contains a withheld term" => {
            "invalid survey definition: title conflicts with rules.withheld_terms; change the title or restrictions"
        }
        _ => "invalid survey definition; check the format and restrictions in the organizer skill",
    }
}

/// Parse answers and check their shape and size; question ids need the
/// verified definition.
fn read_answers(path: &Path) -> Result<Response> {
    let bytes = fs::read(path).map_err(|_| "cannot read answers file")?;
    let response: Response = serde_json::from_slice(&bytes)
        .map_err(|_| "invalid answers JSON; see `discovery submit --help` for the format")?;
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
        // Allow the collection job to finish and the gateway to return its result.
        .timeout(std::time::Duration::from_secs(120))
        .set("X-Payment-Key", key)
        .send_json(json!({ "input": input, "version_key": release.wasm_hash }))
        .map_err(|error| safe_service_error(&error))?
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

/// Fixed recovery guidance only: HTTP reasons, bodies, URLs and transport
/// diagnostics may contain credentials or response material.
fn safe_service_error(error: &ureq::Error) -> &'static str {
    match error {
        ureq::Error::Status(401, _) => {
            "service authentication failed; check the published payment key or contact the organizer through the private help route"
        }
        ureq::Error::Status(402, _) => {
            "service payment failed; contact the organizer through the private help route before retrying"
        }
        ureq::Error::Status(403, _) => {
            "service access was refused; contact the organizer through the private help route before retrying"
        }
        ureq::Error::Status(429, _) => {
            "service is rate-limiting requests; wait before retrying the identical answers if submitting"
        }
        ureq::Error::Status(408 | 500..=599, _) => {
            "service is temporarily unavailable; retry the identical answers if submitting"
        }
        ureq::Error::Status(_, _) => {
            "service rejected the request; contact the organizer through the private help route"
        }
        ureq::Error::Transport(_) => {
            "service connection failed; check connectivity and retry the identical answers if submitting"
        }
    }
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
        .map_err(|_| "answers do not match the verified survey; check answers[].question_id against `discovery verify`")?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use release::tests::{registry, RELEASE};

    /// The local error `args` stop at, with the payment key `key`. Every
    /// case here fails before any network call.
    fn local_error(args: &[&str], key: Option<&str>) -> String {
        local_error_with(&registry().to_string(), args, key)
    }

    fn local_error_with(registry: &str, args: &[&str], key: Option<&str>) -> String {
        let mut cli = Cli::try_parse_from(args).unwrap();
        cli.payment_key = key.map(String::from);
        run(cli, registry).unwrap_err().to_string()
    }

    fn temp_file(name: &str, contents: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("discovery-{name}-{}.json", std::process::id()));
        fs::write(&path, contents).unwrap();
        path
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
    fn verification_next_hint_preserves_the_optional_issue_constraint() {
        let with_target = verify_next_hint("survey", "creation", Some("owner/repo#42"));
        assert!(with_target.contains(
            "`discovery submit survey --init-tx creation --target owner/repo#42 answers.json`"
        ));
        let without_target = verify_next_hint("survey", "creation", None);
        assert!(
            without_target.contains("`discovery submit survey --init-tx creation answers.json`")
        );
        assert!(!without_target.contains("--target"));

        // The target is copied as one shell argument, not executable syntax.
        let unusual = verify_next_hint("survey", "creation", Some("owner/o'$(ignored)#42"));
        assert!(unusual.contains("--target 'owner/o'\\''$(ignored)#42' answers.json"));
    }

    #[test]
    fn service_failures_give_distinct_safe_recovery_instructions() {
        let mut categories = std::collections::BTreeSet::new();
        for (status, expected) in [
            (401, "authentication failed"),
            (402, "payment failed"),
            (403, "access was refused"),
            (429, "rate-limiting"),
            (503, "temporarily unavailable"),
            (400, "rejected the request"),
        ] {
            let error: ureq::Error =
                ureq::Response::new(status, "SECRET reason", "SECRET response body")
                    .unwrap()
                    .into();
            let message = safe_service_error(&error);
            assert!(message.contains(expected), "{status}: {message}");
            assert!(!message.contains("SECRET"));
            assert!(
                categories.insert(message),
                "different recovery cases were collapsed"
            );
            if matches!(status, 400..=403) {
                assert!(message.contains("private help route"));
            }
        }
        let error: ureq::Error =
            std::io::Error::new(std::io::ErrorKind::TimedOut, "SECRET connection details").into();
        let message = safe_service_error(&error);
        assert!(message.contains("check connectivity"));
        assert!(message.contains("identical answers"));
        assert!(!message.contains("SECRET"));
        assert!(categories.insert(message));
    }

    #[test]
    fn wrong_payment_account_stops_before_reads_verification_and_paid_calls() {
        let survey = "c".repeat(64);
        let tx = "8hT4w1nDgUUcV2Z4ztUn5JAtM4uWr6xkJyvQHbTTvAcK";
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let cases: &[&[&str]] = &[
            &["discovery", "verify", &survey, "--init-tx", tx],
            &[
                "discovery",
                "submit",
                &survey,
                "--init-tx",
                tx,
                "/missing/SECRET-answers.json",
            ],
            &[
                "discovery",
                "--target",
                "owner/repo#1",
                "invitation",
                &survey,
                "--init-tx",
                tx,
                "--commit",
                commit,
                "--page",
                "https://example.invalid/invitation",
            ],
        ];
        for args in cases {
            let message = local_error(args, Some("SECRET-other.testnet:1:SECRET-key"));
            assert!(message.contains("different account"), "{message}");
            assert!(message.contains("announcement"));
            assert!(!message.contains("SECRET"));
        }
        let key = "organizer.testnet:1:secret";
        assert_eq!(
            check_payment_key(Some(key), "organizer.testnet").unwrap(),
            key
        );
        for key in [
            "",
            "organizer.testnet:1",
            "organizer.testnet::secret",
            ":1:secret",
        ] {
            let message = check_payment_key(Some(key), "organizer.testnet")
                .unwrap_err()
                .to_string();
            assert!(message.contains("owner:nonce:secret"));
        }
    }

    #[test]
    fn definition_repair_guidance_names_fields_without_echoing_values() {
        let mut definition = json!({"version": discovery_protocol::definition::DEFINITION_VERSION,
            "title": "Onboarding", "target": {"repo": "owner/repo", "number": 42},
            "deadline_ns": 1900000000000000000u64,
            "questions": [{"id": "SECRET-question", "prompt": "What happened?"},
                          {"id": "SECRET-question", "prompt": "What did you try?"}],
            "rules": {"no_direct_quotes": false, "withheld_terms": []}});
        let file = temp_file("repair", &definition.to_string());
        let error = read_definition(&file, None).unwrap_err().to_string();
        assert!(error.contains("questions[].id"));
        assert!(error.contains("unique"));
        assert!(!error.contains("SECRET"));

        definition["questions"] = json!([]);
        fs::write(&file, definition.to_string()).unwrap();
        let error = read_definition(&file, None).unwrap_err().to_string();
        assert!(error.contains("questions must contain at least one"));

        definition["questions"] = json!([{"id": "workflow", "prompt": "What happened?"}]);
        definition["rules"]["withheld_terms"] = json!(["Onboarding"]);
        fs::write(&file, definition.to_string()).unwrap();
        let error = read_definition(&file, None).unwrap_err().to_string();
        assert!(error.contains("title conflicts with rules.withheld_terms"));
        assert!(!error.contains("Onboarding"));
        fs::remove_file(file).unwrap();
    }

    #[test]
    fn local_errors_stop_before_any_network_call_without_echoing_inputs() {
        let survey = "c".repeat(64);
        let tx = "8hT4w1nDgUUcV2Z4ztUn5JAtM4uWr6xkJyvQHbTTvAcK";
        let key = Some("organizer.testnet:1:secret");
        let answers = temp_file(
            "answers",
            r#"{"answers": [{"question_id": "q", "basis": "observed", "text": "x"}]}"#,
        );
        let file = answers.to_str().unwrap();
        let submit = ["discovery", "submit", &survey, "--init-tx", tx, file];
        let cases: &[(&[&str], Option<&str>, &str)] = &[
            (
                &[
                    "discovery",
                    "--target",
                    "SECRET",
                    "submit",
                    "SECRET",
                    "--init-tx",
                    "SECRET",
                    "/SECRET",
                ],
                None,
                "--target must look like OWNER/REPO#NUMBER",
            ),
            (
                &["discovery", "submit", "SECRET", "--init-tx", tx, file],
                key,
                "not a survey id",
            ),
            (
                &["discovery", "submit", &survey, "--init-tx", "SECRET", file],
                key,
                "not a NEAR transaction hash",
            ),
            (&submit, None, "DISCOVERY_PAYMENT_KEY is not set"),
            (&submit, Some("SECRET"), "owner:nonce:secret"),
            (
                &[
                    "discovery",
                    "submit",
                    &survey,
                    "--init-tx",
                    tx,
                    "/missing/SECRET",
                ],
                key,
                "cannot read answers file",
            ),
            (
                &["discovery", "verify", "not-a-survey", "--init-tx", tx],
                key,
                "not a survey id",
            ),
            (
                &[
                    "discovery",
                    "report",
                    &survey,
                    "--finalize-tx",
                    "0xdeadbeef",
                ],
                None,
                "not a NEAR transaction hash",
            ),
            (
                &["discovery", "prepare", "finalize", "not-a-survey"],
                None,
                "not a survey id",
            ),
        ];
        for (args, key, expected) in cases {
            let error = local_error(args, *key);
            assert!(error.contains(expected), "{args:?}: {error}");
            assert!(!error.contains("SECRET"), "{error}");
        }
        for target in [
            "owner/repo",
            "owner#1",
            "owner/repo#0",
            "a/b/c#1",
            "/repo#1",
        ] {
            let args = [
                "discovery",
                "--target",
                target,
                "report",
                &survey,
                "--finalize-tx",
                tx,
            ];
            assert!(
                local_error(&args, None).contains("OWNER/REPO#NUMBER"),
                "{target}"
            );
        }
        // An unregistered release stops every command first.
        let error = local_error_with(r#"{"service": null}"#, &submit, None);
        assert!(error.contains("does not register a release"), "{error}");
        fs::remove_file(answers).unwrap();
    }

    #[test]
    fn invitation_preflight_requires_target_and_publication_details() {
        let survey = "c".repeat(64);
        let tx = "8hT4w1nDgUUcV2Z4ztUn5JAtM4uWr6xkJyvQHbTTvAcK";
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let page = "https://example.invalid/invitation";
        let error = |target: Option<&str>, commit: &str, page: &str, key: Option<&str>| {
            let mut args = vec!["discovery"];
            if let Some(target) = target {
                args.extend(["--target", target]);
            }
            args.extend(["invitation", &survey, "--init-tx", tx]);
            args.extend(["--commit", commit, "--page", page]);
            local_error(&args, key)
        };
        let target = Some("owner/repo#1");
        for key in [
            "organizer.testnet:1:SECRET$(echo)",
            "organizer.testnet:1:SECRET\nexport",
            "organizer.testnet:1:SECRET'",
        ] {
            let error = error(target, commit, page, Some(key));
            assert!(error.contains("payment key"), "{error}");
            assert!(!error.contains("SECRET"));
        }
        let key = Some("organizer.testnet:1:secret");
        for (target, commit, page, key, expected) in [
            (None, commit, page, key, "--target is required"),
            (target, "SECRET", page, key, "--commit"),
            (target, commit, "SECRET", key, "--page"),
            (
                target,
                commit,
                page,
                None,
                "DISCOVERY_PAYMENT_KEY is not set",
            ),
        ] {
            let error = error(target, commit, page, key);
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("SECRET"));
        }
    }

    #[test]
    fn answers_are_checked_locally_for_shape_and_size() {
        let answers = |contents: &str| {
            let file = temp_file("shape", contents);
            let result = read_answers(&file).map_err(|e| e.to_string());
            fs::remove_file(file).unwrap();
            result
        };
        assert!(answers(
            r#"{"answers": [{"question_id": "q", "basis": "observed", "text": "x"}]}"#
        )
        .is_ok());
        let long = format!(
            r#"{{"answers": [{{"question_id": "q", "basis": "observed", "text": "{}"}}]}}"#,
            "x".repeat(MAX_RESPONSE)
        );
        assert!(answers(&long).unwrap_err().contains("byte limit"));
        for invalid in [
            r#"{"answers": []}"#,
            r#"{"answers": [{"question_id": "q", "basis": "observed", "text": " "}]}"#,
            r#"{"answers": [{"question_id": "q", "basis": "observed", "text": "x",
                "disclosure": {"tier": "context_only", "phrasing": " "}}]}"#,
        ] {
            assert!(answers(invalid)
                .unwrap_err()
                .contains("no blank text or phrasing"));
        }
        assert!(answers("{").unwrap_err().contains("invalid answers JSON"));
    }

    #[test]
    fn preparation_validates_inputs_and_uses_the_registered_release() {
        let release = Release::registered(&registry().to_string()).unwrap();
        let definition = json!({"version": discovery_protocol::definition::DEFINITION_VERSION,
            "title": "Onboarding", "target": {"repo": "another/repo", "number": 42},
            "deadline_ns": 1900000000000000000u64,
            "questions": [{"id": "workflow", "prompt": "What did you try?"}],
            "rules": {"no_direct_quotes": false, "withheld_terms": []}});
        let file = temp_file("prepare", &definition.to_string());
        let (_, input) = read_definition(&file, Some("another/repo#42")).unwrap();
        let creation = request(&release, input);
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
        assert!(read_definition(&file, Some("wrong/repo#42")).is_err());
        let mut past = definition.clone();
        past["deadline_ns"] = json!(1_000_000_000_000_000_000u64);
        fs::write(&file, past.to_string()).unwrap();
        assert!(read_definition(&file, None)
            .unwrap_err()
            .to_string()
            .contains("has passed"));
        fs::write(&file, "{}").unwrap();
        assert!(read_definition(&file, None).is_err());
        fs::remove_file(file).unwrap();

        let survey_id = "c".repeat(64);
        let finalization = request(
            &release,
            json!({"operation": "finalize", "survey_id": survey_id}),
        );
        assert_eq!(finalization["source"], creation["source"]);
        assert_eq!(
            finalization["resource_limits"]["max_execution_seconds"],
            180
        );
        assert_eq!(
            finalization["secrets_ref"],
            json!({"profile": "nearai", "account_id": "organizer.testnet"})
        );
        assert!(local_error(
            &[
                "discovery",
                "--target",
                "another/repo#42",
                "prepare",
                "finalize",
                &survey_id
            ],
            None
        )
        .contains("verify the survey's target"));
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
