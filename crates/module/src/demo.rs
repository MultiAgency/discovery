//! Local request-to-report demo: `scripts/demo.sh`.
//!
//! LOCAL SIMULATION. Runs the real survey, envelope, admission, publication
//! check, finalization and cleanup code on the invented answers in
//! `fixtures/`. Simulated: storage (in memory; sealing is a reversible
//! test tag, not encryption), block time, inference (scripted drafts and audit;
//! no model, no attestation) and transactions (none). Nothing here is
//! chain-verified or evidence of deployed confidentiality.
//!
//! `scripts/demo.sh OUT_DIR` also saves the transcript and the two public
//! reports there (`DISCOVERY_DEMO_OUT`).

use std::cell::RefCell;
use std::path::Path;

use base64::{engine::general_purpose::STANDARD as B64, Engine};

use discovery_protocol::definition::SurveyDefinition;
use discovery_protocol::report::{Report, Status};
use discovery_protocol::response::{Disclosure, Response};
use discovery_protocol::Result;
use serde_json::{json, Value};

use crate::collection::{responses_prefix, REGISTRATIONS};
use crate::finalize::{finalize, Llm, ATTEMPTS};
use crate::platform::memory::Memory;
use crate::platform::Mode;
use crate::survey::fixtures::encrypt;
use crate::survey::{init, status, submit};

const DEFINITION: &str = include_str!("../../../fixtures/definition.json");
const ANSWERS: [(&str, &str); 3] = [
    (
        "answers1.json",
        include_str!("../../../fixtures/answers1.json"),
    ),
    (
        "answers2.json",
        include_str!("../../../fixtures/answers2.json"),
    ),
    (
        "answers3.json",
        include_str!("../../../fixtures/answers3.json"),
    ),
];
const CONTEXT_ONLY: &str = include_str!("../../../fixtures/context-only.json");

const SECOND: u64 = 1_000_000_000;
/// Simulated block time at creation.
const T0: u64 = 1_800_000_000 * SECOND;
const DEADLINE: u64 = T0 + 3_600 * SECOND;
const LABEL: &str = "> LOCAL SIMULATION with invented answers, simulated approval, scripted \
inference, simulated time and in-memory storage; not chain-verified.";

type Draft = fn(&Value) -> Value;

/// Scripted stand-in for the confidential model: it replays drafts in order
/// and its audit finds no leaks or unsupported claims, so only the mechanical checks reject. It keeps
/// the synthesis inputs it saw, which carry the private rejection reasons.
struct Scripted {
    drafts: RefCell<Vec<Draft>>,
    inputs: RefCell<Vec<Value>>,
}

impl Llm for Scripted {
    fn model(&self) -> &str {
        "scripted-local-drafts (no model)"
    }
    fn json(&self, system: &str, user: &str) -> Result<Value> {
        if system == crate::finalize::AUDIT {
            return Ok(json!({ "leaks": [], "unsupported_claims": [] }));
        }
        let input: Value = serde_json::from_str(user)?;
        let draft = self.drafts.borrow_mut().remove(0);
        let reply = draft(&input);
        self.inputs.borrow_mut().push(input);
        Ok(reply)
    }
}

fn scripted(drafts: Vec<Draft>) -> Scripted {
    Scripted {
        drafts: RefCell::new(drafts),
        inputs: RefCell::default(),
    }
}

/// Private source labels of authorized answers matching `question` and `basis`,
/// read from the synthesis input as a model would.
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
              "change": "Before contacting any service, the CLI checks local configuration and names the missing setting.",
              "rationale": "Remote errors that arrive before local checks cause repeated blind attempts.",
              "source_ids": both(cite(input, "obstacle", "observed"), cite(input, "outcome", "confirmed")) },
            { "criterion": "Survey announcement",
              "change": "State that responses are still accepted after the deadline, until the organizer starts finalization.",
              "rationale": "Corrects the assumption that the deadline ends collection.",
              "source_ids": cite(input, "obstacle", "interpretation") }
        ]
    })
}

/// The clean draft, but its summary names a context-only host.
fn leaking_draft(input: &Value) -> Value {
    let mut draft = clean_draft(input);
    draft["summary"] = json!(
        "Setup fails on build-farm-7.internal.example.invalid before naming the missing setting."
    );
    draft
}

/// The permissions a fixture encodes, without its private text. Owner
/// approval is simulated; this demo neither obtains nor verifies it.
fn permissions(name: &str, response: &Response) -> String {
    let mut out = format!("  {name}, approved permissions (simulated approval):");
    for a in &response.answers {
        let surfaces = match &a.disclosure {
            Disclosure::Publishable => "publishable".into(),
            Disclosure::ContextOnly { phrasing: Some(p) } => {
                format!("context-only, may appear only as \"{p}\"")
            }
            Disclosure::ContextOnly { phrasing: None } => "context-only, never stated".into(),
        };
        let basis = serde_json::to_value(a.basis).unwrap();
        out += &format!(
            "\n    {:<9} {:<15} {surfaces}",
            a.question_id,
            basis.as_str().unwrap()
        );
    }
    out
}

