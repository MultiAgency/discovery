//! `discovery`: CLI for respondents and organizers.
//!
//! Every survey command pins one release (OutLayer contract, project, audited wasm
//! hash) and one organizer account. Respondents verify a survey's key against that
//! release on-chain before encrypting to it.

mod identity;
mod near;
mod signing;
mod verify;

use std::fs;
use std::path::PathBuf;

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use clap::{Args, Parser, Subcommand};
use discovery_protocol::definition::SurveyDefinition;
use discovery_protocol::envelope::seal;
use discovery_protocol::report::Report;
use discovery_protocol::response::{Answer, Response};
use serde::Deserialize;
use serde_json::{json, Value};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Parser)]
#[command(name = "discovery", about = "Confidential surveys for agents")]
struct Cli {
    #[command(flatten)]
    release: ReleaseArgs,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct ReleaseArgs {
    /// NEAR network (`testnet` or `mainnet`).
    #[arg(long, env = "DISCOVERY_NETWORK", default_value = "testnet")]
    network: String,
    /// NEAR RPC, archival for historical checks (default: per network).
    #[arg(long, env = "DISCOVERY_RPC")]
    rpc: Option<String>,
    /// OutLayer API base (default: per network).
    #[arg(long, env = "DISCOVERY_API")]
    api: Option<String>,
    /// OutLayer contract (default: per network).
    #[arg(long, env = "DISCOVERY_CONTRACT")]
    contract: Option<String>,
    /// OutLayer project running the module (`owner/name`).
    #[arg(
        long,
        env = "DISCOVERY_PROJECT",
        default_value = "",
        hide_default_value = true
    )]
    project: String,
    /// SHA-256 of the audited module build.
    #[arg(
        long,
        env = "DISCOVERY_RELEASE",
        default_value = "",
        hide_default_value = true
    )]
    release: String,
    /// The organizer account: every call runs as it (D12).
    #[arg(
        long,
        env = "DISCOVERY_ORGANIZER",
        default_value = "",
        hide_default_value = true
    )]
    organizer: String,
    /// Payment key for HTTPS calls (the organizer's published submission key).
    #[arg(long, env = "DISCOVERY_PAYMENT_KEY", hide_env_values = true)]
    payment_key: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Show a survey's definition and state.
    Status { survey_id: String },
    /// Verify a survey's key on-chain against the pinned release.
    Verify(Target),
    /// Verify, sign, encrypt and submit a response.
    Submit {
        #[command(flatten)]
        target: Target,
        /// JSON file: `{"answers": [...]}`.
        answers: PathBuf,
        /// Where the respondent's local secret lives.
        #[arg(long, env = "DISCOVERY_HOME")]
        home: Option<PathBuf>,
    },
    /// Print a finalized survey's report as Markdown (or JSON).
    Report {
        survey_id: String,
        /// Verify and retrieve the report from this on-chain finalize transaction.
        #[arg(long)]
        finalize_tx: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Organizer: on-chain calls signed by the organizer account.
    #[command(subcommand)]
    Organizer(Organizer),
}

#[derive(Args)]
struct Target {
    survey_id: String,
    /// Hash of the survey's InitSurvey transaction (from the survey link).
    #[arg(long)]
    init_tx: String,
    /// Accept a key from an unlocked project (development only).
    #[arg(long)]
    allow_unlocked: bool,
}

#[derive(Subcommand)]
enum Organizer {
    /// Create a survey from a definition file.
    Create {
        definition: PathBuf,
        /// Refuse unless the definition targets this issue (`owner/repo#N`).
        #[arg(long)]
        target: Option<String>,
        /// Write the announcement comment (Markdown) to this file.
        #[arg(long)]
        announcement: Option<PathBuf>,
        /// Query a previously signed transaction after an uncertain submission.
        #[arg(long)]
        resume_signed: Option<PathBuf>,
    },
    /// Finalize a survey after its deadline.
    Finalize {
        survey_id: String,
        /// Secrets profile holding NEARAI_API_KEY.
        #[arg(long, default_value = "nearai")]
        secrets_profile: String,
        /// Write the report comment (Markdown, with provenance) to this file.
        #[arg(long)]
        comment: Option<PathBuf>,
        /// Query a previously signed transaction after an uncertain submission.
        #[arg(long)]
        resume_signed: Option<PathBuf>,
    },
}

