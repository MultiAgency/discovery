//! Mechanical publication checks: the stated guarantee.
//!
//! A report fails if any of its strings:
//! 1. repeats [`NGRAM`] consecutive words of `context_only` text, or of
//!    `publishable` text when the organizer forbade quotes; approved phrasings
//!    and the survey's own wording are exempt;
//! 2. contains an identifier-shaped token (email, URL, domain, NEAR account,
//!    path, handle, long hex or number) that appears in `context_only` material
//!    and nowhere publishable;
//! 3. contains a term the organizer withheld;
//! 4. states how many responded: a quantity word followed, at most two words
//!    later (three after "of"), by a word for respondents or responses
//!    ([`states_count`]). This
//!    is a backstop for the prompt; it misses semantic forms ("two humans").

use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

use discovery_protocol::definition::SurveyDefinition;
use discovery_protocol::response::{Basis, Disclosure, Response};

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

pub const NGRAM: usize = 6;

/// Quantity words, and the whole words (or their plural) they must not
/// count. `scripts/smoke.sh` applies the same rule to published reports.
const QUANTITIES: [&str; 26] = [
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven",
    "twelve", "dozen", "both", "single", "couple", "handful", "few", "several", "many", "most",
    "half", "majority", "minority", "none", "neither",
];
const COUNTED: [&str; 8] = [
    "respondent",
    "responder",
    "response",
    "participant",
    "people",
    "person",
    "agent",
    "submission",
];

/// What the checks compare a report against.
#[derive(Default)]
pub struct Material {
    forbidden_grams: BTreeSet<Vec<String>>,
    forbidden_ids: BTreeSet<String>,
    withheld: Vec<String>,
}

/// One interpretation of disclosure permissions for synthesis, grounding and checks.
pub struct Prepared {
    pub material: Material,
    pub respondents: Vec<Value>,
    pub authorized: Vec<Value>,
    pub confidential: Vec<Value>,
    /// All labels are private; only authorized answers can ground findings,
    /// and a finding's public basis comes from theirs.
    pub sources: BTreeMap<String, Option<Basis>>,
}

impl Prepared {
    pub fn new(definition: &SurveyDefinition, responses: &[Response]) -> Self {
        // Context-only text is confidential; publishable text the organizer
        // forbade quoting is public but must be reworded. Both forbid
        // n-grams, but only confidential text forbids identifiers.
        let mut private = Vec::new();
        let mut paraphrased = Vec::new();
        let mut allowed = vec![definition.title.as_str()];
        allowed.extend(definition.questions.iter().map(|q| q.prompt.as_str()));
        let mut respondents = Vec::new();
        let mut authorized = Vec::new();
        let mut confidential = Vec::new();
        let mut sources = BTreeMap::new();
        for (r, response) in responses.iter().enumerate() {
            let mut answers = Vec::new();
            for (i, a) in response.answers.iter().enumerate() {
                let id = format!("S{}.{}", r + 1, i + 1);
                let mut draft =
                    json!({"source_id": id, "question_id": a.question_id, "basis": a.basis});
                let source = match &a.disclosure {
                    Disclosure::Publishable => {
                        // The synthesis and audit inputs label quotable text the same way.
                        let label = if definition.rules.no_direct_quotes {
                            paraphrased.push(a.text.as_str());
                            "paraphrase_only"
                        } else {
                            allowed.push(a.text.as_str());
                            "text"
                        };
                        authorized.push(json!({ label: a.text }));
                        draft["tier"] = json!("publishable");
                        draft[label] = json!(a.text);
                        Some(a.basis)
                    }
                    Disclosure::ContextOnly { phrasing } => {
                        private.push(a.text.as_str());
                        allowed.extend(phrasing.iter().map(String::as_str));
                        if let Some(p) = phrasing {
                            authorized.push(json!({"approved_phrasing": p}));
                        }
                        confidential.push(json!({"fact": a.text}));
                        draft["tier"] = json!("context_only");
                        draft["raw_context"] = json!(a.text);
                        draft["may_surface_as"] = json!(phrasing);
                        phrasing.as_ref().map(|_| a.basis)
                    }
                };
                sources.insert(id, source);
                answers.push(draft);
            }
            // Grouped per respondent, unlabelled, so no label can tie
            // findings to one person.
            respondents.push(json!({ "answers": answers }));
        }
        let allowed_grams: BTreeSet<_> = allowed.iter().flat_map(|t| ngrams(t)).collect();
        let public_ids: BTreeSet<_> = allowed
            .iter()
            .chain(&paraphrased)
            .flat_map(|t| identifiers(t))
            .collect();
        Self {
            material: Material {
                forbidden_grams: private
                    .iter()
                    .chain(&paraphrased)
                    .flat_map(|t| ngrams(t))
                    .filter(|g| !allowed_grams.contains(g))
                    .collect(),
                forbidden_ids: private
                    .iter()
                    .flat_map(|t| identifiers(t))
                    .filter(|i| !public_ids.contains(i))
                    .collect(),
                withheld: definition.rules.withheld_terms.clone(),
            },
            respondents,
            authorized,
            confidential,
            sources,
        }
    }
}

impl Material {
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
            if let Some(phrase) = states_count(s) {
                out.push(format!(
                    "states how many responded (\"{phrase}\"): describe positions without counts"
                ));
            }
        }
        out
    }
}

