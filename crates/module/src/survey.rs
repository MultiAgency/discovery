//! InitSurvey, Submit and Status.
//!
//! Storage layout (raw records, every value sealed under the project key):
//! - `s/{id}/def`: the canonical survey definition
//! - `s/{id}/key`: X25519 secret, replaced with an empty marker after release
//! - `s/{id}/collection`: admission state and immutable registration-list head
//! - `s/{id}/report`: the released report
//! - `s/{id}/r/{hex mac(id:response hash)}`: open/frozen response slot, then empty tombstone
//! - `s/{id}/i/{random}`: sealed registration metadata, no response content
//!
//! `{id}` is the hex definition hash, which is public. Response keys reveal
//! nothing about who responded without the project key.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use discovery_protocol::definition::SurveyDefinition;
use discovery_protocol::envelope::SurveyKey;
use discovery_protocol::response::Response;
use discovery_protocol::{Error, Result, SurveyStatus};
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

/// Fixed output: no hint whether this contribution was a retry.
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
    if p.mode()? != Mode::Https {
        return Err(Error("submit runs through HTTPS only".into()));
    }
    let definition = load_definition(p, survey_id)?;
    if collection::is_closed(p, survey_id)? {
        return Err(Error("survey is closed".into()));
    }
    let envelope = B64
        .decode(envelope_b64)
        .map_err(|_| Error("envelope is not base64".into()))?;
    let hash = definition.hash()?;
    let plaintext = load_key(p, survey_id)?.open(&envelope, &hash)?;

    // Past this point everything derives from plaintext. Errors leave the
    // enclave through OutLayer's front end, so they never echo it.
    let response: Response =
        serde_json::from_slice(&plaintext).map_err(|_| rejected("the response is malformed"))?;
    response
        .validate(&definition)
        .map_err(|_| rejected("the response does not match the survey"))?;
    // Canonical content, not ciphertext: re-encryption on a CLI retry must not
    // create another contribution. The keyed name prevents guessed-answer lookup.
    let canonical = serde_json::to_vec(&response)?;
    let digest = discovery_protocol::sha256(&canonical);
    let tag = p.mac(format!("{survey_id}:{}", hex::encode(digest)).as_bytes())?;
    let key = format!("{}{}", responses_prefix(survey_id), hex::encode(tag));
    collection::register(p, survey_id, &key, canonical.len() as u64)?;
    if collection::is_closed(p, survey_id)? {
        return Err(Error("survey is closed".into()));
    }
    let slot = Slot::Open {
        submission: response,
    };
    let updated = collection::seal_slot(p, &key, &slot)?;
    if p.set_if_absent(&key, &updated)? {
        return Ok(Accepted { accepted: true });
    }
    // A concurrent retry may have inserted it; a frozen/tombstoned slot is
    // a closure fence and must never be overwritten by a delayed submission.
    let stored = p
        .get(&key)?
        .ok_or_else(|| Error("response slot is missing".into()))?;
    match collection::read_slot(p, &key, &stored)? {
        Slot::Open { .. } => Ok(Accepted { accepted: true }),
        _ => Err(Error("survey is closed".into())),
    }
}

fn rejected(reason: &str) -> Error {
    Error(format!("submission rejected: {reason}"))
}

