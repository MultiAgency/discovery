//! Local request-to-report demo: `scripts/demo.sh`.
//!
//! Runs the real survey, envelope, admission, publication-check, finalization
//! and cleanup code on the synthetic fixtures in `scripts/synthetic/`. Storage,
//! time and inference are test doubles and there are no transactions, so the
//! result says nothing about deployed confidentiality. With
//! `DISCOVERY_DEMO_DIR` set, it writes the transcript, the public reports and a
//! machine-checkable `outcomes.json` there. Every outcome must pass.

use std::cell::RefCell;
use std::collections::BTreeSet;

use discovery_protocol::definition::SurveyDefinition;
use discovery_protocol::report::{Report, Status};
use discovery_protocol::response::{Disclosure, Response};
use discovery_protocol::{Error, Result};
use serde_json::{json, Value};

use crate::collection;
use crate::finalize::{finalize, Llm, ATTEMPTS};
use crate::platform::memory::Memory;
use crate::platform::Mode;
use crate::survey::fixtures::encrypt;
use crate::survey::{init, responses_prefix, status, submit};

const DEFINITION: &str = include_str!("../../../scripts/synthetic/definition.json");
const ANSWERS: [&str; 3] = [
    include_str!("../../../scripts/synthetic/answers1.json"),
    include_str!("../../../scripts/synthetic/answers2.json"),
    include_str!("../../../scripts/synthetic/answers3.json"),
];
const CONTEXT_ONLY: &str = include_str!("../../../scripts/synthetic/context-only.json");

/// Every fixture answer carries this marker; no artifact may contain it.
const MARKER: &str = "SYNTHETIC";
const SECOND: u64 = 1_000_000_000;
/// Simulated block time at creation.
const T0: u64 = 1_800_000_000 * SECOND;
const DEADLINE: u64 = T0 + 3_600 * SECOND;

const PROVENANCE: &str = "LOCAL SIMULATION. Real code: survey definitions and hashing, \
X25519/XChaCha20-Poly1305 response envelopes, admission, deduplication, freezing, publication \
checks, finalization and cleanup. Simulated: storage (in memory; sealing is a reversible test \
tag, not encryption), time (fixed block timestamps), inference (scripted drafts and audit; no \
model, no attestation) and transactions (none). Nothing here is chain-verified or evidence of \
deployed confidentiality.";

type Draft = Box<dyn Fn(&Value) -> Value>;

