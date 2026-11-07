//! What a respondent submits.

use serde::{Deserialize, Serialize};

use crate::definition::SurveyDefinition;
use crate::{err, Result};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Response {
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
    /// Checks that need no survey definition, so a client can run them
    /// before any network call.
    pub fn validate_answers(&self) -> Result<()> {
        if self.answers.is_empty() {
            return err("no answers");
        }
        for a in &self.answers {
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

    pub fn validate(&self, definition: &SurveyDefinition) -> Result<()> {
        self.validate_answers()?;
        match self
            .answers
            .iter()
            .find(|a| !definition.questions.iter().any(|q| q.id == a.question_id))
        {
            Some(a) => err(format!("unknown question id {:?}", a.question_id)),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
pub(crate) fn sample() -> Response {
    Response {
        answers: vec![Answer {
            question_id: "workflow".into(),
            basis: Basis::Observed,
            disclosure: Disclosure::ContextOnly {
                phrasing: Some("a contributor setting up a local CLI".into()),
            },
            text: "Setup failed twice because DISCOVERY_RELEASE was unset".into(),
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
            serde_json::from_str(r#"{"tier":"context_only","phrasing":"a small team"}"#).unwrap();
        assert_eq!(
            c,
            Disclosure::ContextOnly {
                phrasing: Some("a small team".into())
            }
        );
    }

    #[test]
    fn validation_checks_questions_and_unknown_fields_are_refused() {
        let d = definition::sample();
        let r = sample();
        assert!(r.validate(&d).is_ok());
        let mut bad = r.clone();
        bad.answers[0].question_id = "nope".into();
        assert!(bad.validate(&d).is_err());
        assert!(bad.validate_answers().is_ok());
        for broken in [
            Response { answers: vec![] },
            Response {
                answers: vec![Answer {
                    text: " ".into(),
                    ..r.answers[0].clone()
                }],
            },
            Response {
                answers: vec![Answer {
                    disclosure: Disclosure::ContextOnly {
                        phrasing: Some(" ".into()),
                    },
                    ..r.answers[0].clone()
                }],
            },
        ] {
            assert!(broken.validate_answers().is_err());
            assert!(broken.validate(&d).is_err());
        }
        let mut extra = serde_json::to_value(&r).unwrap();
        extra["extra"] = serde_json::json!(1);
        assert!(serde_json::from_value::<Response>(extra).is_err());
    }
}
