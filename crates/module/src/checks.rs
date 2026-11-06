//! Mechanical publication checks: the stated guarantee (D3).
//!
//! A report fails if any of its strings:
//! 1. repeats [`NGRAM`] consecutive words of `context_only` material (raw text,
//!    evidence, repro steps), or of `publishable` text when the organizer
//!    forbade quotes; approved phrasings and the survey's own wording are exempt;
//! 2. contains an identifier-shaped token (email, URL, domain, NEAR account,
//!    path, handle, long hex or number) that appears in `context_only` material
//!    and nowhere publishable;
//! 3. contains a term the organizer withheld.

use std::collections::BTreeSet;

use discovery_protocol::definition::SurveyDefinition;
use discovery_protocol::response::{Disclosure, Response};

use crate::merge::words;

pub const NGRAM: usize = 6;

/// What the checks compare a report against.
#[derive(Default)]
pub struct Material {
    forbidden_grams: BTreeSet<Vec<String>>,
    forbidden_ids: BTreeSet<String>,
    withheld: Vec<String>,
}

impl Material {
    pub fn new(definition: &SurveyDefinition, responses: &[Response]) -> Self {
        let mut forbidden = Vec::new();
        let mut allowed = vec![definition.title.as_str()];
        allowed.extend(definition.questions.iter().map(|q| q.prompt.as_str()));
        for r in responses {
            for a in &r.answers {
                match &a.disclosure {
                    Disclosure::Publishable => {
                        let target = if definition.rules.no_direct_quotes {
                            &mut forbidden
                        } else {
                            &mut allowed
                        };
                        target.push(a.text.as_str());
                        target.extend(a.repro_steps.iter().map(String::as_str));
                    }
                    Disclosure::ContextOnly { phrasing } => {
                        forbidden.push(a.text.as_str());
                        forbidden.extend(a.repro_steps.iter().map(String::as_str));
                        allowed.extend(phrasing.iter().map(String::as_str));
                    }
                }
                forbidden.extend(a.evidence.iter().map(String::as_str));
            }
        }
        let allowed_grams: BTreeSet<_> = allowed.iter().flat_map(|t| ngrams(t)).collect();
        let allowed_ids: BTreeSet<_> = allowed.iter().flat_map(|t| identifiers(t)).collect();
        Self {
            forbidden_grams: forbidden
                .iter()
                .flat_map(|t| ngrams(t))
                .filter(|g| !allowed_grams.contains(g))
                .collect(),
            forbidden_ids: forbidden
                .iter()
                .flat_map(|t| identifiers(t))
                .filter(|i| !allowed_ids.contains(i))
                .collect(),
            withheld: definition.rules.withheld_terms.clone(),
        }
    }

    /// Reasons the strings fail, empty when they pass. Each names the
    /// offending text so the model can fix it on retry. Reasons stay inside
    /// the enclave: they go back to the model, never into a released report.
    pub fn violations(&self, strings: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        for s in strings {
            for run in reused_runs(s, &self.forbidden_grams) {
                out.push(format!(
                    "reuses confidential wording \"{run}\": paraphrase it completely"
                ));
            }
            let lower = s.to_lowercase();
            if let Some(id) = self
                .forbidden_ids
                .iter()
                .find(|id| lower.contains(id.as_str()))
            {
                out.push(format!(
                    "contains the confidential identifier \"{id}\": remove it"
                ));
            }
            if let Some(t) = self
                .withheld
                .iter()
                .find(|t| !t.is_empty() && lower.contains(&t.to_lowercase()))
            {
                out.push(format!("contains the withheld term \"{t}\": remove it"));
            }
        }
        out
    }
}