/// Scripted stand-in for the confidential model. Drafts read the synthesis
/// input, as a model would, to cite the private source labels of answers.
struct Scripted {
    drafts: RefCell<Vec<Draft>>,
    /// (kind, input) for every call.
    calls: RefCell<Vec<(&'static str, Value)>>,
}

impl Scripted {
    fn new(drafts: Vec<Draft>) -> Self {
        Self {
            drafts: RefCell::new(drafts),
            calls: RefCell::default(),
        }
    }
    fn count(&self, kind: &str) -> usize {
        self.calls.borrow().iter().filter(|c| c.0 == kind).count()
    }
}

impl Llm for Scripted {
    fn model(&self) -> &str {
        "scripted-local-drafts (no model)"
    }
    fn json(&self, system: &str, user: &str) -> Result<Value> {
        let input: Value = serde_json::from_str(user)?;
        if system.starts_with("You audit a report") {
            self.calls.borrow_mut().push(("audit", input));
            return Ok(json!({ "leaks": [] }));
        }
        self.calls.borrow_mut().push(("draft", input.clone()));
        if self.drafts.borrow().is_empty() {
            return Err(Error("scripted model has no draft left".into()));
        }
        let draft = self.drafts.borrow_mut().remove(0);
        Ok(draft(&input))
    }
}

/// Private source labels of authorized answers matching `question` and `basis`:
/// publishable, or context-only with an approved phrasing.
fn cite(input: &Value, question: &str, basis: &str) -> Vec<String> {
    let respondents = input["respondents"].as_array().into_iter().flatten();
    respondents
        .flat_map(|r| r["answers"].as_array().into_iter().flatten())
        .filter(|a| a["question_id"] == question && a["basis"] == basis)
        .filter(|a| a["tier"] == "publishable" || !a["may_surface_as"].is_null())
        .map(|a| a["source_id"].as_str().unwrap().to_string())
        .collect()
}

fn clean_draft(input: &Value) -> Value {
    let both = |a: Vec<String>, b: Vec<String>| [a, b].concat();
    json!({
        "summary": "Joining a survey breaks down in two places: setup surfaces configuration problems late, and announcements leave the end of collection unclear.",
        "findings": [
            { "statement": "Verification can fail with a remote error before any local check names the missing setting, which leads agents to retry blindly.",
              "source_ids": both(cite(input, "obstacle", "observed"), cite(input, "outcome", "confirmed")) },
            { "statement": "Readers can take the deadline as the moment collection stops, because announcements do not explain that finalization is what closes it.",
              "source_ids": cite(input, "obstacle", "interpretation") },
            { "statement": "Affected contexts include a contributor joining from a managed work laptop and a maintainer responding on behalf of a small team.",
              "source_ids": both(cite(input, "workflow", "observed"), cite(input, "workflow", "confirmed")) }
        ],
        "disagreements": [],
        "open_questions": ["Which settings could have safe defaults instead of being required?"],
        "acceptance_criteria": [
            { "criterion": "First-run setup",
              "change": "Before contacting any service, the CLI reports all absent local configuration in one message.",
              "rationale": "Remote errors that arrive before local checks cause repeated blind attempts." },
            { "criterion": "Survey announcement",
              "change": "State that responses are still accepted after the deadline, until the organizer starts finalization.",
              "rationale": "Corrects the assumption that the deadline ends collection." }
        ]
    })
}

/// Same draft, but the summary names a context-only host.
fn leaking_draft(input: &Value) -> Value {
    let mut draft = clean_draft(input);
    draft["summary"] = json!(
        "Setup fails on build-farm-7.internal.example.invalid before naming the missing setting."
    );
    draft
}

fn withheld_draft(_: &Value) -> Value {
    json!({ "summary": "Verification fails on qa-runner-3.internal.example.invalid behind a proxy.",
        "findings": [], "disagreements": [], "open_questions": [], "acceptance_criteria": [] })
}

/// Identifier-shaped tokens of all context-only fixture text.
fn confidential_identifiers(responses: &[Response]) -> BTreeSet<String> {
    responses
        .iter()
        .flat_map(|r| &r.answers)
        .filter(|a| matches!(a.disclosure, Disclosure::ContextOnly { .. }))
        .flat_map(|a| crate::checks::identifiers(&a.text))
        .collect()
}

/// Problems that make `text` unfit for a public artifact.
fn public_problems(text: &str, identifiers: &BTreeSet<String>) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut out: Vec<String> = identifiers
        .iter()
        .filter(|id| lower.contains(id.as_str()))
        .map(|_| "confidential identifier".into())
        .collect();
    if text.contains(MARKER) {
        out.push("fixture text".into());
    }
    let bytes = text.as_bytes();
    if bytes
        .windows(4)
        .any(|w| w[0] == b'S' && w[1].is_ascii_digit() && w[2] == b'.' && w[3].is_ascii_digit())
    {
        out.push("private source label".into());
    }
    for detail in [
        "reuses confidential",
        "confidential identifier",
        "audit:",
        "rejected",
    ] {
        if lower.contains(detail) {
            out.push("rejection detail".into());
        }
    }
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let numbers = [
        "one", "two", "three", "four", "five", "several", "many", "most",
    ];
    let counted = [
        "respond",
        "participant",
        "people",
        "person",
        "agent",
        "submission",
    ];
    if words.windows(2).any(|w| {
        (w[0].bytes().all(|b| b.is_ascii_digit()) || numbers.contains(&w[0]))
            && counted.iter().any(|c| w[1].starts_with(c))
    }) {
        out.push("respondent count".into());
    }
    out
}