/// The first phrase that counts respondents or responses, such as "two human
/// respondents" or "most responses".
fn states_count(text: &str) -> Option<String> {
    let w = words(text);
    (0..w.len()).find_map(|i| {
        let quantity =
            w[i].bytes().all(|b| b.is_ascii_digit()) || QUANTITIES.contains(&w[i].as_str());
        // A partitive ("one of our human respondents") allows one more word.
        // Pronoun uses ("one understood that responses…") can match too; a
        // false positive costs a retry, a missed count would be published.
        let gap = if w.get(i + 1).map(String::as_str) == Some("of") {
            3
        } else {
            2
        };
        let end = (i + 2 + gap).min(w.len());
        let counted = |w: &str| COUNTED.contains(&w.strip_suffix('s').unwrap_or(w));
        let noun = (i + 1..end).find(|&j| counted(&w[j]))?;
        quantity.then(|| w[i..=noun].join(" "))
    })
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
    use discovery_protocol::definition::DisclosureRules;

    fn definition(rules: DisclosureRules) -> SurveyDefinition {
        SurveyDefinition {
            rules,
            ..crate::survey::fixtures::definition()
        }
    }

    fn response(disclosure: Disclosure, text: &str) -> Response {
        crate::survey::fixtures::response(Basis::Observed, disclosure, text)
    }

    const SECRET: &str = "I maintain the only Rust bindings for Velvet and the installer called \
        northwind.dev twice, logging request 9f3a2b1c4d5e6f708192a3b4c5d6e7f8 to /Users/ana/agent/log.txt";

    const PUBLIC: &str = "setup should name the missing variable before any network call";

    fn material(rules: DisclosureRules) -> Material {
        let ctx = Disclosure::ContextOnly {
            phrasing: Some("a maintainer of a language binding".into()),
        };
        Prepared::new(
            &definition(rules),
            &[
                response(ctx, SECRET),
                response(Disclosure::Publishable, PUBLIC),
            ],
        )
        .material
    }

    #[test]
    fn verbatim_context_fails() {
        let m = material(DisclosureRules::default());
        assert!(!m
            .violations(&[
                "One respondent said: the only Rust bindings for Velvet and the installer"
            ])
            .is_empty());
    }

    #[test]
    fn approved_phrasing_is_exempt() {
        let m = material(DisclosureRules::default());
        assert!(m
            .violations(&["Affects a maintainer of a language binding."])
            .is_empty());
    }

    #[test]
    fn identifiers_from_context_fail() {
        let m = material(DisclosureRules::default());
        assert!(!m.violations(&["See northwind.dev for details."]).is_empty());
        assert!(!m
            .violations(&["Log at /Users/ana/agent/log.txt."])
            .is_empty());
        assert!(!m
            .violations(&["Hash 9f3a2b1c4d5e6f708192a3b4c5d6e7f8."])
            .is_empty());
    }

    #[test]
    fn publishable_quotes_follow_rules() {
        assert!(material(DisclosureRules::default())
            .violations(&[PUBLIC])
            .is_empty());
        let strict = DisclosureRules {
            no_direct_quotes: true,
            ..Default::default()
        };
        assert!(!material(strict).violations(&[PUBLIC]).is_empty());
    }

    /// Forbidding quotes asks for rewording; it doesn't make published
    /// identifiers confidential.
    #[test]
    fn no_quotes_keeps_publishable_identifiers_public() {
        let strict = definition(DisclosureRules {
            no_direct_quotes: true,
            ..Default::default()
        });
        let m = Prepared::new(
            &strict,
            &[
                response(
                    Disclosure::Publishable,
                    "The installer at northwind.dev stops early",
                ),
                response(
                    Disclosure::ContextOnly { phrasing: None },
                    "I tested it from velvet.internal and northwind.dev",
                ),
            ],
        )
        .material;
        assert!(m
            .violations(&["Setup on northwind.dev should list missing settings."])
            .is_empty());
        assert!(!m.violations(&["Seen from velvet.internal."]).is_empty());
    }

    #[test]
    fn withheld_terms_fail() {
        let rules = DisclosureRules {
            withheld_terms: vec!["Velvet".into()],
            ..Default::default()
        };
        assert!(!material(rules)
            .violations(&["Something in velvet."])
            .is_empty());
    }

    /// Phrases shared with `scripts/test-smoke.sh`, which applies the same
    /// rule to deployed reports; several were published by survey 32fac7cf….
    #[test]
    fn respondent_counts_fail_in_any_form() {
        let phrases: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/count-phrases.json")).unwrap();
        let list =
            |kind: &str| -> Vec<String> { serde_json::from_value(phrases[kind].clone()).unwrap() };
        let m = material(DisclosureRules::default());
        for counted in list("counted") {
            let reasons = m.violations(&[&counted]);
            assert!(
                reasons
                    .iter()
                    .any(|r| r.starts_with("states how many responded")),
                "{counted}: {reasons:?}"
            );
        }
        for uncounted in list("uncounted") {
            assert!(m.violations(&[&uncounted]).is_empty(), "{uncounted}");
        }
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
            let m = Prepared::new(
                &definition(DisclosureRules::default()),
                &[response(
                    Disclosure::ContextOnly { phrasing: None },
                    private,
                )],
            )
            .material;
            assert!(!m.violations(&[public]).is_empty(), "{private} -> {public}");
        }
    }
}