/// Maximal runs of words in `text` covered by forbidden n-grams.
fn reused_runs(text: &str, forbidden: &BTreeSet<Vec<String>>) -> Vec<String> {
    let w = words(text);
    let mut covered = vec![false; w.len()];
    for (i, window) in w.windows(NGRAM).enumerate() {
        if forbidden.contains(window) {
            covered[i..i + NGRAM].iter_mut().for_each(|c| *c = true);
        }
    }
    let mut runs = Vec::new();
    let mut start = None;
    for (i, &c) in covered.iter().chain(std::iter::once(&false)).enumerate() {
        match (c, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                runs.push(w[s..i].join(" "));
                start = None;
            }
            _ => {}
        }
    }
    runs
}

fn ngrams(text: &str) -> BTreeSet<Vec<String>> {
    words(text).windows(NGRAM).map(<[String]>::to_vec).collect()
}

/// Identifier-shaped tokens, lowercased.
pub fn identifiers(text: &str) -> BTreeSet<String> {
    text.split_whitespace()
        // Keep whole URLs/paths and also inspect values inside logs, JSON,
        // Markdown links and URL components. A key=value prefix is not part
        // of an email's identity, and a URL's host must stay confidential even
        // when the model omits its scheme or path.
        .flat_map(|t| t.split(|c: char| "=,;()[]{}<>\"'`!?&".contains(c)))
        .flat_map(|t| std::iter::once(t).chain(t.split([':', '/'])))
        .map(|t| t.trim_matches(':').trim_end_matches('.'))
        .filter(|t| is_identifier(t))
        .map(str::to_lowercase)
        .collect()
}

fn is_identifier(t: &str) -> bool {
    if t.len() < 4 {
        return false;
    }
    let hex = t.len() >= 16 && t.chars().all(|c| c.is_ascii_hexdigit());
    let digits = t.len() >= 6 && t.chars().all(|c| c.is_ascii_digit());
    let handle = t.starts_with('@') && t.len() > 2;
    let email_or_url = t.contains('@') || t.contains("://");
    let path = (t.starts_with('/') || t.starts_with("~/")) && t[1..].contains('/');
    let dotted = {
        let parts: Vec<&str> = t.split('.').collect();
        parts.len() >= 2
            && parts.iter().all(|p| {
                !p.is_empty()
                    && p.chars()
                        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            })
            && parts
                .last()
                .is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()))
    };
    let ipv4 = t.split('.').count() == 4
        && t.split('.')
            .all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_digit()));
    hex || digits || handle || email_or_url || path || dotted || ipv4
}

#[cfg(test)]
mod tests {
    use super::*;
    use discovery_protocol::definition::{
        DisclosureRules, Question, SurveyKind, Target, DEFINITION_VERSION,
    };
    use discovery_protocol::response::{Answer, Basis};

    fn definition(rules: DisclosureRules) -> SurveyDefinition {
        SurveyDefinition {
            version: DEFINITION_VERSION,
            kind: SurveyKind::IssueSurvey,
            title: "Payment allowance".into(),
            target: Target {
                repo: "MultiAgency/discovery".into(),
                number: 1,
            },
            deadline_ns: 1,
            questions: vec![Question {
                id: "q".into(),
                prompt: "What happened when you paid?".into(),
            }],
            rules,
        }
    }

    fn response(disclosure: Disclosure, text: &str, evidence: &[&str]) -> Response {
        Response {
            definition_hash: String::new(),
            sent_at_ms: 1,
            answers: vec![Answer {
                question_id: "q".into(),
                basis: Basis::Observed,
                disclosure,
                text: text.into(),
                evidence: evidence.iter().map(|s| s.to_string()).collect(),
                repro_steps: vec![],
            }],
        }
    }

    const SECRET: &str =
        "I run the only clinic in Dayton and our agent paid shop.near twice yesterday";

    fn material(rules: DisclosureRules) -> Material {
        let ctx = Disclosure::ContextOnly {
            phrasing: Some("a small healthcare deployment".into()),
        };
        Material::new(
            &definition(rules),
            &[
                response(
                    ctx,
                    SECRET,
                    &["tx 9f3a2b1c4d5e6f708192a3b4c5d6e7f8 at /Users/ana/agent/log.txt"],
                ),
                response(
                    Disclosure::Publishable,
                    "the second payment should be blocked by an allowance per merchant",
                    &[],
                ),
            ],
        )
    }

