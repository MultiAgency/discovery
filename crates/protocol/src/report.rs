//! The released report: the only thing `Finalize` publishes.

use serde::{Deserialize, Serialize};

use crate::definition::Target;
use crate::response::Basis;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub survey_id: String,
    pub title: String,
    pub target: Target,
    pub status: Status,
    /// What the model wrote, after checks. Empty when withheld.
    pub body: Body,
    /// Model used for synthesis (canonical NEAR AI id).
    pub model: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Released,
    /// Every synthesis attempt failed the checks; nothing from the responses is published.
    Withheld,
}

/// Checked publication content. Private draft fields are not accepted here.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct Body {
    pub summary: String,
    pub findings: Vec<Finding>,
    pub disagreements: Vec<String>,
    pub open_questions: Vec<String>,
    pub acceptance_criteria: Vec<CriterionChange>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub statement: String,
    /// Derived from authorized source answers, never assigned by the model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub basis: Vec<Basis>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CriterionChange {
    pub criterion: String,
    pub change: String,
    pub rationale: String,
}

pub const LIMITS: [&str; 2] = [
    "Combinations of approved phrasings could still identify a respondent to someone who already knows them.",
    "Response counts and submission times are visible in OutLayer's public job log; identities are not.",
];

impl Body {
    /// Every string the model wrote, for the publication checks.
    pub fn strings(&self) -> Vec<&str> {
        let mut out = vec![self.summary.as_str()];
        out.extend(self.findings.iter().map(|f| f.statement.as_str()));
        out.extend(self.disagreements.iter().map(String::as_str));
        out.extend(self.open_questions.iter().map(String::as_str));
        for c in &self.acceptance_criteria {
            out.extend([c.criterion.as_str(), &c.change, &c.rationale]);
        }
        out
    }
}

impl Report {
    pub fn markdown(&self) -> String {
        let mut s = format!("## Survey report: {}\n\n", esc(&self.title));
        s += &format!(
            "Survey `{}` · model `{}`\n\n",
            &self.survey_id[..12.min(self.survey_id.len())],
            self.model,
        );
        if self.status == Status::Withheld {
            s += "**This report could not be released under the disclosure rules.** No response content is published, and the responses have been discarded.\n\n";
        } else {
            let b = &self.body;
            s += &format!("{}\n\n", esc(&b.summary));
            if !b.findings.is_empty() {
                s += "### Findings\n\n";
                for f in &b.findings {
                    let basis = f
                        .basis
                        .iter()
                        .map(|b| match b {
                            Basis::Confirmed => "confirmed by a human",
                            Basis::Observed => "observed by an agent",
                            Basis::Interpretation => "interpretation",
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    s += &if basis.is_empty() {
                        format!("- {}\n", esc(&f.statement))
                    } else {
                        format!("- **[{basis}]** {}\n", esc(&f.statement))
                    };
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
        }
        s += "<details><summary>Limits</summary>\n\n";
        for l in LIMITS {
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
            title: "Survey @octocat <img src=x>".into(),
            target: Target {
                repo: "o/r".into(),
                number: 1,
            },
            status: Status::Released,
            body: Body {
                summary: "ping @octocat <img src=x>".into(),
                ..Default::default()
            },
            model: "m".into(),
        };
        let md = r.markdown();
        assert!(!md.contains("@octocat") && !md.contains("<img"), "{md}");
        assert!(md.contains("&#64;octocat") && md.contains("&lt;img"));
    }

    #[test]
    fn publication_text_includes_every_model_string() {
        let json = serde_json::json!({
            "summary": "s",
            "findings": [{ "statement": "x" }],
            "disagreements": ["d"], "open_questions": ["o"],
            "acceptance_criteria": [{ "criterion": "c", "change": "h", "rationale": "r" }]
        });
        let b: Body = serde_json::from_value(json).unwrap();
        assert_eq!(b.strings(), vec!["s", "x", "d", "o", "c", "h", "r"]);
    }

    #[test]
    fn private_draft_source_ids_are_rejected() {
        let draft = serde_json::json!({
            "summary": "s",
            "findings": [{ "statement": "x", "source_ids": ["S1.1"] }],
            "disagreements": [], "open_questions": [], "acceptance_criteria": []
        });
        assert!(serde_json::from_value::<Body>(draft).is_err());
    }
}
