//! InitSurvey, Submit and Status.
//!
//! Storage layout (raw records, every value sealed under the project key):
//! - `s/{id}/def`: the canonical survey definition
//! - `s/{id}/key`: X25519 secret, replaced with an empty marker after release
//! - `s/{id}/collection`: admission state and the registration-list head, fixed at close
//! - `s/{id}/report`: the released report
//! - `s/{id}/r/{hex mac(id:response hash)}`: open response slot, later an empty fence
//! - `s/{id}/i/{random}`: sealed registration metadata, no response content
//!
//! `{id}` is the hex definition hash, which is public. Response keys reveal
//! nothing about who responded without the project key.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use discovery_protocol::definition::SurveyDefinition;
use discovery_protocol::envelope::{SurveyKey, MAX_RESPONSE};
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
    // Omitted default fields can expand beyond the envelope's plaintext limit.
    if canonical.len() > MAX_RESPONSE {
        return Err(rejected("the response is too large"));
    }
    let digest = discovery_protocol::sha256(&canonical);
    let tag = p.mac(format!("{survey_id}:{}", hex::encode(digest)).as_bytes())?;
    let key = format!("{}{}", responses_prefix(survey_id), hex::encode(tag));
    // An identical earlier submission is accepted once; a fenced slot means
    // collection closed and must never be overwritten by a delayed submission.
    let existing = |stored: Vec<u8>| match collection::read_slot(p, &key, &stored)? {
        Slot::Open { .. } => Ok(Accepted { accepted: true }),
        Slot::Fenced => Err(Error("survey is closed".into())),
    };
    if let Some(stored) = p.get(&key)? {
        return existing(stored);
    }
    // Registration fails once collection has closed; a slot registered before
    // that is on the closed list, so freezing reads or fences it.
    collection::register(p, survey_id, &key)?;
    let slot = Slot::Open {
        submission: response,
    };
    if p.set_if_absent(&key, &collection::seal_slot(p, &key, &slot)?)? {
        return Ok(Accepted { accepted: true });
    }
    // A concurrent identical submission or a freeze fence got there first.
    existing(
        p.get(&key)?
            .ok_or_else(|| Error("response slot is missing".into()))?,
    )
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
    let (closed, full) = collection::admission(p, survey_id)?;
    Ok(SurveyStatus {
        survey_id: survey_id.into(),
        definition,
        closed,
        full,
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
    use super::{init, submit, Accepted, Created};
    use crate::platform::memory::Memory;
    use crate::platform::Mode;
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    use discovery_protocol::definition::{DisclosureRules, Question, SurveyDefinition, Target};
    use discovery_protocol::envelope::seal;
    use discovery_protocol::response::{Answer, Basis, Disclosure, Response};
    use discovery_protocol::Result;

    pub const NOW: u64 = 1_000;
    pub const DEADLINE: u64 = NOW + 100;

    /// One question, `q`, collecting until [`DEADLINE`].
    pub fn definition() -> SurveyDefinition {
        SurveyDefinition {
            version: discovery_protocol::definition::DEFINITION_VERSION,
            title: "t".into(),
            target: Target {
                repo: "MultiAgency/discovery".into(),
                number: 1,
            },
            deadline_ns: DEADLINE,
            questions: vec![Question {
                id: "q".into(),
                prompt: "p".into(),
            }],
            rules: DisclosureRules::default(),
        }
    }

    /// [`definition`], created at [`NOW`] on fresh storage.
    pub fn created() -> (Memory<'static>, Created) {
        let m = Memory::new(Mode::Near { block_ns: NOW });
        let c = init(&m, definition()).unwrap();
        (m, c)
    }

    /// One answer to `q`.
    pub fn response(basis: Basis, disclosure: Disclosure, text: &str) -> Response {
        Response {
            answers: vec![Answer {
                question_id: "q".into(),
                basis,
                disclosure,
                text: text.into(),
            }],
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

    /// Submit `response` to [`created`]'s survey over HTTPS.
    pub fn send(m: &Memory, c: &Created, response: &Response) -> Result<Accepted> {
        submit(
            &m.view(Mode::Https),
            &c.survey_id,
            &encrypt(c, &definition(), response),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{created, definition, encrypt, response, send, DEADLINE};
    use super::*;
    use crate::platform::memory::Memory;
    use discovery_protocol::envelope::seal;
    use discovery_protocol::response::{Basis, Disclosure};

    fn publishable(text: &str) -> Response {
        response(Basis::Confirmed, Disclosure::Publishable, text)
    }

    fn envelope(c: &Created, text: &str) -> String {
        encrypt(c, &definition(), &publishable(text))
    }

    fn https(m: &Memory) -> Memory<'static> {
        m.view(Mode::Https)
    }

    fn responses(m: &Memory, id: &str) -> usize {
        m.list(&responses_prefix(id)).len()
    }

    fn frozen(m: &Memory, c: &Created) -> Vec<Response> {
        collection::freeze(m, &c.survey_id).unwrap().submissions
    }

    #[test]
    fn init_is_on_chain_only_idempotent_and_refuses_past_deadlines() {
        assert!(init(&Memory::new(Mode::Https), definition()).is_err());
        let (m, a) = created();
        assert_eq!(init(&m, definition()).unwrap(), a);
        let late = Memory::new(Mode::Near { block_ns: DEADLINE });
        assert!(init(&late, definition()).is_err());
    }

    #[test]
    fn identical_retries_are_deduplicated_but_distinct_permissions_survive() {
        let (m, c) = created();
        let first = envelope(&c, "one immutable contribution");
        submit(&https(&m), &c.survey_id, &first).unwrap();
        submit(&https(&m), &c.survey_id, &first).unwrap();
        send(&m, &c, &publishable("one immutable contribution")).unwrap();
        assert_eq!(responses(&m, &c.survey_id), 1);
        let mut response = publishable("one immutable contribution");
        response.answers[0].disclosure = Disclosure::ContextOnly { phrasing: None };
        send(&m, &c, &response).unwrap();
        response.answers[0].disclosure = Disclosure::ContextOnly {
            phrasing: Some("an approved phrasing".into()),
        };
        send(&m, &c, &response).unwrap();
        let frozen = frozen(&m, &c);
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
        let (m, c) = created();
        let envelope = envelope(&c, "retry");
        let mut delayed = https(&m);
        delayed.prefix = responses_prefix(&c.survey_id);
        delayed.before_write = Box::new(|_| {
            submit(&https(&m), &c.survey_id, &envelope).unwrap();
        });
        assert!(submit(&delayed, &c.survey_id, &envelope).unwrap().accepted);
        assert_eq!(responses(&m, &c.survey_id), 1);
    }

    #[test]
    fn response_storage_names_do_not_expose_plaintext_or_an_unkeyed_digest() {
        let (m, c) = created();
        send(&m, &c, &publishable("PRIVATE-CONTENT-7731")).unwrap();
        let key = m.list(&responses_prefix(&c.survey_id)).pop().unwrap();
        let digest = hex::encode(discovery_protocol::sha256(
            &serde_json::to_vec(&publishable("PRIVATE-CONTENT-7731")).unwrap(),
        ));
        assert!(!key.contains("PRIVATE-CONTENT-7731"));
        assert!(!key.contains(&digest));
    }

    #[test]
    fn foreign_survey_envelope_refused() {
        let (m, a) = created();
        let mut other = definition();
        other.title = "other".into();
        let b = init(&m, other.clone()).unwrap();
        // An envelope for survey b, submitted to survey a.
        let foreign = encrypt(&b, &other, &publishable("x"));
        assert!(submit(&https(&m), &a.survey_id, &foreign).is_err());
    }

    /// Errors after decryption must not echo plaintext, because they leave
    /// the enclave through OutLayer's front end.
    #[test]
    fn post_decryption_errors_do_not_echo_plaintext() {
        let (m, c) = created();
        let hash = definition().hash().unwrap();
        let pk: [u8; 32] = hex::decode(&c.public_key).unwrap().try_into().unwrap();
        let secret = "SECRET-PLAINTEXT-7731";
        for plaintext in [
            format!(r#"{{"answers":[{{"question_id":"q","basis":"{secret}","text":"t"}}]}}"#),
            format!(r#"{{"not json {secret}"#),
        ] {
            let env = B64.encode(seal(&pk, plaintext.as_bytes(), &hash).unwrap());
            let e = submit(&https(&m), &c.survey_id, &env).unwrap_err();
            assert!(!e.0.contains(secret), "{}", e.0);
        }
        // A well-formed response with an unknown question id.
        let mut unknown = publishable("t");
        unknown.answers[0].question_id = secret.into();
        let e = send(&m, &c, &unknown).unwrap_err();
        assert!(!e.0.contains(secret), "{}", e.0);
    }

    #[test]
    fn submission_size_uses_canonical_defaults_before_registration() {
        let max = discovery_protocol::envelope::MAX_RESPONSE;
        for omit_disclosure in [true, false] {
            for size in [max, max + 1] {
                let (m, c) = created();
                let mut expected = response(
                    Basis::Observed,
                    Disclosure::default(),
                    "PRIVATE-CONTENT-7731",
                );
                let current = serde_json::to_vec(&expected).unwrap().len();
                expected.answers[0]
                    .text
                    .push_str(&"x".repeat(size - current));
                assert_eq!(serde_json::to_vec(&expected).unwrap().len(), size);

                // Both the outer disclosure and its nested phrasing have
                // defaults. A custom client can omit either on the wire.
                let mut wire = serde_json::to_value(&expected).unwrap();
                let answer = &mut wire["answers"][0];
                if omit_disclosure {
                    answer.as_object_mut().unwrap().remove("disclosure");
                } else {
                    answer["disclosure"]
                        .as_object_mut()
                        .unwrap()
                        .remove("phrasing");
                }
                let plaintext = serde_json::to_vec(&wire).unwrap();
                assert!(plaintext.len() <= max);
                let public = hex::decode(&c.public_key).unwrap().try_into().unwrap();
                let envelope =
                    B64.encode(seal(&public, &plaintext, &definition().hash().unwrap()).unwrap());
                let before = m.store.borrow().clone();
                let result = submit(&https(&m), &c.survey_id, &envelope);
                if size == max {
                    assert!(result.unwrap().accepted);
                    assert_eq!(frozen(&m, &c), vec![expected]);
                } else {
                    assert_eq!(
                        result.unwrap_err().0,
                        "submission rejected: the response is too large"
                    );
                    // Oversized input must consume no registration or storage.
                    assert_eq!(*m.store.borrow(), before);
                }
            }
        }
    }

    #[test]
    fn status_reports_collection_key_and_release_state() {
        let (m, c) = created();
        let s = status(&https(&m), &c.survey_id).unwrap();
        assert!(s.key_present);
        assert_eq!(s.definition, definition());
        assert!(!s.closed && !s.full && !s.report_released);
    }

    // The deadline is the earliest finalization time, not an HTTPS cutoff.
    #[test]
    fn https_accepts_after_deadline_until_finalization_closes_collection() {
        let (m, c) = created();
        let env = envelope(&c, "late response");
        let chain = m.view(Mode::Near { block_ns: DEADLINE });
        assert!(submit(&chain, &c.survey_id, &env).is_err());
        assert!(submit(&https(&m), &c.survey_id, &env).unwrap().accepted);
        collection::close(&m, &c.survey_id).unwrap();
        assert!(submit(&https(&m), &c.survey_id, &env).is_err());
    }

    #[test]
    fn a_delayed_first_submission_cannot_recreate_data_after_release() {
        let (m, c) = created();
        let mut submitting = https(&m);
        submitting.prefix = responses_prefix(&c.survey_id);
        submitting.before_write = Box::new(|_| {
            let finalizing = m.view(Mode::Near { block_ns: DEADLINE });
            crate::finalize::finalize(
                &finalizing,
                &crate::nearai::Confidential::new(),
                &c.survey_id,
            )
            .unwrap();
        });
        assert!(submit(&submitting, &c.survey_id, &envelope(&c, "delayed")).is_err());
        assert!(load_key(&m, &c.survey_id).is_err());
    }

    #[test]
    fn freeze_retries_after_partial_storage_failure_without_a_lock() {
        let (m, c) = created();
        send(&m, &c, &publishable("response")).unwrap();
        let stored = m.list(&responses_prefix(&c.survey_id)).pop().unwrap();
        let before = m.get(&stored).unwrap();
        // Admitted writers that registered but have not inserted yet.
        let empty = |n: u8| format!("{}admitted-{n}", responses_prefix(&c.survey_id));
        for n in [1, 2] {
            collection::register(&m, &c.survey_id, &empty(n)).unwrap();
        }
        let mut failing = m.view(Mode::Near { block_ns: DEADLINE });
        failing.prefix = empty(2);
        failing.failures.set(1);
        assert!(collection::freeze(&failing, &c.survey_id).is_err());
        assert!(collection::is_closed(&m, &c.survey_id).unwrap());
        assert_eq!(frozen(&m, &c).len(), 1);
        assert_eq!(frozen(&m, &c).len(), 1);
        // The stored response was read, never rewritten.
        assert_eq!(m.get(&stored).unwrap(), before);
        for n in [1, 2] {
            let raw = m.get(&empty(n)).unwrap().unwrap();
            assert!(matches!(
                collection::read_slot(&m, &empty(n), &raw).unwrap(),
                Slot::Fenced
            ));
            assert!(!m.set_if_absent(&empty(n), b"delayed response").unwrap());
        }
        assert!(send(&m, &c, &publishable("late")).is_err());
    }

    #[test]
    fn frozen_empty_slots_and_secret_markers_cannot_be_recreated() {
        let (m, c) = created();
        let key = format!("{}admitted", responses_prefix(&c.survey_id));
        collection::register(&m, &c.survey_id, &key).unwrap();
        assert!(frozen(&m, &c).is_empty());
        collection::discard(&m, &c.survey_id, None).unwrap();
        assert!(!Records(&m)
            .set_if_absent(&secret_key(&c.survey_id), &[7; 32])
            .unwrap());
        assert!(!m.set_if_absent(&key, b"delayed response").unwrap());
        assert!(init(&m, definition()).is_err());
        collection::assert_discarded(&m, &c.survey_id);
    }

    #[test]
    fn slot_phases_are_authenticated_and_keep_the_full_response_size_limit() {
        let (m, c) = created();
        let key = format!("{}slot", responses_prefix(&c.survey_id));
        let mut submission = publishable("original");
        let max = discovery_protocol::envelope::MAX_RESPONSE;
        let current = serde_json::to_vec(&submission).unwrap().len();
        submission.answers[0]
            .text
            .push_str(&"x".repeat(max - current));
        let raw = collection::seal_slot(&m, &key, &Slot::Open { submission }).unwrap();
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
        let (m, c) = created();
        let mut delayed = https(&m);
        delayed.prefix = collection_key(&c.survey_id);
        delayed.before_write = Box::new(|_| {
            let late = m.view(Mode::Near { block_ns: DEADLINE });
            let report =
                crate::finalize::finalize(&late, &crate::nearai::Confidential::new(), &c.survey_id)
                    .unwrap();
            assert_eq!(report.body.summary, "No responses were received.");
        });
        assert!(submit(&delayed, &c.survey_id, &envelope(&c, "not indexed")).is_err());
        assert!(m.list(&responses_prefix(&c.survey_id)).is_empty());
        collection::assert_discarded(&m, &c.survey_id);
    }

    #[test]
    fn a_registered_inflight_write_that_wins_is_included_in_the_frozen_batch() {
        let (m, c) = created();
        send(&m, &c, &publishable("admitted")).unwrap();
        let key = m.list(&responses_prefix(&c.survey_id)).pop().unwrap();
        let admitted = m.get(&key).unwrap().unwrap();
        // Simulate the admitted writer paused just before its initial insert.
        m.delete(&key).unwrap();
        let mut freezing = m.view(Mode::Near { block_ns: DEADLINE });
        freezing.prefix = key.clone();
        freezing.before_write = Box::new(|_| {
            assert!(m.set_if_absent(&key, &admitted).unwrap());
        });
        let snapshot = frozen(&freezing, &c);
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].answers[0].text, "admitted");
        assert!(collection::is_closed(&m, &c.survey_id).unwrap());
    }

    #[test]
    fn distinct_responses_fill_the_registrations_but_admitted_retries_still_succeed() {
        let (m, c) = created();
        let places = collection::REGISTRATIONS as usize;
        for i in 0..places {
            send(&m, &c, &publishable(&format!("response {i}"))).unwrap();
        }
        let full = send(&m, &c, &publishable("one too many")).unwrap_err();
        assert_eq!(full.0, "survey is full");
        let s = status(&https(&m), &c.survey_id).unwrap();
        assert!(s.full && !s.closed && s.key_present);
        // A re-encrypted retry of an admitted response is still accepted.
        assert!(send(&m, &c, &publishable("response 0")).unwrap().accepted);
        assert_eq!(responses(&m, &c.survey_id), places);
        assert_eq!(frozen(&m, &c).len(), places);
    }

    #[test]
    fn racing_identical_submissions_count_once_but_may_use_a_second_place() {
        let (m, c) = created();
        let places = collection::REGISTRATIONS as usize;
        // Both copies register before either writes its slot.
        let mut delayed = https(&m);
        delayed.prefix = responses_prefix(&c.survey_id);
        delayed.before_write = Box::new(|_| {
            send(&m, &c, &publishable("response 0")).unwrap();
        });
        submit(&delayed, &c.survey_id, &envelope(&c, "response 0")).unwrap();
        assert_eq!(responses(&m, &c.survey_id), 1);
        // The race registered the slot twice, so one fewer distinct response fits.
        for i in 1..places - 1 {
            send(&m, &c, &publishable(&format!("response {i}"))).unwrap();
        }
        let full = send(&m, &c, &publishable("one too many"));
        assert_eq!(full.unwrap_err().0, "survey is full");
        assert_eq!(frozen(&m, &c).len(), places - 1);
    }
}
