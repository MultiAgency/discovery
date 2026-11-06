//! The released report: the only thing `Finalize` publishes (D4, D11).

use serde::{Deserialize, Serialize};

use crate::definition::{SurveyKind, Target};
use crate::response::Basis;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub survey_id: String,
    pub kind: SurveyKind,
    pub title: String,
    pub target: Target,
    pub status: Status,
    pub counts: Counts,
    /// What the model wrote, after checks. Empty when withheld.
    pub body: Body,
    /// Model used for synthesis (canonical NEAR AI id).
    pub model: String,
    /// SHA-256 of the receipt: attestation evidence and every signed model
    /// turn, committed to this report's body. `None` when no model was called.
    #[serde(default)]
    pub receipt_sha256: Option<String>,
    /// Fixed disclosure of the report's reliability limits (D11).
    pub limits: Vec<String>,
    /// Why synthesis attempts were rejected. Only populated by module builds
    /// with the `diagnostics` feature, which run on synthetic data in the dev
    /// project and can't pass a respondent's release verification.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Released,
    /// The publication checks failed twice; nothing from the responses is published.
    Withheld,
}

/// Context only, never the case for a finding (D11).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct Counts {
    pub responses: u32,
    /// Near-identical responses grouped for weighting; no evidence is dropped.
    pub merged: u32,
    /// Legacy report field. New MVP reports always use zero.
    pub anchored: u32,
}

/// The part the model writes. Its JSON schema is [`Body::schema`].
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct Body {
    pub summary: String,
    pub findings: Vec<Finding>,
    pub disagreements: Vec<String>,
    pub open_questions: Vec<String>,
    pub acceptance_criteria: Vec<CriterionChange>,
    pub suggested_tasks: Vec<Task>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub statement: String,
    pub evidence: EvidenceClass,
    /// Steps a maintainer can run. Required for `reproducible`.
    pub repro_steps: Vec<String>,
    /// Product reviews: the criterion (question id) this finding is about.
    pub question_id: Option<String>,
    /// Product reviews: the verdict for that criterion.
    pub verdict: Option<Verdict>,
    /// Product-review failures carry severity; absent on older reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<Severity>,
    /// Derived from authorized source answers, never assigned by the model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub basis: Vec<Basis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_scenario: Option<String>,
    /// Private draft provenance. Never serialized in a released report.
    #[serde(default, skip_serializing)]
    pub source_ids: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceClass {
    Reproducible,
    EvidenceBacked,
    Reported,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Met,
    NotMet,
    Mixed,
    /// No authorized assessment supports a conclusion for this criterion.
    NotAssessed,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CriterionChange {
    pub criterion: String,
    pub change: String,
    pub rationale: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub title: String,
    pub detail: String,
}

pub const LIMITS: [&str; 4] = [
    "Respondents are not verified users; identities are free to create, so counts are context, not evidence.",
    "Findings are weighted by evidence: reproducible findings can be checked by the maintainer regardless of who submitted them.",
    "Combinations of approved phrasings could still identify a respondent to someone who already knows them.",
    "Response counts and submission times are visible in OutLayer's public job log; identities are not.",
];

impl Body {
    /// Every string the model wrote, for the publication checks.
    pub fn strings(&self) -> Vec<&str> {
        let mut out = vec![self.summary.as_str()];
        for f in &self.findings {
            out.push(&f.statement);
            out.extend(f.failure_scenario.iter().map(String::as_str));
            out.extend(f.repro_steps.iter().map(String::as_str));
        }
        out.extend(self.disagreements.iter().map(String::as_str));
        out.extend(self.open_questions.iter().map(String::as_str));
        for c in &self.acceptance_criteria {
            out.extend([c.criterion.as_str(), &c.change, &c.rationale]);
        }
        for t in &self.suggested_tasks {
            out.extend([t.title.as_str(), &t.detail]);
        }
        out
    }

    /// Model draft schema. Source IDs are private; basis is derived after parsing.
    pub fn schema() -> serde_json::Value {
        use serde_json::json;
        let strings = json!({ "type": "array", "items": { "type": "string" } });
        let nullable = |t: serde_json::Value| json!({ "anyOf": [t, { "type": "null" }] });
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["summary", "findings", "disagreements", "open_questions", "acceptance_criteria", "suggested_tasks"],
            "properties": {
                "summary": { "type": "string" },
                "findings": { "type": "array", "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["statement", "evidence", "repro_steps", "question_id", "source_ids"],
                    "properties": {
                        "statement": { "type": "string" },
                        "evidence": { "type": "string", "enum": ["reproducible", "evidence_backed", "reported"] },
                        "repro_steps": strings,
                        "question_id": nullable(json!({ "type": "string" })),
                        "source_ids": strings
                    }
                }},
                "disagreements": strings,
                "open_questions": strings,
                "acceptance_criteria": { "type": "array", "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["criterion", "change", "rationale"],
                    "properties": {
                        "criterion": { "type": "string" },
                        "change": { "type": "string" },
                        "rationale": { "type": "string" }
                    }
                }},
                "suggested_tasks": { "type": "array", "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["title", "detail"],
                    "properties": {
                        "title": { "type": "string" },
                        "detail": { "type": "string" }
                    }
                }}
            }
        })
    }
}