/// A public report must hold no fixture text, context-only identifier or
/// private source label.
fn assert_public(markdown: &str, responses: &[Response]) {
    assert!(!markdown.contains("SYNTHETIC"), "fixture text");
    for answer in responses.iter().flat_map(|r| &r.answers) {
        if matches!(answer.disclosure, Disclosure::ContextOnly { .. }) {
            for id in crate::checks::identifiers(&answer.text) {
                assert!(!markdown.to_lowercase().contains(&id), "identifier {id}");
            }
        }
    }
    let label = markdown
        .as_bytes()
        .windows(4)
        .any(|w| w[0] == b'S' && w[1].is_ascii_digit() && w[2] == b'.' && w[3].is_ascii_digit());
    assert!(!label, "private source label");
}

fn local_report(report: &Report) -> String {
    format!("{LABEL}\n\n{}", report.markdown())
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> T {
    serde_json::from_str(text).expect("valid fixture")
}

fn at(p: &Memory, block_ns: u64) -> Memory<'static> {
    p.view(Mode::Near { block_ns })
}

/// What a presenter keeps: the transcript and each public report on its own.
struct Demo {
    transcript: String,
    released: String,
    withheld: String,
}

impl Demo {
    /// Write into an existing directory, never replacing a file.
    fn save(&self, dir: &Path) -> std::io::Result<()> {
        use std::io::Write;
        for (name, text) in [
            ("transcript.txt", &self.transcript),
            ("released.md", &self.released),
            ("withheld.md", &self.withheld),
        ] {
            std::fs::File::create_new(dir.join(name))?.write_all(text.as_bytes())?;
        }
        Ok(())
    }
}

