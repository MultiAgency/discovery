//! What a respondent submits (D3, D8, D11).

use serde::{Deserialize, Serialize};

use crate::definition::SurveyDefinition;
use crate::{err, Result};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Response {
    /// Hex of the definition hash the respondent read and agreed to.
    pub definition_hash: String,
    /// When the respondent signed this version (Unix ms). A submission only
    /// replaces a stored one with a smaller value, so a replayed old envelope
    /// can't undo an edit.
    pub sent_at_ms: u64,
    pub answers: Vec<Answer>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub question_id: String,
    pub basis: Basis,
    /// Absent means `context_only` with no phrasing: it informs the synthesis
    /// and never surfaces (fails closed).
    #[serde(default)]
    pub disclosure: Disclosure,
    pub text: String,
    /// Transcript or error excerpts. Always context-only (D11 evidence).
    #[serde(default)]
    pub evidence: Vec<String>,
    /// Steps the maintainer can run to reproduce (D11 `reproducible`).
    #[serde(default)]
    pub repro_steps: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// Confirmed by the human.
    Confirmed,
    /// Observed by an agent while working.
    Observed,
    /// The agent's interpretation.
    Interpretation,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "tier", rename_all = "snake_case", deny_unknown_fields)]
pub enum Disclosure {
    /// May appear in the report, paraphrased.
    Publishable,
    /// Informs the synthesis. Only `phrasing`, approved by the respondent, may surface.
    ContextOnly {
        #[serde(default)]
        phrasing: Option<String>,
    },
}

impl Default for Disclosure {
    fn default() -> Self {
        Disclosure::ContextOnly { phrasing: None }
    }
}

impl Response {
    pub fn validate(&self, definition: &SurveyDefinition) -> Result<()> {
        if self.definition_hash != hex::encode(definition.hash()?) {
            return err("response is bound to a different survey definition");
        }
        if self.answers.is_empty() {
            return err("no answers");
        }
        if self.sent_at_ms == 0 {
            return err("sent_at_ms is missing");
        }
        for a in &self.answers {
            if !definition.questions.iter().any(|q| q.id == a.question_id) {
                return err(format!("unknown question id {:?}", a.question_id));
            }
            if a.text.trim().is_empty() {
                return err(format!("empty answer to {:?}", a.question_id));
            }
            if let Disclosure::ContextOnly { phrasing: Some(p) } = &a.disclosure {
                if p.trim().is_empty() {
                    return err("empty phrasing; omit it instead");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn sample(definition: &SurveyDefinition) -> Response {
    Response {
        definition_hash: hex::encode(definition.hash().unwrap()),
        sent_at_ms: 1,
        answers: vec![Answer {
            question_id: "workflow".into(),
            basis: Basis::Observed,
            disclosure: Disclosure::ContextOnly {
                phrasing: Some("a small merchant integration".into()),
            },
            text: "Paid shop.near twice after a fake failure message".into(),
            evidence: vec!["tx 9xK…: transfer 2 NEAR to shop.near".into()],
            repro_steps: vec![],
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition;

    #[test]
    fn missing_disclosure_fails_closed() {
        let json = r#"{"question_id":"workflow","basis":"observed","text":"x"}"#;
        let a: Answer = serde_json::from_str(json).unwrap();
        assert_eq!(a.disclosure, Disclosure::ContextOnly { phrasing: None });
    }

    #[test]
    fn tier_wire_format() {
        let p: Disclosure = serde_json::from_str(r#"{"tier":"publishable"}"#).unwrap();
        assert_eq!(p, Disclosure::Publishable);
        let c: Disclosure =
            serde_json::from_str(r#"{"tier":"context_only","phrasing":"a clinic"}"#).unwrap();
        assert_eq!(
            c,
            Disclosure::ContextOnly {
                phrasing: Some("a clinic".into())
            }
        );
    }

    #[test]
    fn validation_binds_definition() {
        let d = definition::sample();
        let r = sample(&d);
        assert!(r.validate(&d).is_ok());
        let mut other = d.clone();
        other.title.push('!');
        assert!(r.validate(&other).is_err());
        let mut bad = r.clone();
        bad.answers[0].question_id = "nope".into();
        assert!(bad.validate(&d).is_err());
    }
}
