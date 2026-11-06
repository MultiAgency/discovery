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
use discovery_protocol::envelope::seal;
use discovery_protocol::response::Response;
use discovery_protocol::SurveyStatus;
use serde::Deserialize;
use serde_json::{json, Value};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The trusted release of the checkout this CLI was built from.
const REGISTRY: &str = include_str!("../../../releases.json");
const CONTRACT: &str = "outlayer.testnet";
const API: &str = "https://testnet-api.outlayer.ai";
const RPC: &str = "https://archival-rpc.testnet.near.org";

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

/// The one reviewed testnet release. Its lock is verified on-chain, per block.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    service: Option<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    project: String,
    release: String,
    organizer: String,
}

/// Without a registered service, live use is refused.
fn configured_service(registry: &str) -> Result<Entry> {
    let entry = serde_json::from_str::<Registry>(registry)?
        .service
        .ok_or("releases.json has no registered service")?;
    if !is_hex(&entry.release, 32) || entry.release.to_lowercase() != entry.release {
        return Err("releases.json: release must be the lowercase hex sha256 of the wasm".into());
    }
    match entry.project.split_once('/') {
        Some((owner, _)) if owner == entry.organizer.as_str() => {
            return Err(format!(
                "releases.json: project owner {owner} must not be the organizer; \
                 the owner ends with no access keys"
            )
            .into())
        }
        Some((owner, name)) if is_account(owner) && !name.is_empty() && !name.contains('/') => {}
        _ => return Err("releases.json: project must be <owner account>/<name>".into()),
    }
    if !is_account(&entry.organizer) {
        return Err("releases.json: organizer must be a NEAR account id".into());
    }
    Ok(entry)
}

fn is_hex(s: &str, bytes: usize) -> bool {
    s.len() == 2 * bytes && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// NEAR account id shape: 2–64 chars of `a-z0-9_-` separated by dots.
fn is_account(s: &str) -> bool {
    (2..=64).contains(&s.len())
        && s.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        })
}

/// Input checks that need no network, run before any RPC or paid call.
fn preflight(cli: &Cli) -> Result<()> {
    if let Some(target) = &cli.target {
        let valid = target.rsplit_once('#').is_some_and(|(repo, number)| {
            repo.split('/').filter(|p| !p.is_empty()).count() == 2
                && repo.matches('/').count() == 1
                && number.parse::<u64>().is_ok_and(|n| n > 0)
        });
        if !valid {
            return Err(format!("--target {target} must look like OWNER/REPO#NUMBER").into());
        }
    }
    let survey = |id: &str| -> Result<()> {
        if is_hex(id, 32) {
            Ok(())
        } else {
            Err(format!("{id} is not a survey id (64 hex characters)").into())
        }
    };
    let tx = |hash: &str| -> Result<()> {
        const BASE58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
        if (32..=44).contains(&hash.len()) && hash.bytes().all(|b| BASE58.contains(&b)) {
            Ok(())
        } else {
            Err(format!("{hash} is not a NEAR transaction hash").into())
        }
    };
    let payment_key = || -> Result<()> {
        match cli.payment_key.as_deref() {
            Some(key) if key.splitn(3, ':').count() == 3 => Ok(()),
            Some(_) => Err("DISCOVERY_PAYMENT_KEY must look like owner:nonce:secret".into()),
            None => {
                Err("DISCOVERY_PAYMENT_KEY is not set; it is in the survey announcement".into())
            }
        }
    };
    match &cli.command {
        Command::Prepare { operation } => match operation {
            Preparation::Init { .. } => Ok(()),
            Preparation::Finalize { survey_id } => survey(survey_id),
        },
        Command::Verify { survey_id, init_tx } => {
            survey(survey_id)?;
            tx(init_tx)?;
            payment_key()
        }
        Command::Submit {
            survey_id,
            init_tx,
            answers,
        } => {
            survey(survey_id)?;
            tx(init_tx)?;
            payment_key()?;
            read_answers(answers).map(|_| ())
        }
        Command::Report {
            survey_id,
            finalize_tx,
        } => {
            survey(survey_id)?;
            tx(finalize_tx)
        }
    }
}

/// Parse answers and check their size; question ids need the verified definition.
fn read_answers(path: &PathBuf) -> Result<Response> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let response: Response =
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let size = serde_json::to_vec(&response)?.len();
    if size > discovery_protocol::envelope::MAX_RESPONSE {
        return Err(format!(
            "{}: {size} bytes serialized, over the {} byte limit; shorten the answers",
            path.display(),
            discovery_protocol::envelope::MAX_RESPONSE
        )
        .into());
    }
    Ok(response)
}

struct Service {
    entry: Entry,
    rpc: verify::Rpc,
    payment_key: Option<String>,
}