impl Report {
    /// What a receipt commits to: this survey and this report body.
    pub fn subject(&self) -> serde_json::Value {
        serde_json::json!({ "survey_id": self.survey_id, "body": self.body })
    }

    pub fn markdown(&self) -> String {
        let mut s = format!("## Survey report: {}\n\n", self.title);
        s += &format!(
            "Survey `{}` · {} responses · model `{}`{}\n\n",
            &self.survey_id[..12.min(self.survey_id.len())],
            self.counts.responses,
            self.model,
            self.receipt_sha256
                .as_ref()
                .map(|h| format!(" · receipt `{}`", &h[..12.min(h.len())]))
                .unwrap_or_default()
        );
        if self.status == Status::Withheld {
            s += "**This report could not be released under the disclosure rules.** No response content is published, and the responses have been discarded.\n\n";
        } else {
            let b = &self.body;
            s += &format!("{}\n\n", esc(&b.summary));
            if !b.findings.is_empty() {
                s += "### Findings\n\n";
                for f in &b.findings {
                    let class = match f.evidence {
                        EvidenceClass::Reproducible => "reproducible",
                        EvidenceClass::EvidenceBacked => "evidence-backed",
                        EvidenceClass::Reported => "reported",
                    };
                    let verdict = match (&f.question_id, f.verdict) {
                        (Some(q), Some(v)) => format!(" · `{q}`: {v:?}"),
                        _ => String::new(),
                    };
                    let severity = f.severity.map(|s| format!(" · {s:?}")).unwrap_or_default();
                    let basis = if f.basis.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " · {}",
                            f.basis
                                .iter()
                                .map(|b| match b {
                                    Basis::Confirmed => "confirmed by a human",
                                    Basis::Observed => "observed by an agent",
                                    Basis::Interpretation => "interpretation",
                                })
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    };
                    s += &format!(
                        "- **[{class}{verdict}{severity}{basis}]** {}\n",
                        esc(&f.statement)
                    );
                    if let Some(scenario) = &f.failure_scenario {
                        s += &format!("  - Failure scenario: {}\n", esc(scenario));
                    }
                    for step in &f.repro_steps {
                        s += &format!("  1. {}\n", esc(step));
                    }
                }
                s += "\n";
            }
            section(&mut s, "Disagreements", &b.disagreements);
            section(&mut s, "Open questions", &b.open_questions);
            if !b.acceptance_criteria.is_empty() {
                s += "### Proposed acceptance-criteria changes\n\n";
                for c in &b.acceptance_criteria {
                    s += &format!(
                        "- **{}**: {} ({})\n",
                        esc(&c.criterion),
                        esc(&c.change),
                        esc(&c.rationale)
                    );
                }
                s += "\n";
            }
            if !b.suggested_tasks.is_empty() {
                s += "### Suggested tasks\n\n";
                for t in &b.suggested_tasks {
                    s += &format!("- **{}**: {}\n", esc(&t.title), esc(&t.detail));
                }
                s += "\n";
            }
        }
        if !self.diagnostics.is_empty() {
            s += "### Diagnostics (development build)\n\n";
            for d in &self.diagnostics {
                s += &format!("- {d}\n");
            }
            s += "\n";
        }
        s += "<details><summary>Limits</summary>\n\n";
        for l in &self.limits {
            s += &format!("- {l}\n");
        }
        s += "\n</details>\n";
        s
    }
}

/// Neutralize text the model wrote before it reaches GitHub Markdown: no
/// `@mentions` (which notify people) and no raw HTML.
fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('@', "&#64;")
}

fn section(s: &mut String, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    *s += &format!("### {title}\n\n");
    for i in items {
        *s += &format!("- {}\n", esc(i));
    }
    *s += "\n";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_neutralizes_mentions_and_html() {
        let r = Report {
            survey_id: "abc".into(),
            kind: crate::definition::SurveyKind::IssueSurvey,
            title: "t".into(),
            target: Target {
                repo: "o/r".into(),
                number: 1,
            },
            status: Status::Released,
            counts: Counts::default(),
            body: Body {
                summary: "ping @octocat <img src=x>".into(),
                ..Default::default()
            },
            model: "m".into(),
            receipt_sha256: None,
            limits: vec![],
            diagnostics: vec![],
        };
        let md = r.markdown();
        assert!(!md.contains("@octocat") && !md.contains("<img"), "{md}");
        assert!(md.contains("&#64;octocat") && md.contains("&lt;img"));
    }

    #[test]
    fn body_round_trips_through_schema_shape() {
        let json = serde_json::json!({
            "summary": "s",
            "findings": [{ "statement": "x", "evidence": "reproducible", "repro_steps": ["a"], "question_id": null, "verdict": null }],
            "disagreements": [], "open_questions": [], "acceptance_criteria": [], "suggested_tasks": []
        });
        let b: Body = serde_json::from_value(json).unwrap();
        assert_eq!(b.findings[0].evidence, EvidenceClass::Reproducible);
        assert_eq!(b.strings(), vec!["s", "x", "a"]);
    }
}