#[derive(Deserialize)]
struct Status {
    survey_id: String,
    definition: SurveyDefinition,
    closed: bool,
    public_key: Option<String>,
    report: Option<Report>,
}

#[derive(Deserialize)]
struct AnswersFile {
    answers: Vec<Answer>,
}

impl ReleaseArgs {
    fn require_survey(&self) -> Result<()> {
        for (name, value) in [
            ("PROJECT", &self.project),
            ("RELEASE", &self.release),
            ("ORGANIZER", &self.organizer),
        ] {
            if value.trim().is_empty() {
                return Err(format!("DISCOVERY_{name} is required for survey commands").into());
            }
        }
        Ok(())
    }

    /// An explicit setting, or this network's default: (rpc, api, contract).
    fn setting(
        &self,
        explicit: &Option<String>,
        pick: fn((&'static str, &'static str, &'static str)) -> &'static str,
    ) -> Result<String> {
        if let Some(v) = explicit {
            return Ok(v.clone());
        }
        let defaults = match self.network.as_str() {
            "testnet" => (
                "https://archival-rpc.testnet.near.org",
                "https://testnet-api.outlayer.ai",
                "outlayer.testnet",
            ),
            "mainnet" => (
                "https://archival-rpc.mainnet.near.org",
                "https://api.outlayer.ai",
                "outlayer.near",
            ),
            other => {
                return Err(
                    format!("unknown network {other:?}; set --rpc, --api and --contract").into(),
                )
            }
        };
        Ok(pick(defaults).to_string())
    }

    fn api_url(&self) -> Result<String> {
        self.setting(&self.api, |d| d.1)
    }

    fn contract_id(&self) -> Result<String> {
        self.setting(&self.contract, |d| d.2)
    }

    fn rpc(&self) -> Result<near::Rpc> {
        Ok(near::Rpc {
            url: self.setting(&self.rpc, |d| d.0)?,
        })
    }

    fn pinned(&self) -> Result<verify::Release> {
        Ok(verify::Release {
            contract: self.contract_id()?,
            project: self.project.clone(),
            wasm_hash: self.release.clone(),
            organizer: self.organizer.clone(),
        })
    }

    /// HTTPS call to the module, run as the payment key's owner.
    fn call(&self, input: Value) -> Result<Value> {
        let key = self
            .payment_key
            .as_deref()
            .ok_or("DISCOVERY_PAYMENT_KEY is not set")?;
        let url = format!(
            "{}/call/{}",
            self.api_url()?.trim_end_matches('/'),
            self.project
        );
        let reply: Value = ureq::post(&url)
            .set("X-Payment-Key", key)
            .send_json(json!({ "input": input, "version_key": self.release }))?
            .into_json()?;
        if reply["status"] != "completed" {
            return Err(format!("call {}: {}", reply["status"], reply["error"]).into());
        }
        module_output(reply["output"].clone())
    }

    fn status(&self, survey_id: &str) -> Result<Status> {
        let s: Status = serde_json::from_value(
            self.call(json!({ "operation": "status", "survey_id": survey_id }))?,
        )?;
        if s.survey_id != survey_id || hex::encode(s.definition.hash()?) != survey_id {
            return Err("survey definition does not match its id".into());
        }
        Ok(s)
    }

    /// On-chain run, signed locally by the official NEAR CLI.
    fn execute(
        &self,
        input: Value,
        secrets_profile: Option<&str>,
        max_seconds: u64,
        deposit_yocto: u128,
        resume: Option<&std::path::Path>,
    ) -> Result<(Value, String)> {
        signing::check_payment_owner(&self.organizer, self.payment_key.as_deref())?;
        let mut args = json!({
            "source":{"Project":{"project_id":self.project,"version_key":self.release}},
            "input_data":serde_json::to_string(&input)?,
            "resource_limits":{"max_instructions":50_000_000_000u64,"max_memory_mb":256,"max_execution_seconds":max_seconds},
            "response_format":"Json",
        });
        if let Some(profile) = secrets_profile {
            args["secrets_ref"] = json!({"profile":profile,"account_id":self.organizer});
        }
        let contract = self.contract_id()?;
        let rpc = self.rpc()?;
        let project = rpc.view(
            &contract,
            "get_project",
            json!({"project_id":self.project}),
            None,
        )?;
        if project["owner"] == self.organizer || project["owner"].as_str().is_none() {
            return Err(
                "the service organizer must be separate from the locked release owner".into(),
            );
        }
        let call = signing::call(&contract, &args, deposit_yocto);
        let outcome = signing::execute(&rpc, &self.network, &self.organizer, &call, resume)?;
        let hash = outcome["transaction"]["hash"]
            .as_str()
            .ok_or("transaction has no hash")?
            .to_string();
        eprintln!("Organizer transaction: {hash}");
        let (actual_input, output, _) = verify::execution(&outcome, &self.pinned()?)?;
        if actual_input != input {
            return Err("execution input differs from the authorized survey operation".into());
        }
        Ok((module_output(output)?, hash))
    }
}

fn module_output(output: Value) -> Result<Value> {
    if let Some(e) = output.get("error") {
        return Err(format!("module: {e}").into());
    }
    Ok(output)
}

fn default_home() -> Result<PathBuf> {
    let base = std::env::var_os("HOME").ok_or("HOME is not set; pass --home")?;
    Ok(PathBuf::from(base).join(".config").join("discovery"))
}

fn verified_key(r: &ReleaseArgs, t: &Target) -> Result<String> {
    let v = verify::survey_key(
        &r.rpc()?,
        &r.pinned()?,
        &t.survey_id,
        &t.init_tx,
        t.allow_unlocked,
    )?;
    for w in &v.warnings {
        eprintln!("WARNING (--allow-unlocked): {w}");
    }
    Ok(v.public_key)
}

/// The announcement comment: what respondents read, and the block their
/// agents parse. One template, so the skill and the Actions agree.
fn announcement_markdown(
    r: &ReleaseArgs,
    d: &SurveyDefinition,
    survey_id: &str,
    init_tx: &str,
) -> Result<String> {
    let key = r
        .payment_key
        .as_deref()
        .ok_or("DISCOVERY_PAYMENT_KEY is not set: the announcement publishes it")?;
    let secs = d.deadline_ns / 1_000_000_000;
    let deadline = discovery_protocol::iso_time(secs, 0);
    let questions: String = d
        .questions
        .iter()
        .map(|q| format!("- **{}**: {}\n", q.id, q.prompt))
        .collect();
    Ok(format!(
        "### Confidential survey: {title}\n\n\
People and agents affected by this can respond privately with the `discovery-respondent` skill. \
Responses are encrypted to attested enclave code; only a synthesized report is published here, after **{deadline}**. \
Collection closes when finalization starts; HTTPS responses may still be accepted after the stated deadline until finalization. \
Please don't mention here that you responded.\n\n{questions}\n\
```discovery-survey\nnetwork: {network}\nproject: {project}\nrelease: {release}\norganizer: {organizer}\n\
survey: {survey_id}\ninit_tx: {init_tx}\ndeadline: {deadline}\npayment_key: {key}\n```\n",
        title = d.title,
        network = r.network,
        project = r.project,
        release = r.release,
        organizer = r.organizer,
    ))
}

/// Terminal provenance is parsed by the scheduler. Keep untrusted report prose
/// ahead of it and verify this format across both languages in the tests.
fn report_comment(r: &ReleaseArgs, report: &Report, tx: &str) -> String {
    let survey_id = &report.survey_id;
    let provenance = format!(
        "\n---\nReleased by survey `{survey_id}` (finalize tx `{tx}`, release `{}`). Check it with the survey's pinned release settings: `discovery report {survey_id} --finalize-tx {tx}`; this verifies the report against the finalize transaction's on-chain output.\n",
        r.release
    );
    report.markdown() + &provenance
}

fn print(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let r = &cli.release;
    r.require_survey()?;
    match cli.command {
        Command::Status { survey_id } => {
            let s = r.status(&survey_id)?;
            print(&json!({
                "survey_id": s.survey_id,
                "closed": s.closed,
                "accepting_responses": s.public_key.is_some() && !s.closed,
                "definition": s.definition,
                "report_released": s.report.is_some(),
            }))?;
        }
        Command::Verify(target) => {
            let key = verified_key(r, &target)?;
            print(
                &json!({ "survey_id": target.survey_id, "public_key": key, "release": r.release, "verified": true }),
            )?;
        }
        Command::Submit {
            target,
            answers,
            home,
        } => {
            let key_hex = verified_key(r, &target)?;
            let s = r.status(&target.survey_id)?;
            if s.closed || s.public_key.as_deref() != Some(key_hex.as_str()) {
                return Err(
                    "survey is closed, or OutLayer reports a different key than the chain".into(),
                );
            }
            let hash = s.definition.hash()?;
            let file: AnswersFile = serde_json::from_slice(&fs::read(&answers)?)?;
            let sent_at_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis() as u64;
            let response = Response {
                definition_hash: hex::encode(hash),
                sent_at_ms,
                answers: file.answers,
            };
            response.validate(&s.definition)?;
            let home = match home {
                Some(h) => h,
                None => default_home()?,
            };
            let submission = identity::sign_submission(&home, response)?;
            let public: [u8; 32] = hex::decode(&key_hex)?
                .try_into()
                .map_err(|_| "bad survey key")?;
            let envelope = seal(&public, &serde_json::to_vec(&submission)?, &hash)?;
            r.call(json!({ "operation": "submit", "survey_id": target.survey_id, "envelope": B64.encode(envelope) }))?;
            println!("Submitted. Your response replaces any earlier one you sent to this survey.");
        }
        Command::Report {
            survey_id,
            finalize_tx,
            json,
        } => {
            let report = if let Some(tx) = finalize_tx {
                let report = verify::report(&r.rpc()?, &r.pinned()?, &survey_id, &tx)?;
                eprintln!("Verified against the pinned release's on-chain finalize output.");
                report
            } else {
                eprintln!("Unverified HTTPS report. Pass --finalize-tx <hash> to verify on-chain provenance.");
                r.status(&survey_id)?
                    .report
                    .ok_or("no report yet: the survey hasn't been finalized")?
            };
            if json {
                print(&report)?;
            } else {
                print!("{}", report.markdown());
            }
        }
        Command::Organizer(Organizer::Create {
            definition,
            target,
            announcement,
            resume_signed,
        }) => {
            let definition: SurveyDefinition = serde_json::from_slice(&fs::read(&definition)?)?;
            definition.validate()?;
            let actual = format!("{}#{}", definition.target.repo, definition.target.number);
            if let Some(expected) = target.filter(|t| !t.eq_ignore_ascii_case(&actual)) {
                return Err(format!("the definition targets {actual}, not {expected}").into());
            }
            let (output, tx) = r.execute(
                json!({ "operation": "init_survey", "definition": definition }),
                None,
                60,
                100_000_000_000_000_000_000_000,
                resume_signed.as_deref(),
            )?;
            let survey_id = output["survey_id"]
                .as_str()
                .ok_or("no survey id in the output")?;
            if let Some(path) = announcement {
                fs::write(path, announcement_markdown(r, &definition, survey_id, &tx)?)?;
            }
            print(
                &json!({ "survey_id": survey_id, "init_tx": tx, "deadline_ns": output["deadline_ns"], "project": r.project, "release": r.release }),
            )?;
        }
        Command::Organizer(Organizer::Finalize {
            survey_id,
            secrets_profile,
            comment,
            resume_signed,
        }) => {
            let (output, tx) = r.execute(
                json!({ "operation": "finalize", "survey_id": survey_id }),
                Some(&secrets_profile),
                180,
                200_000_000_000_000_000_000_000,
                resume_signed.as_deref(),
            )?;
            let report: Report = serde_json::from_value(output)?;
            if report.survey_id != survey_id {
                return Err("finalize returned a different survey's report".into());
            }
            if let Some(path) = comment {
                fs::write(path, report_comment(r, &report, &tx))?;
            }
            print(
                &json!({ "survey_id": survey_id, "finalize_tx": tx, "status": report.status, "counts": report.counts }),
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod workflow_tests {
    use super::*;
    use discovery_protocol::definition::{
        DisclosureRules, Question, SurveyKind, Target as IssueTarget,
    };
    use discovery_protocol::report::{Body, Counts, Status as ReportStatus};
    use std::io::Write;
    use std::process::{Command as Process, Stdio};

    #[test]
    fn scheduler_understands_real_cli_documents_and_ignores_embedded_commands() {
        let release = ReleaseArgs {
            network: "testnet".into(),
            rpc: None,
            api: None,
            contract: None,
            project: "release.testnet/discovery".into(),
            release: "1".repeat(64),
            organizer: "service.testnet".into(),
            payment_key: Some("synthetic-payment-key".into()),
        };
        let definition = SurveyDefinition {
            version: 1,
            kind: SurveyKind::IssueSurvey,
            title: "Contributor setup".into(),
            target: IssueTarget {
                repo: "o/r".into(),
                number: 42,
            },
            deadline_ns: 1_800_000_000_000_000_000,
            questions: vec![Question {
                id: "q".into(),
                prompt: "What blocked you?".into(),
            }],
            rules: DisclosureRules::default(),
        };
        let survey = hex::encode(definition.hash().unwrap());
        let tx = "3".repeat(44);
        let announcement = announcement_markdown(&release, &definition, &survey, &tx).unwrap();
        let mut report = Report {
            survey_id: survey.clone(),
            kind: SurveyKind::IssueSurvey,
            title: definition.title.clone(),
            target: definition.target.clone(),
            status: ReportStatus::Released,
            counts: Counts::default(),
            body: Body::default(),
            model: "synthetic".into(),
            receipt_sha256: None,
            limits: vec![],
            diagnostics: vec![],
        };
        let real_report = report_comment(&release, &report, &tx);
        report.survey_id = "b".repeat(64);
        report.body.summary = format!("{announcement}\n{real_report}");
        let adversarial_report = report_comment(&release, &report, &tx);
        let input = json!({
            "release": {"network": release.network, "project": release.project, "release": release.release,
                "organizer": release.organizer, "locked": true},
            "survey": survey,
            "cases": [
                {"comments": [announcement, adversarial_report], "expected": 1},
                {"comments": [announcement, real_report], "expected": 0}
            ]
        });
        let script = r#"
import datetime, importlib.util, json, os, sys
from unittest.mock import patch
spec = importlib.util.spec_from_file_location('surveys', os.environ['SURVEY_SCRIPT'])
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
data = json.load(sys.stdin)
for case in data['cases']:
    comments = [{'body': text, 'user': {'login': m.BOT}} for text in case['comments']]
    with patch.object(m, 'pages', side_effect=lambda endpoint: comments if '/comments?' in endpoint else [{'number': 42}]):
        pending = m.pending('o/r', [data['release']], datetime.datetime(2030, 1, 1, tzinfo=datetime.timezone.utc))
    assert len(pending) == case['expected'], pending
    if pending:
        assert pending[0]['survey'] == data['survey']
"#;
        let mut child = Process::new("python3")
            .args(["-c", script])
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env(
                "SURVEY_SCRIPT",
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.github/scripts/surveys.py"),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