impl Service {
    fn new(cli: &Cli) -> Result<Self> {
        Ok(Self {
            entry: configured_service(REGISTRY)?,
            rpc: verify::Rpc { url: RPC.into() },
            payment_key: cli.payment_key.clone(),
        })
    }

    fn pinned(&self) -> verify::Release {
        verify::Release {
            contract: CONTRACT.into(),
            project: self.entry.project.clone(),
            wasm_hash: self.entry.release.clone(),
            organizer: self.entry.organizer.clone(),
        }
    }

    /// Unsigned `request_execution` arguments.
    fn prepare(&self, operation: &Preparation, expected: Option<&str>) -> Result<Value> {
        let (input, seconds) = match operation {
            Preparation::Init { definition } => {
                let path = definition;
                let definition: SurveyDefinition = serde_json::from_slice(&fs::read(path)?)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                definition.validate()?;
                check_target(&definition.target, expected)?;
                let now_ns = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos();
                if u128::from(definition.deadline_ns) <= now_ns {
                    return Err("deadline_ns has passed; creation would fail on-chain".into());
                }
                eprintln!("survey_id: {}", hex::encode(definition.hash()?));
                let input = json!({"operation": "init_survey", "definition": definition});
                (input, 60)
            }
            Preparation::Finalize { survey_id } => {
                if expected.is_some() {
                    return Err("verify the survey's target before preparing finalization".into());
                }
                (
                    json!({"operation": "finalize", "survey_id": survey_id}),
                    180,
                )
            }
        };
        let Entry {
            project,
            release,
            organizer,
        } = &self.entry;
        let mut request = json!({
            "source": {"Project": {"project_id": project, "version_key": release}},
            "input_data": input.to_string(),
            "resource_limits": {"max_instructions": 50000000000u64,
                "max_memory_mb": 256, "max_execution_seconds": seconds},
            "response_format": "Json",
        });
        if input["operation"] == "finalize" {
            request["secrets_ref"] = json!({"profile": "nearai", "account_id": organizer});
        }
        Ok(request)
    }

    /// HTTPS call to the module, run as the payment key's owner.
    fn call(&self, input: Value) -> Result<Value> {
        let key = self
            .payment_key
            .as_deref()
            .ok_or("DISCOVERY_PAYMENT_KEY is not set")?;
        let reply: Value = ureq::post(&format!("{}/call/{}", API, self.entry.project))
            .set("X-Payment-Key", key)
            .send_json(json!({ "input": input, "version_key": self.entry.release }))?
            .into_json()?;
        if reply["status"] != "completed" {
            return Err(format!("call {}: {}", reply["status"], reply["error"]).into());
        }
        if let Some(e) = reply["output"].get("error") {
            return Err(format!("module: {e}").into());
        }
        Ok(reply["output"].clone())
    }

    fn verified(
        &self,
        survey_id: &str,
        init_tx: &str,
        expected: Option<&str>,
    ) -> Result<verify::Verified> {
        let survey = verify::survey_key(&self.rpc, &self.pinned(), survey_id, init_tx)?;
        check_target(&survey.definition.target, expected)?;
        Ok(survey)
    }
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
    response.validate(&survey.definition)?;
    let public: [u8; 32] = hex::decode(&survey.public_key)?
        .try_into()
        .map_err(|_| "bad survey key")?;
    Ok(B64.encode(seal(&public, &serde_json::to_vec(&response)?, &hash)?))
}

