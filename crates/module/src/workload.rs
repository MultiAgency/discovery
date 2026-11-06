//! Workload approval is baked into the pinned module, not supplied by an
//! organizer or a secrets profile. Hardware attestation alone is insufficient.

use discovery_nearai::attest::Attestation;
use discovery_protocol::{Error, Result};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovedWorkload {
    compose_sha256: String,
    approval_reference: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    model: String,
    approved_workloads: Vec<ApprovedWorkload>,
}

impl Policy {
    fn parse(json: &str, model: &str) -> Result<Self> {
        let policy: Self = serde_json::from_str(json)
            .map_err(|_| Error("invalid release workload policy".into()))?;
        if policy.model != model || policy.approved_workloads.is_empty() {
            return Err(Error(
                "release has no approved workload policy for this model".into(),
            ));
        }
        for workload in &policy.approved_workloads {
            if workload.compose_sha256.len() != 64
                || !workload
                    .compose_sha256
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit())
                || workload.approval_reference.trim().is_empty()
            {
                return Err(Error("invalid approved workload record".into()));
            }
        }
        Ok(policy)
    }

    pub fn release(model: &str) -> Result<Self> {
        Self::parse(include_str!("../workload-policy.json"), model)
    }

    fn permits(&self, compose_hash: &str) -> bool {
        self.approved_workloads
            .iter()
            .any(|w| w.compose_sha256.eq_ignore_ascii_case(compose_hash))
    }

    pub fn check(&self, attestation: &Attestation) -> Result<()> {
        if attestation.models.is_empty()
            || attestation
                .models
                .iter()
                .any(|m| !self.permits(&m.summary.compose_hash))
        {
            return Err(Error(
                "model evidence does not match the release's approved workload policy".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use discovery_nearai::attest::{Model, Summary};
    use serde_json::json;
    fn summary(hash: &str) -> Summary {
        Summary {
            tcb: "UpToDate".into(),
            advisories: vec![],
            signer: "signer".into(),
            compose_hash: hash.into(),
        }
    }
    fn attestation(hashes: &[String]) -> Attestation {
        let models: Vec<Model> = hashes
            .iter()
            .map(|h| Model {
                summary: summary(h),
                gpu_token: "verified separately".into(),
                gpu_key: json!({}),
                public_key: "signer".into(),
            })
            .collect();
        Attestation {
            gateway: summary("gateway"),
            model: models[0].clone(),
            models,
        }
    }

    #[test]
    fn unapproved_missing_and_wrong_model_policies_fail_closed() {
        assert!(Policy::parse(r#"{"model":"model","approved_workloads":[]}"#, "model").is_err());
        assert!(Policy::parse(r#"{"model":"other","approved_workloads":[{"compose_sha256":"bad","approval_reference":"review"}]}"#, "model").is_err());
    }
    #[test]
    fn every_verified_candidate_must_match_an_approved_configuration() {
        let first = "a".repeat(64);
        let second = "b".repeat(64);
        let json = json!({"model":"model", "approved_workloads":[{"compose_sha256":first, "approval_reference":"reviewed source/build evidence"}]}).to_string();
        let policy = Policy::parse(&json, "model").unwrap();
        assert!(policy
            .check(&attestation(std::slice::from_ref(&first)))
            .is_ok());
        assert!(policy.check(&attestation(&[first, second])).is_err());
    }
    #[test]
    fn malformed_hashes_and_missing_review_references_are_refused() {
        for (hash, reference) in [
            ("a".repeat(63), "review"),
            ("g".repeat(64), "review"),
            ("a".repeat(64), " "),
        ] {
            let json = json!({"model":"model", "approved_workloads":[{"compose_sha256":hash,"approval_reference":reference}]}).to_string();
            assert!(Policy::parse(&json, "model").is_err());
        }
    }
}