fn run() -> Demo {
    let mut out = String::new();
    macro_rules! say {
        ($($arg:tt)*) => {{
            let line = format!($($arg)*);
            println!("{line}");
            out += &line;
            out.push('\n');
        }};
    }
    let mut definition: SurveyDefinition = parse(DEFINITION);
    definition.deadline_ns = DEADLINE;
    let responses: Vec<Response> = ANSWERS.iter().map(|(_, a)| parse(a)).collect();
    let context_only: Response = parse(CONTEXT_ONLY);
    say!(
        "LOCAL SIMULATION. Real: the survey, envelope, admission, publication-check, \
         finalization and cleanup code. Simulated: owner approval (encoded in the invented \
         answers in fixtures), storage (in memory), block time, and inference \
         (scripted drafts, no model). No transactions; not chain-verified. Details: \
         crates/module/src/demo.rs.\n"
    );

    // 1. The maintainer creates a survey on an issue.
    let p = Memory::new(Mode::Near { block_ns: T0 });
    let created = init(&p, definition.clone()).unwrap();
    let id = created.survey_id.clone();
    let target = &definition.target;
    say!(
        "1. The request. Survey {} on {}#{}; the whole definition is public:",
        &id[..12],
        target.repo,
        target.number
    );
    for q in &definition.questions {
        say!("  {}: {}", q.id, q.prompt);
    }
    let rules = &definition.rules;
    assert!(!rules.no_direct_quotes && rules.withheld_terms.is_empty());
    say!(
        "  Restrictions: short quotes of publishable answers allowed (no_direct_quotes: false); \
         no withheld terms. Deadline: one simulated hour after creation, the earliest \
         finalization. At most {REGISTRATIONS} registrations."
    );

    // 2. Approved answers are encrypted to the survey key and submitted; an
    //    identical retry is re-encrypted but stored once.
    say!("\n2. Personal agents submit approved, encrypted answers:");
    let https = p.view(Mode::Https);
    let seal = |response: &Response| encrypt(&created, &definition, response);
    let send = |envelope: &str| submit(&https, &id, envelope);
    let mut envelopes = Vec::new();
    for ((name, _), response) in ANSWERS[..2].iter().zip(&responses) {
        say!("{}", permissions(name, response));
        envelopes.push(seal(response));
        assert!(send(envelopes.last().unwrap()).unwrap().accepted);
    }
    let size = |envelope: &str| B64.decode(envelope).unwrap().len();
    let plaintext: Vec<usize> = responses[..2]
        .iter()
        .map(|r| serde_json::to_vec(r).unwrap().len())
        .collect();
    let sealed: Vec<usize> = envelopes.iter().map(|e| size(e)).collect();
    assert!(plaintext[0] != plaintext[1] && sealed[0] == sealed[1]);
    say!(
        "  Answers of {} and {} bytes both seal to {}-byte envelopes.",
        plaintext[0],
        plaintext[1],
        sealed[0]
    );
    let resent = seal(&responses[0]);
    assert_ne!(resent, envelopes[0]);
    assert!(send(&resent).unwrap().accepted);
    assert_eq!(p.list(&responses_prefix(&id)).len(), 2);
    say!(
        "  answers1.json is retried with fresh ciphertext and counted once: 2 responses stored. \
         Responses are immutable."
    );

    // 3. The deadline is the earliest finalization, not the end of collection.
    let early = finalize(&at(&p, DEADLINE - SECOND), &scripted(vec![]), &id).unwrap_err();
    assert_eq!(early.0, "the collection period has not ended");
    say!(
        "\n3. Deadline semantics. Finalizing one second early is refused before any model \
         call (\"{}\"). answers3.json arrives after the deadline and is accepted: \
         collection closes only when finalization starts.",
        early.0
    );
    say!("{}", permissions(ANSWERS[2].0, &responses[2]));
    let late = seal(&responses[2]);
    assert_eq!(size(&late), sealed[0]);
    assert!(send(&late).unwrap().accepted);

    // 4. Finalization rejects a leaking draft, releases the corrected one,
    //    then closes the survey and discards responses and key.
    let model = scripted(vec![leaking_draft, clean_draft]);
    let report = finalize(&at(&p, DEADLINE + 60 * SECOND), &model, &id).unwrap();
    assert_eq!(report.status, Status::Released);
    assert!(model.drafts.borrow().is_empty());
    let inputs = model.inputs.borrow();
    let rejected = leaking_draft(&inputs[0]);
    let reasons = inputs[1]["previous_attempt_rejected_because"]
        .as_array()
        .unwrap();
    assert!(!reasons.is_empty());
    say!("\n4. Finalization. PRIVATE, shown only in this simulation:");
    say!(
        "  Draft 1 summary: \"{}\"",
        rejected["summary"].as_str().unwrap()
    );
    for reason in reasons {
        say!(
            "  Rejected; returned only to the model: {}",
            reason.as_str().unwrap()
        );
    }
    say!("  Draft 2 passes the checks and is released.");
    crate::collection::assert_discarded(&p, &id);
    let closed = status(&p, &id).unwrap();
    assert!(closed.closed && !closed.key_present && closed.report_released);
    assert_eq!(
        send(&seal(&responses[0])).unwrap_err().0,
        "survey is closed"
    );
    let retry = finalize(&at(&p, DEADLINE + 120 * SECOND), &scripted(vec![]), &id).unwrap();
    assert_eq!(retry, report);
    say!(
        "  Cleanup: the survey is closed, its key and responses replaced by markers, a new \
         submission gets \"survey is closed\", and a finalization retry returns the same \
         report without inference."
    );
    let released = local_report(&report);
    assert_public(&released, &responses);
    say!("\n{released}");

    // 5. A separate survey whose every draft fails the checks is withheld.
    let mut quiet = definition.clone();
    quiet.title = "Joining a survey through an agent (context-only answers)".into();
    let q = Memory::new(Mode::Near { block_ns: T0 });
    let other = init(&q, quiet.clone()).unwrap();
    let envelope = encrypt(&other, &quiet, &context_only);
    submit(&q.view(Mode::Https), &other.survey_id, &envelope).unwrap();
    let model = scripted(vec![leaking_draft as Draft; ATTEMPTS]);
    let withheld = finalize(&at(&q, DEADLINE), &model, &other.survey_id).unwrap();
    assert_eq!(withheld.status, Status::Withheld);
    crate::collection::assert_discarded(&q, &other.survey_id);
    say!(
        "\n5. Withholding. A survey of context-only answers without phrasings: all {ATTEMPTS} \
         drafts fail the checks, so only a fixed notice is published. Withholding is a \
         successful privacy outcome."
    );
    say!("{}", permissions("context-only.json", &context_only));
    let notice = local_report(&withheld);
    assert_public(&notice, &[context_only]);
    say!("\n{notice}");
    Demo {
        transcript: out,
        released,
        withheld: notice,
    }
}

/// `scripts/demo.sh`: fails unless every step behaves as described.
#[test]
fn local_demo() {
    let demo = run();
    if let Some(dir) = std::env::var_os("DISCOVERY_DEMO_OUT") {
        demo.save(Path::new(&dir)).expect("save the demo artifacts");
    }
}

#[test]
fn demo_artifacts_are_labelled_separate_and_never_overwritten() {
    let dir = std::env::temp_dir().join(format!("discovery-demo-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let demo = run();
    demo.save(&dir).unwrap();
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
    let transcript = read("transcript.txt");
    assert!(transcript.starts_with("LOCAL SIMULATION."));
    for step in ["\n1. ", "\n2. ", "\n3. ", "\n4. ", "\n5. ", "PRIVATE"] {
        assert!(transcript.contains(step), "{step}");
    }
    let (released, withheld) = (read("released.md"), read("withheld.md"));
    for report in [&released, &withheld] {
        assert!(report.starts_with(LABEL));
        assert!(!report.contains("PRIVATE") && !report.contains("Rejected"));
    }
    assert!(released.contains("### Findings") && !released.contains("could not be released"));
    assert!(withheld.contains("could not be released") && !withheld.contains("### Findings"));
    assert!(demo.save(&dir).is_err(), "overwrote an artifact");
    std::fs::remove_dir_all(dir).unwrap();
}