fn print(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run(Cli::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("discovery: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    preflight(&cli)?;
    let s = Service::new(&cli)?;
    let expected = cli.target.as_deref();
    match &cli.command {
        Command::Prepare { operation } => print(&s.prepare(operation, expected)?)?,
        Command::Verify { survey_id, init_tx } => {
            let survey = s.verified(survey_id, init_tx, expected)?;
            let state: SurveyStatus = serde_json::from_value(
                s.call(json!({ "operation": "status", "survey_id": survey_id }))?,
            )?;
            if state.definition != survey.definition {
                return Err("OutLayer reports a different definition than the chain".into());
            }
            print(&json!({
                "survey_id": survey_id,
                "release": s.entry.release,
                "definition": survey.definition,
                "accepting_responses": state.key_present && !state.closed,
                "closed": state.closed,
                "key_present": state.key_present,
                "report_released": state.report_released,
            }))?;
        }
        Command::Submit {
            survey_id,
            init_tx,
            answers,
        } => {
            let response = read_answers(answers)?;
            let envelope = seal_answers(&s.verified(survey_id, init_tx, expected)?, response)?;
            s.call(json!({ "operation": "submit", "survey_id": survey_id, "envelope": envelope }))?;
            println!("Submitted. Responses are immutable; identical retries are counted once.");
        }
        Command::Report {
            survey_id,
            finalize_tx,
        } => {
            let report = verify::report(&s.rpc, &s.pinned(), survey_id, finalize_tx)?;
            check_target(&report.target, expected)?;
            eprintln!("Verified against the release's on-chain finalize output.");
            print!(
                "{}\n---\nReleased by survey `{survey_id}` (finalize tx `{finalize_tx}`, release `{}`). \
                 Verify with this CLI: `discovery report {survey_id} --finalize-tx {finalize_tx}`.\n",
                report.markdown(),
                s.entry.release,
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE: &str = "5f1e3c2a9b8d7e6f5f1e3c2a9b8d7e6f5f1e3c2a9b8d7e6f5f1e3c2a9b8d7e6f";

    fn service() -> Service {
        Service {
            entry: configured_service(&registry(RELEASE)).unwrap(),
            rpc: verify::Rpc { url: String::new() },
            payment_key: None,
        }
    }

    fn registry(release: &str) -> String {
        json!({"service": {"project": "owner.testnet/discovery", "release": release,
            "organizer": "organizer.testnet"}})
        .to_string()
    }

    /// Whatever this checkout registers must be usable or explicitly absent.
    #[test]
    fn the_compiled_registry_is_absent_or_valid() {
        if let Err(error) = configured_service(REGISTRY) {
            assert_eq!(error.to_string(), "releases.json has no registered service");
        }
    }

    #[test]
    fn registry_entries_are_validated() {
        let error = configured_service(r#"{"service": null}"#).err().unwrap();
        assert_eq!(error.to_string(), "releases.json has no registered service");
        for registry in [
            r#"{"service": null, "historical": []}"#,
            r#"{"service": null, "unexpected": true}"#,
        ] {
            let error = configured_service(registry).err().unwrap().to_string();
            assert!(error.contains("unknown field"), "{error}");
        }
        assert_eq!(
            configured_service(&registry(RELEASE)).unwrap().release,
            RELEASE
        );
        let mut extra: Value = serde_json::from_str(&registry(RELEASE)).unwrap();
        extra["service"]["network"] = json!("mainnet");
        assert!(configured_service(&extra.to_string()).is_err());
        for (field, value) in [
            ("release", "reviewed"),
            ("release", &RELEASE.to_uppercase()),
            ("project", "discovery"),
            ("project", "owner.testnet/a/b"),
            ("project", "Owner.testnet/discovery"),
            ("project", "organizer.testnet/discovery"),
            ("organizer", ""),
        ] {
            let mut entry: Value = serde_json::from_str(&registry(RELEASE)).unwrap();
            entry["service"][field] = json!(value);
            assert!(
                configured_service(&entry.to_string()).is_err(),
                "{field}: {value}"
            );
        }
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
            preflight(&cli).map_err(|e| e.to_string())
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
        let s = service();
        let file =
            std::env::temp_dir().join(format!("discovery-prepare-{}.json", std::process::id()));
        let definition = json!({"version": discovery_protocol::definition::DEFINITION_VERSION,
            "title": "Onboarding", "target": {"repo": "another/repo", "number": 42},
            "deadline_ns": 1900000000000000000u64,
            "questions": [{"id": "workflow", "prompt": "What did you try?"}],
            "rules": {"no_direct_quotes": false, "withheld_terms": []}});
        fs::write(&file, definition.to_string()).unwrap();
        let init = Preparation::Init {
            definition: file.clone(),
        };
        let creation = s.prepare(&init, Some("another/repo#42")).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(creation["input_data"].as_str().unwrap()).unwrap(),
            json!({"operation": "init_survey", "definition": definition})
        );
        assert_eq!(
            creation["source"]["Project"],
            json!({"project_id": "owner.testnet/discovery", "version_key": RELEASE})
        );
        assert!(creation.get("secrets_ref").is_none());
        assert!(s.prepare(&init, Some("wrong/repo#42")).is_err());
        let mut past = definition.clone();
        past["deadline_ns"] = json!(1_000_000_000_000_000_000u64);
        fs::write(&file, past.to_string()).unwrap();
        assert!(s
            .prepare(&init, None)
            .unwrap_err()
            .to_string()
            .contains("has passed"));
        fs::write(&file, "{}").unwrap();
        assert!(s.prepare(&init, None).is_err());
        fs::remove_file(file).unwrap();

        let survey_id = "c".repeat(64);
        let finalize = Preparation::Finalize {
            survey_id: survey_id.clone(),
        };
        let finalization = s.prepare(&finalize, None).unwrap();
        assert_eq!(finalization["source"], creation["source"]);
        assert_eq!(
            serde_json::from_str::<Value>(finalization["input_data"].as_str().unwrap()).unwrap(),
            json!({"operation": "finalize", "survey_id": survey_id})
        );
        assert_eq!(
            finalization["secrets_ref"],
            json!({"profile": "nearai", "account_id": "organizer.testnet"})
        );
        assert!(s.prepare(&finalize, Some("another/repo#42")).is_err());
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
    fn report_requires_a_transaction_before_any_network_call() {
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
