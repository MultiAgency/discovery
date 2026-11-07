//! The survey definition: fixed at `InitSurvey`, bound to every response by its hash.

use serde::{Deserialize, Serialize};

use crate::{err, sha256, Hash, Result};

pub const DEFINITION_VERSION: u32 = 2;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SurveyDefinition {
    pub version: u32,
    pub title: String,
    pub target: Target,
    /// Earliest NEAR block timestamp (nanoseconds) at which `Finalize` may run.
    /// HTTPS submissions remain accepted until finalization closes collection.
    pub deadline_ns: u64,
    pub questions: Vec<Question>,
    pub rules: DisclosureRules,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Target {
    /// `owner/name`
    pub repo: String,
    pub number: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub id: String,
    pub prompt: String,
}

/// Organizer-chosen restrictions. They can only tighten the service's own
/// publication checks, never loosen them or a respondent's tiers.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct DisclosureRules {
    /// Forbid quoting respondents at all, even `publishable` text.
    pub no_direct_quotes: bool,
    /// Extra terms that must never appear in the report.
    pub withheld_terms: Vec<String>,
}

impl SurveyDefinition {
    pub fn validate(&self) -> Result<()> {
        if self.version != DEFINITION_VERSION {
            return err(format!("unsupported definition version {}", self.version));
        }
        if self.target.number == 0 {
            return err("target.number must be a positive issue number");
        }
        if self.title.trim().is_empty() {
            return err("title is empty");
        }
        let Some((owner, name)) = self.target.repo.split_once('/') else {
            return err("target.repo must be owner/name");
        };
        if owner.is_empty() || name.is_empty() || name.contains('/') {
            return err("target.repo must be owner/name");
        }
        if self.questions.is_empty() {
            return err("no questions");
        }
        let mut seen = std::collections::BTreeSet::new();
        for q in &self.questions {
            if q.id.is_empty() || q.prompt.trim().is_empty() {
                return err("every question needs an id and a prompt");
            }
            if !seen.insert(q.id.as_str()) {
                return err(format!("duplicate question id {:?}", q.id));
            }
        }
        // A blank term would match every report and withhold it.
        if self
            .rules
            .withheld_terms
            .iter()
            .any(|t| t.trim().is_empty())
        {
            return err("withheld_terms must not be blank");
        }
        Ok(())
    }

    /// Canonical bytes: serde_json over a struct with fixed field order and no maps.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }

    /// The survey's identity everywhere: storage names and encrypted envelopes.
    pub fn hash(&self) -> Result<Hash> {
        Ok(sha256(&self.canonical_bytes()?))
    }
}

#[cfg(test)]
pub(crate) fn sample() -> SurveyDefinition {
    SurveyDefinition {
        version: DEFINITION_VERSION,
        title: "First-run configuration".into(),
        target: Target {
            repo: "MultiAgency/discovery".into(),
            number: 1,
        },
        deadline_ns: 1_800_000_000_000_000_000,
        questions: vec![
            Question {
                id: "workflow".into(),
                prompt: "What were you doing?".into(),
            },
            Question {
                id: "outcome".into(),
                prompt: "What should have happened?".into(),
            },
        ],
        rules: DisclosureRules::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_across_round_trip() {
        let d = sample();
        let back: SurveyDefinition = serde_json::from_slice(&d.canonical_bytes().unwrap()).unwrap();
        assert_eq!(d.hash().unwrap(), back.hash().unwrap());
    }

    #[test]
    fn any_change_changes_the_hash() {
        let d = sample();
        let mut later = d.clone();
        later.deadline_ns += 1;
        assert_ne!(d.hash().unwrap(), later.hash().unwrap());
    }

    #[test]
    fn validation() {
        assert!(sample().validate().is_ok());
        let mut d = sample();
        d.questions.push(d.questions[0].clone());
        assert!(d.validate().is_err());
        let mut d = sample();
        d.target.repo = "nope".into();
        assert!(d.validate().is_err());
        let mut d = sample();
        d.target.number = 0;
        assert!(d.validate().is_err());
        for blank in ["", " "] {
            let mut d = sample();
            d.rules.withheld_terms = vec![blank.into()];
            assert!(d.validate().is_err(), "{blank:?}");
        }
    }

    #[test]
    fn unknown_fields_are_refused() {
        let mut v = serde_json::to_value(sample()).unwrap();
        v["extra"] = serde_json::json!(1);
        assert!(serde_json::from_value::<SurveyDefinition>(v).is_err());
    }
}