pub fn status<P: Platform>(p: &P, survey_id: &str) -> Result<SurveyStatus> {
    let records = Records(p);
    let definition = load_definition(p, survey_id)?;
    let key_present = records
        .get(&secret_key(survey_id))?
        .is_some_and(|bytes| !bytes.is_empty());
    let report_released = p.get(&report_key(survey_id))?.is_some();
    Ok(SurveyStatus {
        survey_id: survey_id.into(),
        definition,
        closed: collection::is_closed(p, survey_id)?,
        key_present,
        report_released,
    })
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
pub(crate) mod fixtures {
    use super::Created;
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    use discovery_protocol::definition::{DisclosureRules, Question, SurveyDefinition, Target};
    use discovery_protocol::envelope::seal;
    use discovery_protocol::response::Response;

    pub const NOW: u64 = 1_000;

    /// One question, `q`, collecting until `NOW + 100`.
    pub fn definition() -> SurveyDefinition {
        SurveyDefinition {
            version: discovery_protocol::definition::DEFINITION_VERSION,
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

    /// What the CLI submits: the response sealed to the created survey's key.
    pub fn encrypt(created: &Created, d: &SurveyDefinition, response: &Response) -> String {
        let public = hex::decode(&created.public_key)
            .unwrap()
            .try_into()
            .unwrap();
        let plaintext = serde_json::to_vec(response).unwrap();
        B64.encode(seal(&public, &plaintext, &d.hash().unwrap()).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{definition, encrypt, NOW};
    use super::*;
    use crate::platform::interleaving::View;
    use crate::platform::memory::Memory;
    use discovery_protocol::envelope::seal;
    use discovery_protocol::response::{Answer, Basis, Disclosure, Response};

    fn envelope(created: &Created, d: &SurveyDefinition, text: &str) -> String {
        let response = Response {
            answers: vec![Answer {
                question_id: "q".into(),
                basis: Basis::Confirmed,
                disclosure: Disclosure::Publishable,
                text: text.into(),
            }],
        };
        encrypt(created, d, &response)
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
    fn identical_retries_are_deduplicated_but_distinct_permissions_survive() {
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        let first = envelope(&c, &d, "one immutable contribution");
        submit(&View::new(&m, Mode::Https), &c.survey_id, &first).unwrap();
        submit(&View::new(&m, Mode::Https), &c.survey_id, &first).unwrap();
        submit(
            &View::new(&m, Mode::Https),
            &c.survey_id,
            &envelope(&c, &d, "one immutable contribution"),
        )
        .unwrap();
        assert_eq!(responses(&m, &c.survey_id), 1);
        let slot = m.list(&responses_prefix(&c.survey_id)).pop().unwrap();
        let mut response = collection::test_submission(&m, &slot);
        response.answers[0].disclosure = Disclosure::ContextOnly { phrasing: None };
        submit(
            &View::new(&m, Mode::Https),
            &c.survey_id,
            &encrypt(&c, &d, &response),
        )
        .unwrap();
        response.answers[0].disclosure = Disclosure::ContextOnly {
            phrasing: Some("an approved phrasing".into()),
        };
        submit(
            &View::new(&m, Mode::Https),
            &c.survey_id,
            &encrypt(&c, &d, &response),
        )
        .unwrap();
        let frozen = collection::freeze(&m, &c.survey_id).unwrap();
        assert_eq!(frozen.len(), 3);
        assert!(frozen
            .iter()
            .any(|r| matches!(r.answers[0].disclosure, Disclosure::Publishable)));
        assert!(frozen.iter().any(|r| matches!(
            r.answers[0].disclosure,
            Disclosure::ContextOnly { phrasing: Some(_) }
        )));
    }

    #[test]
    fn concurrent_identical_inserts_keep_one_immutable_response() {
        use crate::platform::interleaving::View;
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        let envelope = envelope(&c, &d, "retry");
        let mut delayed = View::new(&m, Mode::Https);
        delayed.prefix = responses_prefix(&c.survey_id);
        *delayed.before_write.borrow_mut() = Some(Box::new(|| {
            submit(&View::new(&m, Mode::Https), &c.survey_id, &envelope).unwrap();
        }));
        assert!(submit(&delayed, &c.survey_id, &envelope).unwrap().accepted);
        assert_eq!(responses(&m, &c.survey_id), 1);
    }

    #[test]
    fn response_storage_names_do_not_expose_plaintext_or_an_unkeyed_digest() {
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        submit(
            &View::new(&m, Mode::Https),
            &c.survey_id,
            &envelope(&c, &d, "PRIVATE-CONTENT-7731"),
        )
        .unwrap();
        let key = m.list(&responses_prefix(&c.survey_id)).pop().unwrap();
        let response = collection::test_submission(&m, &key);
        let digest = hex::encode(discovery_protocol::sha256(
            &serde_json::to_vec(&response).unwrap(),
        ));
        assert!(!key.contains("PRIVATE-CONTENT-7731"));
        assert!(!key.contains(&digest));
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
        assert!(submit(
            &View::new(&chain, Mode::Https),
            &a.survey_id,
            &envelope(&b, &other, "x")
        )
        .is_err());
    }

    /// Errors after decryption must not echo plaintext, because they leave
    /// the enclave through OutLayer's front end.
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
            format!(r#"{{"answers":[{{"question_id":"q","basis":"{secret}","text":"t"}}]}}"#),
            format!(r#"{{"not json {secret}"#),
        ] {
            let env = B64.encode(seal(&pk, plaintext.as_bytes(), &hash).unwrap());
            let e = submit(&View::new(&chain, Mode::Https), &created.survey_id, &env).unwrap_err();
            assert!(!e.0.contains(secret), "{}", e.0);
        }
        // A well-formed response with an unknown question id.
        let sub = Response {
            answers: vec![Answer {
                question_id: secret.into(),
                basis: Basis::Confirmed,
                disclosure: Disclosure::Publishable,
                text: "t".into(),
            }],
        };
        let env = B64.encode(seal(&pk, &serde_json::to_vec(&sub).unwrap(), &hash).unwrap());
        let e = submit(&View::new(&chain, Mode::Https), &created.survey_id, &env).unwrap_err();
        assert!(!e.0.contains(secret), "{}", e.0);
    }

    #[test]
    fn status_reports_collection_key_and_release_state() {
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
        assert!(s.key_present);
        assert_eq!(s.definition, d);
        assert!(!s.closed && !s.report_released);
    }

    // The deadline is the earliest finalization time, not an HTTPS cutoff.
    #[test]
    fn https_accepts_after_deadline_until_finalization_closes_collection() {
        let d = definition();
        let mut m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        let env = envelope(&c, &d, "late response");
        m.mode = Mode::Near {
            block_ns: d.deadline_ns,
        };
        assert!(super::submit(&m, &c.survey_id, &env).is_err());
        m.mode = Mode::Https;
        assert!(
            submit(&View::new(&m, Mode::Https), &c.survey_id, &env)
                .unwrap()
                .accepted
        );
        collection::close(&m, &c.survey_id).unwrap();
        assert!(submit(&View::new(&m, Mode::Https), &c.survey_id, &env).is_err());
    }

    #[test]
    fn a_delayed_first_submission_cannot_recreate_data_after_release() {
        use crate::platform::interleaving::View;
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let created = init(&m, d.clone()).unwrap();
        let env = envelope(&created, &d, "delayed");
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
                &crate::nearai::Confidential::new(),
                &created.survey_id,
            )
            .unwrap();
        }));
        assert!(submit(&submitting, &created.survey_id, &env).is_err());
        assert!(load_key(&m, &created.survey_id).is_err());
    }

    #[test]
    fn freeze_retries_after_partial_storage_failure_without_a_lock() {
        use crate::platform::interleaving::View;
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        for identity in [1, 2] {
            submit(
                &View::new(&m, Mode::Https),
                &c.survey_id,
                &envelope(&c, &d, &format!("response {identity}")),
            )
            .unwrap();
        }
        let mut failing = View::new(
            &m,
            Mode::Near {
                block_ns: NOW + 100,
            },
        );
        let slots = m.list(&responses_prefix(&c.survey_id));
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
        assert!(submit(
            &View::new(&m, Mode::Https),
            &c.survey_id,
            &envelope(&c, &d, "late")
        )
        .is_err());
    }

    #[test]
    fn frozen_empty_slots_and_secret_markers_cannot_be_recreated() {
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        let key = format!("{}admitted", responses_prefix(&c.survey_id));
        collection::register(&m, &c.survey_id, &key, 0).unwrap();
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
        submit(
            &View::new(&m, Mode::Https),
            &c.survey_id,
            &envelope(&c, &d, "original"),
        )
        .unwrap();
        let key = m.list(&responses_prefix(&c.survey_id)).pop().unwrap();
        let mut submission = collection::test_submission(&m, &key);
        let max = discovery_protocol::envelope::MAX_RESPONSE;
        let current = serde_json::to_vec(&submission).unwrap().len();
        submission.answers[0]
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
            let report =
                crate::finalize::finalize(&late, &crate::nearai::Confidential::new(), &c.survey_id)
                    .unwrap();
            assert_eq!(report.body.summary, "No responses were received.");
        }));
        assert!(submit(&delayed, &c.survey_id, &envelope(&c, &d, "not indexed")).is_err());
        assert!(m.list(&responses_prefix(&c.survey_id)).is_empty());
        collection::assert_discarded(&m, &c.survey_id);
    }

    #[test]
    fn a_registered_inflight_write_that_wins_is_included_in_the_frozen_batch() {
        use crate::platform::interleaving::View;
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        submit(
            &View::new(&m, Mode::Https),
            &c.survey_id,
            &envelope(&c, &d, "admitted"),
        )
        .unwrap();
        let key = m.list(&responses_prefix(&c.survey_id)).pop().unwrap();
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
        assert_eq!(snapshot[0].answers[0].text, "admitted");
        assert!(collection::is_closed(&m, &c.survey_id).unwrap());
    }

    #[test]
    fn distinct_responses_fill_the_byte_budget_but_admitted_retries_still_succeed() {
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        let near_max = |i: usize| {
            let mut text = format!("{i:03} ");
            text.push_str(&"x".repeat(3900));
            envelope(&c, &d, &text)
        };
        let size = {
            let r = Response {
                answers: vec![Answer {
                    question_id: "q".into(),
                    basis: Basis::Confirmed,
                    disclosure: Disclosure::Publishable,
                    text: format!("000 {}", "x".repeat(3900)),
                }],
            };
            serde_json::to_vec(&r).unwrap().len() as u64
        };
        assert!(size <= discovery_protocol::envelope::MAX_RESPONSE as u64);
        let fits = (collection::BUDGET / size) as usize;
        for i in 0..fits {
            submit(&View::new(&m, Mode::Https), &c.survey_id, &near_max(i)).unwrap();
        }
        let full = submit(&View::new(&m, Mode::Https), &c.survey_id, &near_max(fits)).unwrap_err();
        assert_eq!(full.0, "survey is full");
        // A re-encrypted retry of an admitted response is still accepted.
        assert!(
            submit(&View::new(&m, Mode::Https), &c.survey_id, &near_max(0))
                .unwrap()
                .accepted
        );
        assert_eq!(responses(&m, &c.survey_id), fits);
        assert_eq!(collection::freeze(&m, &c.survey_id).unwrap().len(), fits);
    }

    #[test]
    fn racing_identical_submissions_count_once_but_may_charge_the_budget_again() {
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        let near_max = |i: usize| envelope(&c, &d, &format!("{i:03} {}", "x".repeat(3900)));
        let size = {
            let r = Response {
                answers: vec![Answer {
                    question_id: "q".into(),
                    basis: Basis::Confirmed,
                    disclosure: Disclosure::Publishable,
                    text: format!("000 {}", "x".repeat(3900)),
                }],
            };
            serde_json::to_vec(&r).unwrap().len() as u64
        };
        let fits = (collection::BUDGET / size) as usize;
        // Both copies register before either writes its slot.
        let first = near_max(0);
        let mut delayed = View::new(&m, Mode::Https);
        delayed.prefix = responses_prefix(&c.survey_id);
        *delayed.before_write.borrow_mut() = Some(Box::new(|| {
            submit(&View::new(&m, Mode::Https), &c.survey_id, &first).unwrap();
        }));
        submit(&delayed, &c.survey_id, &near_max(0)).unwrap();
        assert_eq!(responses(&m, &c.survey_id), 1);
        // The race registered the slot again, so one fewer distinct response fits.
        for i in 1..fits - 1 {
            submit(&View::new(&m, Mode::Https), &c.survey_id, &near_max(i)).unwrap();
        }
        let full = submit(
            &View::new(&m, Mode::Https),
            &c.survey_id,
            &near_max(fits - 1),
        );
        assert_eq!(full.unwrap_err().0, "survey is full");
        assert_eq!(
            collection::freeze(&m, &c.survey_id).unwrap().len(),
            fits - 1
        );
    }

    #[test]
    fn oversized_responses_cannot_be_sealed() {
        let d = definition();
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, d.clone()).unwrap();
        let public: [u8; 32] = hex::decode(&c.public_key).unwrap().try_into().unwrap();
        let text = "x".repeat(discovery_protocol::envelope::MAX_RESPONSE);
        assert!(seal(&public, text.as_bytes(), &d.hash().unwrap()).is_ok());
        assert!(seal(&public, format!("{text}x").as_bytes(), &d.hash().unwrap()).is_err());
    }
}
