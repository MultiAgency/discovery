//! InitSurvey, Submit and Status.
//!
//! Storage layout (raw records, every value sealed under the project key):
//! - `s/{id}/def`: the canonical survey definition
//! - `s/{id}/key`: X25519 secret, replaced with an empty marker after release
//! - `s/{id}/collection`: admission state and immutable registration-list head
//! - `s/{id}/report`: the released report
//! - `s/{id}/receipt/{hash}`: the report's receipt (public, stored unsealed)
//! - `s/{id}/r/{hex mac(id:identity)}`: open/frozen response slot, then empty tombstone
//! - `s/{id}/i/{random}`: sealed registration metadata, no response content
//!
//! `{id}` is the hex definition hash, which is public. Response keys reveal
//! nothing about who responded without the project key.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use discovery_protocol::definition::SurveyDefinition;
use discovery_protocol::envelope::SurveyKey;
use discovery_protocol::identity::Submission;
use discovery_protocol::report::Report;
use discovery_protocol::{Error, Result};
use serde::Serialize;

use crate::collection::{self, Slot};
use crate::platform::{Mode, Platform, Records};

pub fn def_key(id: &str) -> String {
    format!("s/{id}/def")
}
pub fn secret_key(id: &str) -> String {
    format!("s/{id}/key")
}
pub fn collection_key(id: &str) -> String {
    format!("s/{id}/collection")
}
pub fn report_key(id: &str) -> String {
    format!("s/{id}/report")
}
pub fn receipt_key(id: &str, hash: &str) -> String {
    format!("s/{id}/receipt/{hash}")
}
pub fn responses_prefix(id: &str) -> String {
    format!("s/{id}/r/")
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Created {
    pub survey_id: String,
    /// Hex X25519 public key respondents encrypt to.
    pub public_key: String,
    pub deadline_ns: u64,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Status {
    pub survey_id: String,
    pub definition: SurveyDefinition,
    pub closed: bool,
    /// Present while the survey accepts responses; secret material is erased at release.
    pub public_key: Option<String>,
    /// Present once `Finalize` has released it.
    pub report: Option<Report>,
}

/// Fixed output: no hint whether the submission was new or an edit.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Accepted {
    pub accepted: bool,
}

/// On-chain only, so the deadline is checked against trusted time and a
/// published payment key can't create surveys.
pub fn init<P: Platform>(p: &P, definition: SurveyDefinition) -> Result<Created> {
    let Mode::Near { block_ns } = p.mode()? else {
        return Err(Error("init_survey runs on-chain only".into()));
    };
    definition.validate()?;
    if definition.deadline_ns <= block_ns {
        return Err(Error("deadline is in the past".into()));
    }
    let id = hex::encode(definition.hash()?);
    let records = Records(p);
    records.set_if_absent(&def_key(&id), &definition.canonical_bytes()?)?;
    records.set_if_absent(&secret_key(&id), &SurveyKey::generate()?.to_bytes())?;
    collection::init(p, &id)?;
    let key = load_key(p, &id)?;
    Ok(Created {
        survey_id: id,
        public_key: hex::encode(key.public()),
        deadline_ns: definition.deadline_ns,
    })
}

pub fn submit<P: Platform>(p: &P, survey_id: &str, envelope_b64: &str) -> Result<Accepted> {
    let definition = load_definition(p, survey_id)?;
    if collection::is_closed(p, survey_id)? {
        return Err(Error("survey is closed".into()));
    }
    if let Mode::Near { block_ns } = p.mode()? {
        if block_ns >= definition.deadline_ns {
            return Err(Error("survey is closed".into()));
        }
    }
    let envelope = B64
        .decode(envelope_b64)
        .map_err(|_| Error("envelope is not base64".into()))?;
    let hash = definition.hash()?;
    let plaintext = load_key(p, survey_id)?.open(&envelope, &hash)?;

    // Past this point everything derives from plaintext. Errors leave the
    // enclave through OutLayer's front end, so they never echo it.
    let submission: Submission =
        serde_json::from_slice(&plaintext).map_err(|_| rejected("the submission is malformed"))?;
    if submission.anchor.is_some() {
        return Err(rejected(
            "wallet anchoring is not supported by this release",
        ));
    }
    let identity = submission
        .verify()
        .map_err(|_| rejected("the signature does not verify"))?;
    submission
        .response
        .validate(&definition)
        .map_err(|_| rejected("the response does not match the survey"))?;

    let tag = p.mac(format!("{survey_id}:{}", identity.name()).as_bytes())?;
    let key = format!("{}{}", responses_prefix(survey_id), hex::encode(tag));
    collection::register(p, survey_id, &key)?;
    let sent_at_ms = submission.response.sent_at_ms;
    let slot = Slot::Open { submission };
    let updated = collection::seal_slot(p, &key, &slot)?;
    for _ in 0..5 {
        if collection::is_closed(p, survey_id)? {
            return Err(Error("survey is closed".into()));
        }
        let changed = match p.get(&key)? {
            Some(stored) => {
                let Slot::Open {
                    submission: previous,
                } = collection::read_slot(p, &key, &stored)?
                else {
                    return Err(Error("survey is closed".into()));
                };
                if previous.response.sent_at_ms >= sent_at_ms {
                    return Ok(Accepted { accepted: true });
                }
                p.compare_and_swap(&key, &stored, &updated)?
            }
            None => p.set_if_absent(&key, &updated)?,
        };
        if changed {
            return Ok(Accepted { accepted: true });
        }
        // Another run won. Re-read and compare timestamps before retrying.
    }
    Err(Error(
        "response changed repeatedly; retry submission".into(),
    ))
}

fn rejected(reason: &str) -> Error {
    Error(format!("submission rejected: {reason}"))
}

pub fn status<P: Platform>(p: &P, survey_id: &str) -> Result<Status> {
    let records = Records(p);
    let definition = load_definition(p, survey_id)?;
    let public_key = match records.get(&secret_key(survey_id))? {
        Some(bytes) if bytes.is_empty() => None,
        Some(_) => Some(hex::encode(load_key(p, survey_id)?.public())),
        None => None,
    };
    let report = match records.get(&report_key(survey_id))? {
        Some(bytes) => Some(serde_json::from_slice(&bytes)?),
        None => None,
    };
    Ok(Status {
        survey_id: survey_id.into(),
        definition,
        closed: collection::is_closed(p, survey_id)?,
        public_key,
        report,
    })
}

/// The receipt text, once `Finalize` has written one.
pub fn receipt<P: Platform>(p: &P, survey_id: &str) -> Result<String> {
    let report: Report =
        serde_json::from_slice(&Records(p).require(&report_key(survey_id), "report")?)?;
    let hash = report
        .receipt_sha256
        .ok_or_else(|| Error("no receipt for this survey".into()))?;
    let bytes = p
        .get(&receipt_key(survey_id, &hash))?
        .ok_or_else(|| Error("report receipt is missing".into()))?;
    if hex::encode(discovery_protocol::sha256_bytes(&bytes)) != hash {
        return Err(Error("receipt does not match the report".into()));
    }
    String::from_utf8(bytes).map_err(|_| Error("receipt is not UTF-8".into()))
}

pub fn load_definition<P: Platform>(p: &P, id: &str) -> Result<SurveyDefinition> {
    let bytes = Records(p).require(&def_key(id), "survey")?;
    let definition: SurveyDefinition = serde_json::from_slice(&bytes)?;
    if hex::encode(definition.hash()?) != id {
        return Err(Error("stored definition does not match its id".into()));
    }
    Ok(definition)
}

pub fn load_key<P: Platform>(p: &P, id: &str) -> Result<SurveyKey> {
    let bytes = Records(p).require(&secret_key(id), "survey key")?;
    let secret: [u8; 32] = bytes
        .try_into()
        .map_err(|_| Error("survey key is not 32 bytes".into()))?;
    Ok(SurveyKey::from_bytes(secret))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::memory::Memory;
    use discovery_protocol::definition::{
        DisclosureRules, Question, SurveyKind, Target, DEFINITION_VERSION,
    };
    use discovery_protocol::envelope::seal;
    use discovery_protocol::identity::RespondentKey;
    use discovery_protocol::response::{Answer, Basis, Disclosure, Response};

    const NOW: u64 = 1_000;

    fn definition() -> SurveyDefinition {
        SurveyDefinition {
            version: DEFINITION_VERSION,
            kind: SurveyKind::IssueSurvey,
            title: "t".into(),
            target: Target {
                repo: "MultiAgency/discovery".into(),
                number: 1,
            },
            deadline_ns: NOW + 100,
            questions: vec![Question {
                id: "q".into(),
                prompt: "p".into(),
            }],
            rules: DisclosureRules::default(),
        }
    }

    fn envelope(created: &Created, d: &SurveyDefinition, secret: u8, text: &str) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CLOCK: AtomicU64 = AtomicU64::new(1);
        envelope_at(
            created,
            d,
            secret,
            text,
            CLOCK.fetch_add(1, Ordering::Relaxed),
        )
    }

    fn envelope_at(
        created: &Created,
        d: &SurveyDefinition,
        secret: u8,
        text: &str,
        sent_at_ms: u64,
    ) -> String {
        let hash = d.hash().unwrap();
        let response = Response {
            definition_hash: hex::encode(hash),
            sent_at_ms,
            answers: vec![Answer {
                question_id: "q".into(),
                basis: Basis::Confirmed,
                disclosure: Disclosure::Publishable,
                text: text.into(),
                evidence: vec![],
                repro_steps: vec![],
            }],
        };
        let sub = RespondentKey::for_survey(&[secret; 32], &hash)
            .unwrap()
            .sign(response)
            .unwrap();
        let pk: [u8; 32] = hex::decode(&created.public_key)
            .unwrap()
            .try_into()
            .unwrap();
        B64.encode(seal(&pk, &serde_json::to_vec(&sub).unwrap(), &hash).unwrap())
    }

    fn responses(m: &Memory, id: &str) -> usize {
        m.store
            .borrow()
            .keys()
            .filter(|k| k.starts_with(&responses_prefix(id)))
            .count()
    }

    #[test]
    fn init_is_on_chain_only_and_idempotent() {
        let https = Memory::new(Mode::Https);
        assert!(init(&https, definition()).is_err());
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let a = init(&m, definition()).unwrap();
        let b = init(&m, definition()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn init_refuses_past_deadline() {
        let m = Memory::new(Mode::Near {
            block_ns: NOW + 100,
        });
        assert!(init(&m, definition()).is_err());
    }

    #[test]
    fn one_slot_per_respondent_and_edits_replace() {
        let d = definition();
        let chain = Memory::new(Mode::Near { block_ns: NOW });
        let created = init(&chain, d.clone()).unwrap();
        let id = created.survey_id.clone();
        submit(&chain, &id, &envelope(&created, &d, 1, "first")).unwrap();
        submit(&chain, &id, &envelope(&created, &d, 1, "edited")).unwrap();
        submit(&chain, &id, &envelope(&created, &d, 2, "other")).unwrap();
        assert_eq!(responses(&chain, &id), 2);
    }

    #[test]
    fn closed_and_past_deadline_refuse() {
        let d = definition();
        let chain = Memory::new(Mode::Near { block_ns: NOW });
        let created = init(&chain, d.clone()).unwrap();
        let env = envelope(&created, &d, 1, "x");

        let late = Memory {
            mode: Mode::Near {
                block_ns: NOW + 100,
            },
            store: chain.store.clone(),
        };
        assert!(submit(&late, &created.survey_id, &env).is_err());

        collection::close(&chain, &created.survey_id).unwrap();
        let https = Memory {
            mode: Mode::Https,
            store: chain.store.clone(),
        };
        assert!(submit(&https, &created.survey_id, &env).is_err());
    }

    #[test]
    fn foreign_survey_envelope_refused() {
        let d = definition();
        let mut other = definition();
        other.title = "other".into();
        let chain = Memory::new(Mode::Near { block_ns: NOW });
        let a = init(&chain, d.clone()).unwrap();
        let b = init(&chain, other.clone()).unwrap();
        // An envelope for survey b, submitted to survey a.
        assert!(submit(&chain, &a.survey_id, &envelope(&b, &other, 1, "x")).is_err());
    }

    #[test]
    fn storage_keys_do_not_name_respondents() {
        let d = definition();
        let chain = Memory::new(Mode::Near { block_ns: NOW });
        let created = init(&chain, d.clone()).unwrap();
        submit(&chain, &created.survey_id, &envelope(&created, &d, 1, "x")).unwrap();
        let hash = d.hash().unwrap();
        let signer = RespondentKey::for_survey(&[1; 32], &hash)
            .unwrap()
            .public_key();
        for k in chain.store.borrow().keys() {
            assert!(!k.contains(&signer) && !k.contains("key:"), "{k}");
        }
    }

    /// Finding A: errors after decryption must not echo plaintext, because
    /// they leave the enclave through OutLayer's front end.
    #[test]
    fn post_decryption_errors_do_not_echo_plaintext() {
        let d = definition();
        let chain = Memory::new(Mode::Near { block_ns: NOW });
        let created = init(&chain, d.clone()).unwrap();
        let hash = d.hash().unwrap();
        let pk: [u8; 32] = hex::decode(&created.public_key)
            .unwrap()
            .try_into()
            .unwrap();
        let secret = "SECRET-PLAINTEXT-7731";
        for plaintext in [
            format!(
                r#"{{"response":{{"definition_hash":"x","answers":[{{"question_id":"q","basis":"{secret}","text":"t"}}]}}}}"#
            ),
            format!(r#"{{"not json {secret}"#),
        ] {
            let env = B64.encode(seal(&pk, plaintext.as_bytes(), &hash).unwrap());
            let e = submit(&chain, &created.survey_id, &env).unwrap_err();
            assert!(!e.0.contains(secret), "{}", e.0);
        }
        // A validly signed response with an unknown question id.
        let sub = {
            let response = Response {
                definition_hash: hex::encode(hash),
                sent_at_ms: 1,
                answers: vec![Answer {
                    question_id: secret.into(),
                    basis: Basis::Confirmed,
                    disclosure: Disclosure::Publishable,
                    text: "t".into(),
                    evidence: vec![],
                    repro_steps: vec![],
                }],
            };
            RespondentKey::for_survey(&[1; 32], &hash)
                .unwrap()
                .sign(response)
                .unwrap()
        };
        let env = B64.encode(seal(&pk, &serde_json::to_vec(&sub).unwrap(), &hash).unwrap());
        let e = submit(&chain, &created.survey_id, &env).unwrap_err();
        assert!(!e.0.contains(secret), "{}", e.0);
    }

    /// Finding C: an older submission replayed later must not replace a newer one.
    #[test]
    fn replayed_older_submission_is_refused() {
        let d = definition();
        let chain = Memory::new(Mode::Near { block_ns: NOW });
        let created = init(&chain, d.clone()).unwrap();
        let old = envelope_at(&created, &d, 1, "first", 100);
        let new = envelope_at(&created, &d, 1, "edited", 200);
        submit(&chain, &created.survey_id, &old).unwrap();
        submit(&chain, &created.survey_id, &new).unwrap();
        // Replays get the same reply as success but change nothing.
        assert_eq!(
            submit(&chain, &created.survey_id, &old).unwrap(),
            Accepted { accepted: true }
        );
        assert_eq!(
            submit(&chain, &created.survey_id, &new).unwrap(),
            Accepted { accepted: true }
        );
        let store = chain.store.borrow();
        let stored: Vec<_> = store
            .iter()
            .filter(|(k, _)| k.starts_with(&responses_prefix(&created.survey_id)))
            .collect();
        assert_eq!(stored.len(), 1);
        let sub: Submission = collection::test_submission(&chain, stored[0].0);
        assert_eq!(sub.response.answers[0].text, "edited");
    }

    #[test]
    fn status_reports_key_and_definition() {
        let d = definition();
        let chain = Memory::new(Mode::Near { block_ns: NOW });
        let created = init(&chain, d.clone()).unwrap();
        let s = status(
            &Memory {
                mode: Mode::Https,
                store: chain.store.clone(),
            },
            &created.survey_id,
        )
        .unwrap();
        assert_eq!(s.public_key, Some(created.public_key));
        assert_eq!(s.definition, d);
        assert!(!s.closed && s.report.is_none());
    }

    // D10: the deadline is the earliest finalization time, not an HTTPS cutoff.
    #[test]
    fn https_accepts_after_deadline_until_finalization_closes_collection() {
        let d = definition();
        let mut m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        let env = envelope(&c, &d, 1, "late response");
        m.mode = Mode::Near {
            block_ns: d.deadline_ns,
        };
        assert!(submit(&m, &c.survey_id, &env).is_err());
        m.mode = Mode::Https;
        assert!(submit(&m, &c.survey_id, &env).unwrap().accepted);
        collection::close(&m, &c.survey_id).unwrap();
        assert!(submit(&m, &c.survey_id, &env).is_err());
    }

    #[test]
    fn interleaved_edit_never_overwrites_a_newer_response() {
        use std::cell::Cell;
        struct Racing<'a> {
            inner: &'a Memory,
            id: &'a str,
            newer: String,
            fired: Cell<bool>,
        }
        impl Platform for Racing<'_> {
            fn mode(&self) -> Result<Mode> {
                self.inner.mode()
            }
            fn encrypt(&self, p: &[u8], a: &[u8]) -> Result<Vec<u8>> {
                self.inner.encrypt(p, a)
            }
            fn decrypt(&self, c: &[u8], a: &[u8]) -> Result<Vec<u8>> {
                self.inner.decrypt(c, a)
            }
            fn mac(&self, d: &[u8]) -> Result<Vec<u8>> {
                self.inner.mac(d)
            }
            fn get(&self, k: &str) -> Result<Option<Vec<u8>>> {
                let old = self.inner.get(k)?;
                if k.starts_with(&responses_prefix(self.id)) && !self.fired.replace(true) {
                    submit(self.inner, self.id, &self.newer)?;
                }
                Ok(old)
            }
            fn set(&self, k: &str, v: &[u8]) -> Result<()> {
                self.inner.set(k, v)
            }
            fn set_if_absent(&self, k: &str, v: &[u8]) -> Result<bool> {
                self.inner.set_if_absent(k, v)
            }
            fn compare_and_swap(&self, k: &str, e: &[u8], v: &[u8]) -> Result<bool> {
                self.inner.compare_and_swap(k, e, v)
            }
            fn list(&self, p: &str) -> Result<Vec<String>> {
                self.inner.list(p)
            }
            fn delete(&self, k: &str) -> Result<bool> {
                self.inner.delete(k)
            }
        }
        for exists in [false, true] {
            for (outer, inner) in [(200, 300), (300, 200)] {
                let d = definition();
                let m = Memory::new(Mode::Near { block_ns: NOW });
                let c = init(&m, d.clone()).unwrap();
                if exists {
                    submit(&m, &c.survey_id, &envelope_at(&c, &d, 1, "original", 100)).unwrap();
                }
                let racing = Racing {
                    inner: &m,
                    id: &c.survey_id,
                    newer: envelope_at(&c, &d, 1, "interleaved", inner),
                    fired: Cell::new(false),
                };
                submit(
                    &racing,
                    &c.survey_id,
                    &envelope_at(&c, &d, 1, "outer", outer),
                )
                .unwrap();
                let key = m
                    .list(&responses_prefix(&c.survey_id))
                    .unwrap()
                    .pop()
                    .unwrap();
                let stored: Submission = collection::test_submission(&m, &key);
                assert_eq!(stored.response.sent_at_ms, 300);
                assert_eq!(
                    stored.response.answers[0].text,
                    if outer == 300 { "outer" } else { "interleaved" }
                );
            }
        }
    }

    #[test]
    fn a_delayed_first_submission_cannot_recreate_data_after_release() {
        use crate::platform::interleaving::View;
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let created = init(&m, d.clone()).unwrap();
        let env = envelope(&created, &d, 1, "delayed");
        let mut submitting = View::new(&m, Mode::Https);
        submitting.prefix = responses_prefix(&created.survey_id);
        *submitting.before_write.borrow_mut() = Some(Box::new(|| {
            let finalizing = View::new(
                &m,
                Mode::Near {
                    block_ns: NOW + 100,
                },
            );
            crate::finalize::finalize(
                &finalizing,
                &crate::confidential::Confidential::new(),
                &created.survey_id,
            )
            .unwrap();
        }));
        assert!(submit(&submitting, &created.survey_id, &env).is_err());
        assert!(load_key(&m, &created.survey_id).is_err());
    }

    #[test]
    fn a_delayed_edit_cannot_cross_freeze_or_restore_discarded_data() {
        use crate::platform::interleaving::View;
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        submit(&m, &c.survey_id, &envelope_at(&c, &d, 1, "original", 100)).unwrap();
        let mut delayed = View::new(&m, Mode::Https);
        delayed.prefix = responses_prefix(&c.survey_id);
        *delayed.before_write.borrow_mut() = Some(Box::new(|| {
            let frozen = collection::freeze(&m, &c.survey_id).unwrap();
            assert_eq!(frozen.len(), 1);
            assert_eq!(frozen[0].response.answers[0].text, "original");
            collection::discard(&m, &c.survey_id).unwrap();
        }));
        assert!(submit(
            &delayed,
            &c.survey_id,
            &envelope_at(&c, &d, 1, "delayed", 200)
        )
        .is_err());
        collection::assert_discarded(&m, &c.survey_id);
    }

    #[test]
    fn freeze_retries_after_partial_storage_failure_without_a_lock() {
        use crate::platform::interleaving::View;
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        for identity in [1, 2] {
            submit(
                &m,
                &c.survey_id,
                &envelope_at(&c, &d, identity, "response", 100),
            )
            .unwrap();
        }
        let mut failing = View::new(
            &m,
            Mode::Near {
                block_ns: NOW + 100,
            },
        );
        let slots = m.list(&responses_prefix(&c.survey_id)).unwrap();
        failing.prefix = slots[1].clone();
        failing.failures.set(1);
        assert!(collection::freeze(&failing, &c.survey_id).is_err());
        assert!(collection::is_closed(&m, &c.survey_id).unwrap());
        let first = m.get(&slots[0]).unwrap().unwrap();
        assert!(matches!(
            collection::read_slot(&m, &slots[0], &first).unwrap(),
            Slot::Frozen { .. }
        ));
        let frozen = collection::freeze(&m, &c.survey_id).unwrap();
        assert_eq!(frozen.len(), 2);
        assert_eq!(collection::freeze(&m, &c.survey_id).unwrap().len(), 2);
        assert!(submit(&m, &c.survey_id, &envelope_at(&c, &d, 3, "late", 100)).is_err());
    }

    #[test]
    fn frozen_empty_slots_and_secret_markers_cannot_be_recreated() {
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        let key = format!("{}admitted", responses_prefix(&c.survey_id));
        collection::register(&m, &c.survey_id, &key).unwrap();
        assert!(collection::freeze(&m, &c.survey_id).unwrap().is_empty());
        collection::discard(&m, &c.survey_id).unwrap();
        assert!(!Records(&m)
            .set_if_absent(&secret_key(&c.survey_id), &[7; 32])
            .unwrap());
        assert!(!m.set_if_absent(&key, b"delayed response").unwrap());
        assert!(init(&m, d).is_err());
        collection::assert_discarded(&m, &c.survey_id);
    }

    #[test]
    fn slot_phases_are_authenticated_and_keep_the_full_response_size_limit() {
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        submit(&m, &c.survey_id, &envelope_at(&c, &d, 1, "original", 100)).unwrap();
        let key = m
            .list(&responses_prefix(&c.survey_id))
            .unwrap()
            .pop()
            .unwrap();
        let mut submission = collection::test_submission(&m, &key);
        let max = discovery_protocol::envelope::BUCKETS[3] - 4;
        let current = serde_json::to_vec(&submission).unwrap().len();
        submission.response.answers[0]
            .text
            .push_str(&"x".repeat(max - current));
        let slot = Slot::Open { submission };
        let raw = collection::seal_slot(&m, &key, &slot).unwrap();
        assert_eq!(
            m.decrypt(&raw[1..], format!("{key}:1").as_bytes())
                .unwrap()
                .len(),
            max
        );
        let mut tampered = raw.clone();
        tampered[0] = 2;
        assert!(collection::read_slot(&m, &key, &tampered).is_err());
        assert!(collection::read_slot(&m, "other slot", &raw).is_err());
        assert!(matches!(
            collection::read_slot(&m, &key, &raw).unwrap(),
            Slot::Open { .. }
        ));
    }

    #[test]
    fn closing_during_registration_never_admits_an_unindexed_response() {
        use crate::platform::interleaving::View;
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        let mut delayed = View::new(&m, Mode::Https);
        delayed.prefix = collection_key(&c.survey_id);
        *delayed.before_write.borrow_mut() = Some(Box::new(|| {
            let late = View::new(
                &m,
                Mode::Near {
                    block_ns: NOW + 100,
                },
            );
            let report = crate::finalize::finalize(
                &late,
                &crate::confidential::Confidential::new(),
                &c.survey_id,
            )
            .unwrap();
            assert_eq!(report.counts.responses, 0);
        }));
        assert!(submit(&delayed, &c.survey_id, &envelope(&c, &d, 1, "not indexed")).is_err());
        assert!(m.list(&responses_prefix(&c.survey_id)).unwrap().is_empty());
        collection::assert_discarded(&m, &c.survey_id);
    }

    #[test]
    fn a_registered_inflight_write_that_wins_is_included_in_the_frozen_batch() {
        use crate::platform::interleaving::View;
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        submit(&m, &c.survey_id, &envelope_at(&c, &d, 1, "admitted", 100)).unwrap();
        let key = m
            .list(&responses_prefix(&c.survey_id))
            .unwrap()
            .pop()
            .unwrap();
        let admitted = m.get(&key).unwrap().unwrap();
        // Simulate the admitted writer paused just before its initial insert.
        m.delete(&key).unwrap();
        let mut freezing = View::new(
            &m,
            Mode::Near {
                block_ns: NOW + 100,
            },
        );
        freezing.prefix = key.clone();
        *freezing.before_write.borrow_mut() = Some(Box::new(|| {
            assert!(m.set_if_absent(&key, &admitted).unwrap());
        }));
        let snapshot = collection::freeze(&freezing, &c.survey_id).unwrap();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].response.answers[0].text, "admitted");
        assert!(collection::is_closed(&m, &c.survey_id).unwrap());
    }
}
