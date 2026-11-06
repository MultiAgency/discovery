// Derived from MultiAgency/private-ai@dd469a5 app/src/attest.rs. Local changes: VENDORED.md.
//! Verifies a NEAR AI Cloud attestation report before any data is sent, per
//! docs.near.ai/cloud/verification: for the gateway and every model candidate,
//! an Intel TDX quote whose report data binds the signing key and our nonce,
//! whose MRCONFIGID is the hash of the attested compose file, and whose RTMR3
//! matches the replayed event log; for the model, NVIDIA's signed verdict on
//! its GPUs for the same nonce. Port of `core/attest.mjs`.
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use dcap_qvl::configs::DefaultConfig;
use dcap_qvl::verify::QuoteVerifier;
use dcap_qvl::QuoteCollateralV3;
use p384::ecdsa::signature::Verifier;
use serde_json::Value;
use sha2::{Digest, Sha256, Sha384};

/// The model reads the data, so its platform must be fully patched, unless the
/// run's policy opts in to a pending update, which every review and receipt
/// then states.
pub const MODEL_TCB: &[&str] = &["UpToDate"];
/// Under E2EE the gateway relays ciphertext only. Its quote must still verify
/// and bind our nonce, but a pending platform patch there does not expose
/// data. This is also the most a model may be allowed: never Revoked or Unknown.
pub const GATEWAY_TCB: &[&str] = &[
    "UpToDate",
    "SWHardeningNeeded",
    "ConfigurationNeeded",
    "ConfigurationAndSWHardeningNeeded",
    "OutOfDate",
    "OutOfDateConfigurationNeeded",
];

pub const NRAS: &str = "https://nras.attestation.nvidia.com";

/// What a quote verifier reports about one TDX quote.
pub struct Verified {
    pub status: String,
    pub advisory_ids: Vec<String>,
    pub report_data: [u8; 64],
    pub mr_config_id: [u8; 48],
    pub rt_mr3: [u8; 48],
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
        rt_mr3: td.rt_mr3,
        debug: td.td_attributes[0] & 1 == 1,
    })
}

/// Where the NVIDIA verdict and keys come from: the attestation service for a
/// live run; for a receipt, the verdict it recorded.
pub trait Nvidia {
    fn token(&self, payload: &Value) -> Result<String>;
    fn keys(&self) -> Result<Vec<Value>>;
}

pub struct Options<'a> {
    pub verify_quote: &'a dyn Fn(&[u8]) -> Result<Verified>,
    pub nvidia: &'a dyn Nvidia,
    pub gpu_token: Option<&'a str>,
    /// Recorded verdict for each candidate, in report order.
    pub gpu_tokens: Option<&'a [String]>,
    pub allow_unpatched_model: bool,
}

/// What the receipt and the review state about one verified report.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Summary {
    pub tcb: String,
    pub advisories: Vec<String>,
    pub signer: String,
    pub compose_hash: String,
}

#[derive(Clone, Debug)]
pub struct Model {
    pub summary: Summary,
    pub gpu_token: String,
    /// The NVIDIA key that signed the verdict, with its certificate chain.
    pub gpu_key: Value,
    pub public_key: String,
}

#[derive(Clone, Debug)]
pub struct Attestation {
    pub gateway: Summary,
    pub model: Model,
    /// All verified preflight candidates; signer identities are unique.
    pub models: Vec<Model>,
}

/// A JSON field that NEAR AI sends either as an object or as a JSON string of one.
fn parse(value: &Value) -> Result<Value> {
    match value {
        Value::String(text) => Ok(serde_json::from_str(text)?),
        other => Ok(other.clone()),
    }
}

