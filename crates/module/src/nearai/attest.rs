// Derived from MultiAgency/private-ai@dd469a5 app/src/attest.rs. Local changes: module documentation in mod.rs.
//! Verifies the NEAR AI model this service encrypts to before any data is
//! sent, per docs.near.ai/cloud/verification: an Intel TDX quote whose report
//! data binds the model's signing key and our nonce and whose MRCONFIGID is
//! the hash of the attested compose file, and NVIDIA's signed verdict on its
//! GPUs for the same nonce. The gateway is not checked: under E2EE it relays
//! only ciphertext. Port of `core/attest.mjs`.
use anyhow::{anyhow, bail, Result};
use base64::Engine;
use dcap_qvl::configs::DefaultConfig;
use dcap_qvl::verify::QuoteVerifier;
use dcap_qvl::QuoteCollateralV3;
use p384::ecdsa::signature::Verifier;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The model reads private data, so its platform must be fully patched.
pub const MODEL_TCB: &[&str] = &["UpToDate"];

pub const NRAS: &str = "https://nras.attestation.nvidia.com";

/// What a quote verifier reports about one TDX quote.
pub struct Verified {
    pub status: String,
    pub advisory_ids: Vec<String>,
    pub report_data: [u8; 64],
    pub mr_config_id: [u8; 48],
    pub debug: bool,
}

/// Verifies a quote against Intel collateral at `now` (Unix seconds).
pub fn verify_quote(quote: &[u8], collateral: &QuoteCollateralV3, now: u64) -> Result<Verified> {
    let verified = QuoteVerifier::new_prod()
        .with_config::<DefaultConfig>()
        .verify(quote, collateral, now)?;
    let td = verified
        .report
        .as_td10()
        .ok_or_else(|| anyhow!("expected a TDX quote"))?;
    Ok(Verified {
        status: verified.status,
        advisory_ids: verified.advisory_ids,
        report_data: td.report_data,
        mr_config_id: td.mr_config_id,
        debug: td.td_attributes[0] & 1 == 1,
    })
}

/// Fetch a fresh NVIDIA verdict and the service's verification keys.
pub trait Nvidia {
    fn token(&self, payload: &Value) -> Result<String>;
    fn keys(&self) -> Result<Vec<Value>>;
}

pub struct Options<'a> {
    pub verify_quote: &'a dyn Fn(&[u8]) -> Result<Verified>,
    pub nvidia: &'a dyn Nvidia,
}

/// A JSON field that NEAR AI sends either as an object or as a JSON string of one.
fn parse(value: &Value) -> Result<Value> {
    match value {
        Value::String(text) => Ok(serde_json::from_str(text)?),
        other => Ok(other.clone()),
    }
}

/// Verify the TDX report, signer, nonce and application configuration;
/// returns the attested signing key.
fn check_report(
    report: &Value,
    nonce: &str,
    verify_quote: &dyn Fn(&[u8]) -> Result<Verified>,
) -> Result<String> {
    let quote = report["intel_quote"]
        .as_str()
        .ok_or_else(|| anyhow!("no Intel quote"))?;
    let verified = verify_quote(&crate::nearai::bytes(quote)?)?;
    if !MODEL_TCB.contains(&verified.status.as_str()) {
        let advisories = if verified.advisory_ids.is_empty() {
            "no advisories".to_string()
        } else {
            verified.advisory_ids.join(", ")
        };
        bail!("TCB status {} not accepted ({advisories})", verified.status);
    }
    if verified.debug {
        bail!("TDX debug mode is enabled");
    }
    if report["signing_algo"].as_str() != Some("ed25519") {
        bail!("unexpected signing algorithm {}", report["signing_algo"]);
    }
    let signer = report["signing_address"].as_str().unwrap_or("");
    let identity = crate::nearai::bytes(signer).unwrap_or_default();
    if identity.len() != 32 {
        bail!("malformed signing address");
    }
    let expected = [identity, crate::nearai::bytes(nonce)?].concat();
    if verified.report_data.as_slice() != expected.as_slice() {
        bail!("report data does not bind the signer and nonce");
    }
    let tcb_info = parse(&report["info"]["tcb_info"])?;
    let compose = tcb_info["app_compose"]
        .as_str()
        .ok_or_else(|| anyhow!("no compose file attested"))?;
    let mut mr_config_id = [0u8; 48];
    mr_config_id[0] = 1;
    mr_config_id[1..33].copy_from_slice(&Sha256::digest(compose.as_bytes()));
    if verified.mr_config_id != mr_config_id {
        bail!("MRCONFIGID does not match the attested compose file");
    }
    Ok(signer.to_string())
}

