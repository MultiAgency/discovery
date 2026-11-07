//! Wire formats shared by the CLI and the OutLayer module.
//!
//! - [`definition`]: the survey definition fixed at `InitSurvey` and its hash.
//! - [`response`]: what a respondent submits, with disclosure tiers.
//! - [`envelope`]: encryption to the survey key, with size-bucket padding.
//! - [`report`]: the released report.

pub mod definition;
pub mod envelope;
pub mod report;
pub mod response;

use std::fmt;

/// Public state returned by the service and consumed by the CLI.
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SurveyStatus {
    pub survey_id: String,
    pub definition: definition::SurveyDefinition,
    pub closed: bool,
    /// Every registration is used: new responses are refused, while a retry
    /// of an admitted one still succeeds. Releases before this field
    /// (such as `be2b43ef`) omit it.
    #[serde(default)]
    pub full: bool,
    /// Whether the service still holds the survey decryption key.
    pub key_present: bool,
    pub report_released: bool,
}

/// SHA-256 digest, hex-encoded on the wire.
pub type Hash = [u8; 32];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error(format!("json: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn err<T>(message: impl Into<String>) -> Result<T> {
    Err(Error(message.into()))
}

pub fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).map_err(|e| Error(format!("randomness: {e}")))?;
    Ok(out)
}

pub fn sha256(data: &[u8]) -> Hash {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registered release `be2b43ef` reports status without `full`.
    #[test]
    fn status_without_full_parses() {
        let definition = include_str!("../../../scripts/synthetic/definition.json");
        let old = format!(
            r#"{{"survey_id":"x","definition":{definition},"closed":false,"key_present":true,"report_released":false}}"#
        );
        let status: SurveyStatus = serde_json::from_str(&old).unwrap();
        assert!(!status.full && status.key_present);
    }
}