pub fn replay_rtmr3(event_log: &Value) -> Result<[u8; 48]> {
    let mut rtmr3 = [0u8; 48];
    for event in parse(event_log)?.as_array().into_iter().flatten() {
        if event["imr"].as_u64() != Some(3) {
            continue;
        }
        let mut hasher = Sha384::new();
        hasher.update((event["event_type"].as_u64().unwrap_or(0) as u32).to_le_bytes());
        hasher.update(b":");
        hasher.update(event["event"].as_str().unwrap_or("").as_bytes());
        hasher.update(b":");
        hasher.update(crate::bytes(event["event_payload"].as_str().unwrap_or(""))?);
        let digest = hasher.finalize();
        if let Some(recorded) = event["digest"].as_str().filter(|d| !d.is_empty()) {
            if crate::bytes(recorded)? != digest.as_slice() {
                bail!(
                    "event log digest mismatch for {}",
                    event["event"].as_str().unwrap_or("")
                );
            }
        }
        let mut extend = Sha384::new();
        extend.update(rtmr3);
        extend.update(digest);
        rtmr3.copy_from_slice(&extend.finalize());
    }
    Ok(rtmr3)
}

/// The TDX checks on one report; returns what the receipt records.
pub fn check_report(
    report: &Value,
    nonce: &str,
    accepted: &[&str],
    verify_quote: &dyn Fn(&[u8]) -> Result<Verified>,
) -> Result<Summary> {
    let quote = report["intel_quote"]
        .as_str()
        .ok_or_else(|| anyhow!("no Intel quote"))?;
    let verified = verify_quote(&crate::bytes(quote)?)?;
    if !accepted.contains(&verified.status.as_str()) {
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
    let identity = crate::bytes(signer).unwrap_or_default();
    if identity.len() != 32 {
        bail!("malformed signing address");
    }
    let bound = match report["tls_cert_fingerprint"]
        .as_str()
        .filter(|f| !f.is_empty())
    {
        Some(fingerprint) => Sha256::new()
            .chain_update(&identity)
            .chain_update(crate::bytes(fingerprint)?)
            .finalize()
            .to_vec(),
        None => identity,
    };
    let expected: Vec<u8> = [bound, crate::bytes(nonce)?].concat();
    if verified.report_data.as_slice() != expected.as_slice() {
        bail!("report data does not bind the signer and nonce");
    }
    if report["request_nonce"]
        .as_str()
        .map(str::to_lowercase)
        .as_deref()
        != Some(nonce)
    {
        bail!("echoed nonce does not match");
    }

    let tcb_info = parse(&report["info"]["tcb_info"])?;
    let compose = tcb_info["app_compose"]
        .as_str()
        .ok_or_else(|| anyhow!("no compose file attested"))?;
    let compose_hash = Sha256::digest(compose.as_bytes());
    let mut mr_config_id = [0u8; 48];
    mr_config_id[0] = 1;
    mr_config_id[1..33].copy_from_slice(&compose_hash);
    if verified.mr_config_id != mr_config_id {
        bail!("MRCONFIGID does not match the attested compose file");
    }
    if replay_rtmr3(&report["event_log"])? != verified.rt_mr3 {
        bail!("event log does not match RTMR3");
    }

    Ok(Summary {
        tcb: verified.status,
        advisories: verified.advisory_ids,
        signer: signer.to_string(),
        compose_hash: hex::encode(compose_hash),
    })
}

fn base64url(text: &str) -> Result<Vec<u8>> {
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text.trim_end_matches('='))?)
}

/// Checks NVIDIA's signature on the verdict, its issuer, its result and our
/// nonce. Returns the key that signed it, for the receipt to keep.
pub fn check_nvidia_token(token: &str, nonce: &str, keys: &[Value]) -> Result<Value> {
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
    Ok(key.clone())
}

fn check_model(
    candidate: &Value,
    nonce: &str,
    options: &Options,
    gpu_token: Option<&str>,
) -> Result<Model> {
    let accepted = if options.allow_unpatched_model {
        GATEWAY_TCB
    } else {
        MODEL_TCB
    };
    let summary = check_report(candidate, nonce, accepted, options.verify_quote)?;
    if candidate["nvidia_payload"].is_null() || candidate["nvidia_payload"] == "" {
        bail!("no GPU evidence");
    }
    let payload = parse(&candidate["nvidia_payload"])?;
    let payload_nonce = match &payload["nonce"] {
        Value::String(s) => s.to_lowercase(),
        other => other.to_string(),
    };
    if payload_nonce != nonce {
        bail!("GPU evidence does not bind the nonce");
    }
    let token = match gpu_token {
        Some(token) => token.to_string(),
        None => options.nvidia.token(&payload)?,
    };
    let gpu_key = check_nvidia_token(&token, nonce, &options.nvidia.keys()?)?;
    let public_key = candidate["signing_public_key"].as_str().unwrap_or("");
    if public_key.to_lowercase() != summary.signer.to_lowercase() {
        bail!("encryption key is not the attested signing key");
    }
    Ok(Model {
        summary,
        gpu_token: token,
        gpu_key,
        public_key: public_key.to_string(),
    })
}