    #[test]
    fn clean_report_passes() {
        let m = material(DisclosureRules::default());
        assert!(m
            .violations(&[
                "A small healthcare deployment saw a duplicate payment after a fake failure."
            ])
            .is_empty());
    }

    #[test]
    fn verbatim_context_fails() {
        let m = material(DisclosureRules::default());
        assert!(!m
            .violations(&["One respondent said: the only clinic in Dayton and our agent paid"])
            .is_empty());
    }

    #[test]
    fn approved_phrasing_is_exempt() {
        let m = material(DisclosureRules::default());
        assert!(m
            .violations(&["Affects a small healthcare deployment."])
            .is_empty());
    }

    #[test]
    fn identifiers_from_context_fail() {
        let m = material(DisclosureRules::default());
        assert!(!m.violations(&["See shop.near for details."]).is_empty());
        assert!(!m
            .violations(&["Log at /Users/ana/agent/log.txt."])
            .is_empty());
        assert!(!m
            .violations(&["Hash 9f3a2b1c4d5e6f708192a3b4c5d6e7f8."])
            .is_empty());
    }

    #[test]
    fn publishable_quotes_follow_rules() {
        let quote = "the second payment should be blocked by an allowance per merchant";
        assert!(material(DisclosureRules::default())
            .violations(&[quote])
            .is_empty());
        let strict = DisclosureRules {
            no_direct_quotes: true,
            ..Default::default()
        };
        assert!(!material(strict).violations(&[quote]).is_empty());
    }

    #[test]
    fn withheld_terms_fail() {
        let rules = DisclosureRules {
            withheld_terms: vec!["Dayton".into()],
            ..Default::default()
        };
        assert!(!material(rules)
            .violations(&["Something in dayton."])
            .is_empty());
    }

    #[test]
    fn reasons_name_the_offending_text() {
        let m = material(DisclosureRules::default());
        let v = m.violations(&["the only clinic in Dayton and our agent paid shop.near"]);
        assert!(
            v.iter()
                .any(|r| r.contains("\"the only clinic in dayton and our agent paid shop near\"")),
            "{v:?}"
        );
        assert!(v.iter().any(|r| r.contains("shop.near")), "{v:?}");
    }

    #[test]
    fn identifier_shapes() {
        for id in [
            "a@b.co",
            "https://x.y/z",
            "alice.near",
            "x.testnet",
            "/var/log/a",
            "~/x/y",
            "@octocat",
            "192.168.1.10",
            "1234567",
        ] {
            assert!(is_identifier(id), "{id}");
        }
        for not in ["hello", "e.g", "v1.2", "1.5", "the"] {
            assert!(!is_identifier(not), "{not}");
        }
    }

    #[test]
    fn identifiers_in_logs_json_and_urls_cannot_be_reformatted_into_public_text() {
        for (private, public) in [
            ("user=alice@example.com", "alice@example.com"),
            (r#"{"email":"alice@example.com"}"#, "alice@example.com"),
            ("email:alice@example.com", "alice@example.com"),
            ("url=https://private.example/account", "private.example"),
            ("file=/Users/alice/log.txt", "/Users/alice/log.txt"),
            (
                "transaction=9f3a2b1c4d5e6f708192a3b4c5d6e7f8",
                "9f3a2b1c4d5e6f708192a3b4c5d6e7f8",
            ),
        ] {
            let m = Material::new(
                &definition(DisclosureRules::default()),
                &[response(
                    Disclosure::ContextOnly { phrasing: None },
                    private,
                    &[],
                )],
            );
            assert!(!m.violations(&[public]).is_empty(), "{private} -> {public}");
        }
    }
}