struct Demo {
    transcript: Vec<String>,
    outcomes: Vec<Value>,
}

impl Demo {
    fn say(&mut self, line: impl Into<String>) {
        self.transcript.push(line.into());
    }
    fn outcome(&mut self, id: &str, name: &str, pass: bool, observed: Value) {
        self.say(format!(
            "  [{}] outcome {id}: {name}",
            if pass { "pass" } else { "FAIL" }
        ));
        self.outcomes
            .push(json!({ "id": id, "name": name, "pass": pass, "observed": observed }));
    }
}

fn local_report(report: &Report) -> String {
    format!(
        "> {PROVENANCE}\n\n{}\n---\nLocal simulation: no finalize transaction, not chain-verified.\n",
        report.markdown()
    )
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> T {
    serde_json::from_str(text).expect("valid fixture")
}

#[test]
fn local_demo() {
    let mut demo = Demo {
        transcript: vec![PROVENANCE.into(), String::new()],
        outcomes: Vec::new(),
    };
    let mut definition: SurveyDefinition = parse(DEFINITION);
    definition.deadline_ns = DEADLINE;
    let responses: Vec<Response> = ANSWERS.iter().map(|a| parse(a)).collect();
    let context_only: Response = parse(CONTEXT_ONLY);
    let identifiers =
        confidential_identifiers(&[responses.clone(), vec![context_only.clone()]].concat());
    let target = format!("{}#{}", definition.target.repo, definition.target.number);

    // 1. Create a survey attached to an issue (simulated on-chain call).
    let mut p = Memory::new(Mode::Near { block_ns: T0 });
    let created = init(&p, definition.clone()).unwrap();
    let id = created.survey_id.clone();
    let state = status(&p, &id).unwrap();
    demo.say(format!(
        "Survey {} for {target}; deadline = creation + 1 h (simulated block time)",
        &id[..12]
    ));
    let open = state.key_present && !state.closed;
    demo.outcome(
        "1",
        "survey created for a specific issue",
        state.definition.target == definition.target && open,
        json!({ "survey_id": id, "target": target, "accepting_responses": open }),
    );

    // 2. Distinct responses with mixed tiers and bases, over simulated HTTPS.
    p.mode = Mode::Https;
    let first = encrypt(&created, &definition, &responses[0]);
    let accepted = [&first, &encrypt(&created, &definition, &responses[1])]
        .iter()
        .all(|e| submit(&p, &id, e).unwrap().accepted);
    let slots = |p: &Memory| p.list(&responses_prefix(&id)).len();
    let tiers: BTreeSet<_> = responses[..2]
        .iter()
        .flat_map(|r| &r.answers)
        .map(|a| match &a.disclosure {
            Disclosure::Publishable => "publishable",
            Disclosure::ContextOnly { phrasing: Some(_) } => "context_only with phrasing",
            Disclosure::ContextOnly { phrasing: None } => "context_only without phrasing",
        })
        .collect();
    let bases: BTreeSet<_> = responses[..2]
        .iter()
        .flat_map(|r| &r.answers)
        .map(|a| a.basis)
        .collect();
    demo.say("answers1.json and answers2.json are encrypted to the survey key and submitted.");
    demo.outcome(
        "2",
        "distinct responses with mixed tiers and bases accepted",
        accepted && slots(&p) == 2 && tiers.len() == 3 && bases.len() == 3,
        json!({ "stored_responses": slots(&p), "tiers": tiers, "bases": bases }),
    );

    // 3. A retry with identical answers is re-encrypted and contributes once.
    let retry = encrypt(&created, &definition, &responses[0]);
    let retried = submit(&p, &id, &retry).unwrap().accepted;
    demo.say("answers1.json is submitted again; the CLI encrypts it afresh.");
    demo.outcome(
        "3",
        "identical retry re-encrypted and counted once",
        retried && retry != first && slots(&p) == 2,
        json!({ "fresh_ciphertext": retry != first, "stored_responses": slots(&p) }),
    );

    // 4. Finalization before the deadline is refused, before any inference.
    let model = Scripted::new(vec![Box::new(leaking_draft), Box::new(clean_draft)]);
    p.mode = Mode::Near {
        block_ns: DEADLINE - SECOND,
    };
    let early = finalize(&p, &model, &id).unwrap_err();
    let open = !status(&p, &id).unwrap().closed;
    demo.say("The organizer tries to finalize one second before the deadline.");
    demo.outcome(
        "4",
        "finalization before the deadline rejected",
        early.0 == "the collection period has not ended" && model.calls.borrow().is_empty() && open,
        json!({ "error": early.0, "model_calls": model.calls.borrow().len(), "still_open": open }),
    );

    // 5. The deadline alone does not close collection.
    p.mode = Mode::Near {
        block_ns: DEADLINE + 60 * SECOND,
    };
    let after = status(&p, &id).unwrap();
    p.mode = Mode::Https;
    let late = submit(&p, &id, &encrypt(&created, &definition, &responses[2]))
        .unwrap()
        .accepted;
    demo.say("A minute after the deadline, answers3.json is submitted; nobody has finalized yet.");
    let open = !after.closed && after.key_present;
    demo.outcome(
        "5",
        "deadline passed but submissions still accepted",
        open && late && slots(&p) == 3,
        json!({ "open_after_deadline": open, "late_submission_accepted": late,
            "stored_responses": slots(&p) }),
    );

    // 6–7a. Finalize: the leaking draft is rejected, the corrected one released.
    p.mode = Mode::Near {
        block_ns: DEADLINE + 120 * SECOND,
    };
    let report = finalize(&p, &model, &id).unwrap();
    let markdown = local_report(&report);
    let problems = public_problems(&markdown, &identifiers);
    let findings: Vec<Value> = report
        .body
        .findings
        .iter()
        .map(|f| json!({ "basis": f.basis }))
        .collect();
    let criteria = report.body.acceptance_criteria.len();
    demo.say("The organizer finalizes after the deadline.");
    let grounded = report.body.findings.iter().all(|f| !f.basis.is_empty());
    demo.outcome(
        "6",
        "useful report with grounded acceptance-criterion changes",
        report.status == Status::Released
            && report.target == definition.target
            && criteria >= 1
            && grounded
            && problems.is_empty(),
        json!({ "status": report.status, "findings": findings,
            "acceptance_criteria_changes": criteria, "public_problems": problems }),
    );
    let calls = model.calls.borrow();
    let inputs: Vec<&Value> = calls
        .iter()
        .filter(|c| c.0 == "draft")
        .map(|c| &c.1)
        .collect();
    let reasons = inputs.get(1).map_or(json!([]), |i| {
        i["previous_attempt_rejected_because"].clone()
    });
    let named_identifier = reasons.as_array().into_iter().flatten().any(|r| {
        r.as_str()
            .unwrap_or_default()
            .contains("confidential identifier")
    });
    let contributors = inputs
        .first()
        .map_or(0, |i| i["respondents"].as_array().map_or(0, Vec::len));
    drop(calls);
    demo.say(
        "  Draft 1 named a context-only host and was rejected; the reasons went back to \
         the model only. Draft 2 passed the checks and the (scripted) audit.",
    );
    demo.outcome(
        "7a",
        "confidential draft rejected, corrected draft released",
        named_identifier && model.count("draft") == 2 && model.count("audit") == 1,
        json!({ "draft_calls": model.count("draft"), "audit_calls": model.count("audit"),
            "rejection_kind": if named_identifier { "confidential identifier" } else { "other" },
            "retry_contributed_once": contributors == 3 }),
    );

    // 8. Closed, key discarded, no more submissions, retry returns the stored report.
    collection::assert_discarded(&p, &id);
    let closed = status(&p, &id).unwrap();
    let again = Scripted::new(vec![]);
    let same = finalize(&p, &again, &id).unwrap() == report;
    p.mode = Mode::Https;
    let refused = submit(&p, &id, &first).unwrap_err();
    demo.say(
        "Afterwards the survey is closed, its key and responses are replaced by markers, \
         and a finalization retry returns the stored report.",
    );
    demo.outcome(
        "8",
        "closure, cleanup, refusal and identical retry without inference",
        closed.closed
            && !closed.key_present
            && closed.report_released
            && refused.0 == "survey is closed"
            && same
            && again.calls.borrow().is_empty(),
        json!({ "closed": closed.closed, "key_present": closed.key_present,
            "report_released": closed.report_released, "late_submission_error": refused.0,
            "retry_identical": same, "retry_model_calls": again.calls.borrow().len() }),
    );

    // 7b. A separate survey: when every attempt is rejected, the report is withheld.
    let mut withheld_definition = definition.clone();
    withheld_definition.title = "Joining a survey through an agent (context-only answers)".into();
    let mut q = Memory::new(Mode::Near { block_ns: T0 });
    let other = init(&q, withheld_definition.clone()).unwrap();
    q.mode = Mode::Https;
    submit(
        &q,
        &other.survey_id,
        &encrypt(&other, &withheld_definition, &context_only),
    )
    .unwrap();
    q.mode = Mode::Near {
        block_ns: DEADLINE + SECOND,
    };
    let rejected = Scripted::new(
        (0..ATTEMPTS)
            .map(|_| Box::new(withheld_draft) as Draft)
            .collect(),
    );
    let withheld = finalize(&q, &rejected, &other.survey_id);
    let withheld_markdown = withheld.as_ref().map(local_report).unwrap_or_default();
    let withheld_ok = withheld
        .as_ref()
        .is_ok_and(|r| r.status == Status::Withheld)
        && withheld_markdown.contains("could not be released")
        && public_problems(&withheld_markdown, &identifiers).is_empty();
    if withheld_ok {
        collection::assert_discarded(&q, &other.survey_id);
    }
    demo.say(format!(
        "Separate survey {}: context-only.json, with no approved phrasing. \
         Every draft names a private host.",
        &other.survey_id[..12]
    ));
    demo.outcome(
        "7b",
        "every rejected attempt withholds the report",
        withheld_ok && rejected.count("draft") == ATTEMPTS,
        json!({ "status": withheld.as_ref().map(|r| r.status).ok(),
            "error": withheld.as_ref().err().map(|e| e.0.clone()),
            "draft_calls": rejected.count("draft") }),
    );

    demo.say(String::new());
    demo.say("Public report:");
    demo.say(markdown.clone());
    let transcript = demo.transcript.join("\n");
    let outcomes = serde_json::to_string_pretty(&json!({
        "provenance": PROVENANCE, "outcomes": demo.outcomes,
    }))
    .unwrap();
    println!("{transcript}");
    for (name, text) in [
        ("transcript", &transcript),
        ("outcomes.json", &outcomes),
        ("withheld report", &withheld_markdown),
    ] {
        let problems = public_problems(text, &identifiers);
        // Outcome names and error strings legitimately mention rejection.
        let problems: Vec<_> = problems
            .iter()
            .filter(|p| *p != "rejection detail" || name == "withheld report")
            .collect();
        assert!(problems.is_empty(), "{name}: {problems:?}");
    }
    if let Some(dir) = std::env::var_os("DISCOVERY_DEMO_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in [
            ("transcript.txt", &transcript),
            ("outcomes.json", &outcomes),
            ("report.md", &markdown),
            ("withheld.md", &withheld_markdown),
        ] {
            std::fs::write(dir.join(file), text).unwrap();
        }
    }
    let failed: Vec<_> = demo.outcomes.iter().filter(|o| o["pass"] != true).collect();
    assert!(failed.is_empty(), "failed outcomes: {failed:#?}");
}