/// Verify every candidate before inference, rejecting ambiguous signer
/// identities. The first verified model is the key selected for E2EE; signed
/// turns must match that key, which has exactly one retained preflight report.
pub fn verify_attestation(body: &Value, nonce: &str, options: &Options) -> Result<Attestation> {
    let gateway = check_report(
        &body["gateway_attestation"],
        nonce,
        GATEWAY_TCB,
        options.verify_quote,
    )
    .map_err(|e| anyhow!("gateway: {e}"))?;
    let candidates = body["model_attestations"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if candidates.is_empty() {
        bail!("no model evidence returned");
    }
    if let Some(tokens) = options.gpu_tokens {
        if tokens.len() != candidates.len() {
            bail!("recorded GPU verdict count does not match model evidence");
        }
    } else if options.gpu_token.is_some() && candidates.len() != 1 {
        bail!("multiple model candidates require separate recorded GPU verdicts");
    }
    let mut models: Vec<Model> = Vec::with_capacity(candidates.len());
    for (index, candidate) in candidates.iter().enumerate() {
        let token = options
            .gpu_tokens
            .map(|tokens| tokens[index].as_str())
            .or(options.gpu_token);
        let model = check_model(candidate, nonce, options, token)
            .map_err(|e| anyhow!("model candidate {index}: {e}"))?;
        if models
            .iter()
            .any(|m| m.public_key.eq_ignore_ascii_case(&model.public_key))
        {
            bail!("ambiguous model signer: more than one preflight report matches");
        }
        models.push(model);
    }
    Ok(Attestation {
        gateway,
        model: models[0].clone(),
        models,
    })
}

/// A recorded set of collateral, tried in turn: what checking a receipt offline uses.
pub fn verify_with_recorded(
    quote: &[u8],
    collateral: &[QuoteCollateralV3],
    now: u64,
) -> Result<Verified> {
    let mut last = anyhow!("no collateral recorded");
    for c in collateral {
        match verify_quote(quote, c, now) {
            Ok(verified) => return Ok(verified),
            Err(e) => last = e,
        }
    }
    Err(last).context("quote")
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
    use p384::ecdsa::{signature::Signer, Signature, SigningKey};
    use serde_json::json;
    use std::cell::Cell;

    // Real NVIDIA-format JWT signatures; hardware quotes are supplied by an
    // injected verifier so these tests exercise candidate policy offline.
    struct TestNvidia {
        key: SigningKey,
    }
    impl TestNvidia {
        fn new() -> Self {
            Self {
                key: SigningKey::from_slice(&[7; 48]).unwrap(),
            }
        }
    }
    impl Nvidia for TestNvidia {
        fn token(&self, payload: &Value) -> Result<String> {
            let header = B64.encode(json!({"alg":"ES384", "kid":"test"}).to_string());
            let claims = B64.encode(
                json!({"iss": NRAS, "eat_nonce": payload["nonce"],
                "x-nvidia-overall-att-result": true})
                .to_string(),
            );
            let text = format!("{header}.{claims}");
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
            "signing_address":signer(seed), "signing_public_key":signer(seed),
            "request_nonce":NONCE, "report_data":format!("{}{}",signer(seed),NONCE),
            "info":{"tcb_info":{"app_compose":"test compose"}}, "event_log":[],
            "nvidia_payload":{"nonce":NONCE} })
    }
    fn quote(bytes: &[u8]) -> Result<Verified> {
        if bytes == [99] {
            bail!("invalid hardware quote");
        }
        let mut data = [0; 64];
        data[..32].copy_from_slice(&crate::bytes(&signer(bytes[0]))?);
        data[32..].copy_from_slice(&crate::bytes(NONCE)?);
        let mut config = [0; 48];
        config[0] = 1;
        config[1..33].copy_from_slice(&Sha256::digest(b"test compose"));
        Ok(Verified {
            status: "UpToDate".into(),
            advisory_ids: vec![],
            report_data: data,
            mr_config_id: config,
            rt_mr3: [0; 48],
            debug: false,
        })
    }
    fn body(candidates: Vec<Value>) -> Value {
        json!({"gateway_attestation":report(10), "model_attestations":candidates})
    }

    #[test]
    fn a_bad_candidate_is_rejected_even_beside_a_valid_candidate() {
        let nvidia = TestNvidia::new();
        let options = Options {
            verify_quote: &quote,
            nvidia: &nvidia,
            gpu_token: None,
            gpu_tokens: None,
            allow_unpatched_model: false,
        };
        for candidates in [vec![report(1), report(99)], vec![report(99), report(1)]] {
            assert!(verify_attestation(&body(candidates), NONCE, &options).is_err());
        }
    }

    #[test]
    fn every_candidate_is_checked_before_inference() {
        let calls = Cell::new(0);
        let verify = |bytes: &[u8]| {
            calls.set(calls.get() + 1);
            quote(bytes)
        };
        let nvidia = TestNvidia::new();
        let options = Options {
            verify_quote: &verify,
            nvidia: &nvidia,
            gpu_token: None,
            gpu_tokens: None,
            allow_unpatched_model: false,
        };
        assert!(verify_attestation(&body(vec![report(1), report(2)]), NONCE, &options).is_ok());
        assert_eq!(calls.get(), 3); // gateway plus both models
    }

    #[test]
    fn ambiguous_model_signers_are_rejected() {
        let nvidia = TestNvidia::new();
        let options = Options {
            verify_quote: &quote,
            nvidia: &nvidia,
            gpu_token: None,
            gpu_tokens: None,
            allow_unpatched_model: false,
        };
        assert!(verify_attestation(&body(vec![report(1), report(1)]), NONCE, &options).is_err());
    }

    #[test]
    fn recorded_evidence_requires_one_verdict_per_candidate() {
        let nvidia = TestNvidia::new();
        let b = body(vec![report(1), report(2)]);
        let live = Options {
            verify_quote: &quote,
            nvidia: &nvidia,
            gpu_token: None,
            gpu_tokens: None,
            allow_unpatched_model: false,
        };
        let checked = verify_attestation(&b, NONCE, &live).unwrap();
        assert_eq!(checked.models.len(), 2);
        assert_eq!(checked.model.public_key, signer(1));
        let tokens: Vec<_> = checked.models.iter().map(|m| m.gpu_token.clone()).collect();
        let mut recorded = Options {
            verify_quote: &quote,
            nvidia: &nvidia,
            gpu_token: Some(&tokens[0]),
            gpu_tokens: None,
            allow_unpatched_model: false,
        };
        assert!(verify_attestation(&b, NONCE, &recorded).is_err());
        recorded.gpu_tokens = Some(&tokens[..1]);
        assert!(verify_attestation(&b, NONCE, &recorded).is_err());
        recorded.gpu_tokens = Some(&tokens);
        assert!(verify_attestation(&b, NONCE, &recorded).is_ok());
        let mut bad = tokens.clone();
        bad[1] = "invalid JWT".into();
        recorded.gpu_tokens = Some(&bad);
        assert!(verify_attestation(&b, NONCE, &recorded).is_err());
    }

    #[test]
    fn a_signed_turn_must_match_the_uniquely_selected_preflight_key() {
        let nvidia = TestNvidia::new();
        let options = Options {
            verify_quote: &quote,
            nvidia: &nvidia,
            gpu_token: None,
            gpu_tokens: None,
            allow_unpatched_model: false,
        };
        let checked =
            verify_attestation(&body(vec![report(1), report(2)]), NONCE, &options).unwrap();
        let text = crate::sign::signed_text("model", "request", "response");
        for seed in [1, 2] {
            let sig = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]).sign(text.as_bytes());
            let signed = json!({"signature_kind":"provider_tee", "signing_algo":"ed25519",
                "signing_address":signer(seed), "text":text, "signature":hex::encode(sig.to_bytes())});
            let result = crate::sign::check_signature(&signed, &text, &checked.model.public_key);
            assert_eq!(result.is_ok(), seed == 1);
        }
    }
}