fn base64url(text: &str) -> Result<Vec<u8>> {
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text.trim_end_matches('='))?)
}

/// Checks NVIDIA's signature on the verdict, its issuer, its result and our
/// nonce.
pub fn check_nvidia_token(token: &str, nonce: &str, keys: &[Value]) -> Result<()> {
    let mut parts = token.split('.');
    let (Some(header), Some(claims), Some(signature)) = (parts.next(), parts.next(), parts.next())
    else {
        bail!("NVIDIA verdict is not a JWT");
    };
    let head: Value = serde_json::from_slice(&base64url(header)?)?;
    let key = keys.iter().find(|k| k["kid"] == head["kid"]);
    let (Some("ES384"), Some(key)) = (head["alg"].as_str(), key) else {
        bail!("NVIDIA verdict is not signed by a published NVIDIA key");
    };
    // JOSE allows either of an ECDSA signature's two valid S values; NVIDIA uses both.
    let valid = (|| {
        let point = [
            vec![4u8],
            base64url(key["x"].as_str()?).ok()?,
            base64url(key["y"].as_str()?).ok()?,
        ]
        .concat();
        let key = p384::ecdsa::VerifyingKey::from_sec1_bytes(&point).ok()?;
        let signature = p384::ecdsa::Signature::from_slice(&base64url(signature).ok()?).ok()?;
        Some(
            key.verify(format!("{header}.{claims}").as_bytes(), &signature)
                .is_ok(),
        )
    })();
    if valid != Some(true) {
        bail!("NVIDIA verdict has an invalid signature");
    }
    let verdict: Value = serde_json::from_slice(&base64url(claims)?)?;
    if verdict["iss"].as_str() != Some(NRAS) {
        bail!("NVIDIA verdict has the wrong issuer");
    }
    let verdict_nonce = match &verdict["eat_nonce"] {
        Value::String(s) => s.to_lowercase(),
        other => other.to_string(),
    };
    if verdict_nonce != nonce {
        bail!("NVIDIA verdict is for another nonce");
    }
    if verdict["x-nvidia-overall-att-result"] != Value::Bool(true) {
        bail!("NVIDIA did not attest the GPUs");
    }
    Ok(())
}

