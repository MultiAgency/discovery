//! Wire formats shared by the CLI and the OutLayer module.
//!
//! - [`definition`]: the survey definition fixed at `InitSurvey` and its hash (D10).
//! - [`response`]: what a respondent submits, with disclosure tiers (D3).
//! - [`envelope`]: encryption to the survey key, with size-bucket padding (D7).
//! - [`identity`]: per-survey respondent keys and NEP-413 signatures.
//! - [`report`]: the released report (D4, D11).

pub mod definition;
pub mod envelope;
pub mod identity;
pub mod report;
pub mod response;

use std::fmt;

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

pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).map_err(|e| Error(format!("randomness: {e}")))?;
    Ok(out)
}

/// SHA-256, for callers outside this crate.
pub fn sha256_bytes(data: &[u8]) -> Hash {
    sha256(data)
}

pub(crate) fn sha256(data: &[u8]) -> Hash {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).into()
}

/// An ISO 8601 UTC timestamp with milliseconds, as JavaScript's `toISOString`.
pub fn iso_time(seconds: u64, millis: u32) -> String {
    let days = (seconds / 86_400) as i64;
    let rem = seconds % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::iso_time;

    #[test]
    fn utc_format_handles_epoch_day_rollover_and_leap_centuries() {
        assert_eq!(iso_time(0, 7), "1970-01-01T00:00:00.007Z");
        assert_eq!(iso_time(86_399, 999), "1970-01-01T23:59:59.999Z");
        assert_eq!(iso_time(86_400, 0), "1970-01-02T00:00:00.000Z");
        assert_eq!(iso_time(951_782_400, 0), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso_time(4_107_542_400, 0), "2100-03-01T00:00:00.000Z");
    }
}