/// Verify the first model candidate, the one requests are encrypted to and
/// whose key must sign every reply; other candidates never see data.
pub fn verify_attestation(body: &Value, nonce: &str, options: &Options) -> Result<String> {
    let candidate = body["model_attestations"]
        .get(0)
        .ok_or_else(|| anyhow!("no model evidence returned"))?;
    let key = check_report(candidate, nonce, options.verify_quote)?;
    if candidate["nvidia_payload"].is_null() || candidate["nvidia_payload"] == "" {
        bail!("no GPU evidence");
    }
    let token = options
        .nvidia
        .token(&parse(&candidate["nvidia_payload"])?)?;
    check_nvidia_token(&token, nonce, &options.nvidia.keys()?)?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
    use p384::ecdsa::{signature::Signer, Signature, SigningKey};
    use serde_json::json;

    // Real NVIDIA-format JWT signatures; hardware quotes are supplied by an
    // injected verifier so these tests exercise the policy offline.
    struct TestNvidia {
        key: SigningKey,
        claims: Value,
    }
    impl TestNvidia {
        fn new() -> Self {
            Self::with(json!({}))
        }
        /// Overrides merged into otherwise valid verdict claims.
        fn with(claims: Value) -> Self {
            Self {
                key: SigningKey::from_slice(&[7; 48]).unwrap(),
                claims,
            }
        }
    }
    impl Nvidia for TestNvidia {
        fn token(&self, payload: &Value) -> Result<String> {
            let header = B64.encode(json!({"alg":"ES384", "kid":"test"}).to_string());
            let mut claims = json!({"iss": NRAS, "eat_nonce": payload["nonce"],
                "x-nvidia-overall-att-result": true});
            for (k, v) in self.claims.as_object().unwrap() {
                claims[k] = v.clone();
            }
            let text = format!("{header}.{}", B64.encode(claims.to_string()));
            let sig: Signature = self.key.sign(text.as_bytes());
            Ok(format!("{text}.{}", B64.encode(sig.to_bytes())))
        }
        fn keys(&self) -> Result<Vec<Value>> {
            let point = self.key.verifying_key().to_encoded_point(false);
            Ok(vec![
                json!({"kid":"test", "x":B64.encode(point.x().unwrap()), "y":B64.encode(point.y().unwrap())}),
            ])
        }
    }

    const NONCE: &str = "0909090909090909090909090909090909090909090909090909090909090909";
    fn signer(seed: u8) -> String {
        hex::encode(
            ed25519_dalek::SigningKey::from_bytes(&[seed; 32])
                .verifying_key()
                .to_bytes(),
        )
    }
    fn report(seed: u8) -> Value {
        json!({ "intel_quote": hex::encode([seed]), "signing_algo":"ed25519",
            "signing_address":signer(seed), "info":{"tcb_info":{"app_compose":"test compose"}},
            "nvidia_payload":{"nonce":NONCE} })
    }
    /// A valid quote for the report with `seed`; 99 fails verification.
    fn quote(bytes: &[u8]) -> Result<Verified> {
        if bytes == [99] {
            bail!("invalid hardware quote");
        }
        let mut data = [0; 64];
        data[..32].copy_from_slice(&crate::nearai::bytes(&signer(bytes[0]))?);
        data[32..].copy_from_slice(&crate::nearai::bytes(NONCE)?);
        let mut config = [0; 48];
        config[0] = 1;
        config[1..33].copy_from_slice(&Sha256::digest(b"test compose"));
        Ok(Verified {
            status: "UpToDate".into(),
            advisory_ids: vec![],
            report_data: data,
            mr_config_id: config,
            debug: false,
        })
    }
    fn verify(candidates: Vec<Value>, nvidia: &TestNvidia) -> Result<String> {
        let body = json!({ "model_attestations": candidates });
        verify_attestation(
            &body,
            NONCE,
            &Options {
                verify_quote: &quote,
                nvidia,
            },
        )
    }

    #[test]
    fn the_first_candidate_is_verified_and_its_key_returned() {
        let nvidia = TestNvidia::new();
        // Later candidates are never used, so they are not verified.
        assert_eq!(
            verify(vec![report(1), report(99)], &nvidia).unwrap(),
            signer(1)
        );
        assert!(verify(vec![report(99), report(1)], &nvidia).is_err());
        assert!(verify(vec![], &nvidia).is_err());
    }

    #[test]
    fn every_report_and_gpu_check_fails_closed() {
        let nvidia = TestNvidia::new();
        let mut cases: Vec<(&str, Value)> = Vec::new();
        for (name, field, value) in [
            ("signing algorithm", "signing_algo", json!("secp256k1")),
            (
                "signer not bound by the quote",
                "signing_address",
                json!(signer(2)),
            ),
            (
                "compose file",
                "info",
                json!({"tcb_info": {"app_compose": "other"}}),
            ),
            ("no GPU evidence", "nvidia_payload", Value::Null),
        ] {
            let mut r = report(1);
            r[field] = value;
            cases.push((name, r));
        }
        for (name, r) in cases {
            assert!(verify(vec![r], &nvidia).is_err(), "{name}");
        }
        let check = |verified: Verified| {
            check_report(&report(1), NONCE, &move |_: &[u8]| {
                Ok(verified_copy(&verified))
            })
        };
        let valid = quote(&[1]).unwrap();
        assert!(check(verified_copy(&valid)).is_ok());
        assert!(check(Verified {
            status: "OutOfDate".into(),
            ..verified_copy(&valid)
        })
        .is_err());
        assert!(check(Verified {
            debug: true,
            ..verified_copy(&valid)
        })
        .is_err());
        let mut other_nonce = verified_copy(&valid);
        other_nonce.report_data[63] ^= 1;
        assert!(check(other_nonce).is_err());
        for claims in [
            json!({"iss": "https://example.invalid"}),
            json!({"eat_nonce": "00"}),
            json!({"x-nvidia-overall-att-result": false}),
        ] {
            assert!(
                verify(vec![report(1)], &TestNvidia::with(claims.clone())).is_err(),
                "{claims}"
            );
        }
    }

    fn verified_copy(v: &Verified) -> Verified {
        Verified {
            status: v.status.clone(),
            advisory_ids: v.advisory_ids.clone(),
            report_data: v.report_data,
            mr_config_id: v.mr_config_id,
            debug: v.debug,
        }
    }

    #[test]
    fn a_signed_turn_must_match_the_attested_key() {
        let key = verify(vec![report(1)], &TestNvidia::new()).unwrap();
        let text = crate::nearai::sign::signed_text("model", "request", "response");
        for seed in [1, 2] {
            let sig = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]).sign(text.as_bytes());
            let signed = json!({"signature_kind":"provider_tee", "signing_algo":"ed25519",
                "signing_address":signer(seed), "text":text, "signature":hex::encode(sig.to_bytes())});
            let result = crate::nearai::sign::check_signature(&signed, &text, &key);
            assert_eq!(result.is_ok(), seed == 1);
        }
    }
}
