//! Monotone collection closure using single-record CAS, without leases.
//!
//! Register a slot before writing a response. Closing fixes the registration
//! list; freezing each slot then fences admitted writers. Discard replaces
//! contents with tombstones that late CAS/inserts cannot overwrite.

use crate::platform::{Platform, Records};
use crate::survey::{collection_key, responses_prefix, secret_key};
use discovery_protocol::identity::Submission;
use discovery_protocol::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const RETRIES: usize = 8;

#[derive(Serialize, Deserialize)]
struct Collection {
    closed: bool,
    head: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Registration {
    slot: String,
    next: Option<String>,
}

pub enum Slot {
    Open { submission: Submission },
    Frozen { submission: Option<Submission> },
    Discarded,
}

pub fn init<P: Platform>(p: &P, id: &str) -> Result<()> {
    Records(p).set_if_absent(
        &collection_key(id),
        &serde_json::to_vec(&Collection {
            closed: false,
            head: None,
        })?,
    )?;
    Ok(())
}

fn state<P: Platform>(p: &P, id: &str) -> Result<(Vec<u8>, Collection)> {
    let key = collection_key(id);
    let raw = p
        .get(&key)?
        .ok_or_else(|| Error("collection not initialized".into()))?;
    let value = serde_json::from_slice(&p.decrypt(&raw, key.as_bytes())?)
        .map_err(|_| Error("collection state is unreadable".into()))?;
    Ok((raw, value))
}

pub fn is_closed<P: Platform>(p: &P, id: &str) -> Result<bool> {
    Ok(state(p, id)?.1.closed)
}

/// Existing slots were already registered. A crash before the first response
/// write may leave a registration with no data; finalization freezes it empty.
pub fn register<P: Platform>(p: &P, id: &str, slot: &str) -> Result<()> {
    if p.get(slot)?.is_some() {
        return Ok(());
    }
    let key = collection_key(id);
    for _ in 0..RETRIES {
        let (raw, mut collection) = state(p, id)?;
        if collection.closed {
            return Err(Error("survey is closed".into()));
        }
        let mut random = [0; 32];
        getrandom::fill(&mut random).map_err(|_| Error("randomness unavailable".into()))?;
        let node = format!("s/{id}/i/{}", hex::encode(random));
        let registration = Registration {
            slot: slot.into(),
            next: collection.head.clone(),
        };
        if !Records(p).set_if_absent(&node, &serde_json::to_vec(&registration)?)? {
            continue;
        }
        collection.head = Some(node.clone());
        let updated = p.encrypt(&serde_json::to_vec(&collection)?, key.as_bytes())?;
        if p.compare_and_swap(&key, &raw, &updated)? {
            return Ok(());
        }
        // Only this run knows this random node; an unsuccessful CAS never
        // publishes it. A crash can leave metadata, never a response, here.
        p.delete(&node)?;
    }
    Err(Error(
        "collection changed repeatedly; retry submission".into(),
    ))
}

/// Close admission and return every registered slot, including empty ones.
/// In-flight writers can finish only until their own slot has been frozen.
pub fn close<P: Platform>(p: &P, id: &str) -> Result<Vec<String>> {
    let key = collection_key(id);
    let mut head = None;
    let mut closed = false;
    for _ in 0..RETRIES {
        let (raw, mut collection) = state(p, id)?;
        head = collection.head.clone();
        if collection.closed {
            closed = true;
            break;
        }
        collection.closed = true;
        let updated = p.encrypt(&serde_json::to_vec(&collection)?, key.as_bytes())?;
        if p.compare_and_swap(&key, &raw, &updated)? {
            closed = true;
            break;
        }
    }
    if !closed {
        return Err(Error(
            "collection changed repeatedly; retry finalization".into(),
        ));
    }
    let mut nodes = BTreeSet::new();
    let mut slots = BTreeSet::new();
    while let Some(node) = head {
        if !node.starts_with(&format!("s/{id}/i/")) || !nodes.insert(node.clone()) {
            return Err(Error("invalid collection index".into()));
        }
        let bytes = Records(p).require(&node, "collection index")?;
        let registration: Registration = serde_json::from_slice(&bytes)
            .map_err(|_| Error("collection index is unreadable".into()))?;
        if !registration.slot.starts_with(&responses_prefix(id)) {
            return Err(Error("invalid response slot".into()));
        }
        slots.insert(registration.slot);
        head = registration.next;
    }
    Ok(slots.into_iter().collect())
}

/// The phase is authenticated as AAD, outside the encrypted submission.
/// This preserves the existing per-response plaintext limit without adding
/// a JSON wrapper around a maximum-size submission.
pub fn seal_slot<P: Platform>(p: &P, key: &str, slot: &Slot) -> Result<Vec<u8>> {
    let (phase, plaintext) = match slot {
        Slot::Open { submission } => (1, serde_json::to_vec(submission)?),
        Slot::Frozen {
            submission: Some(submission),
        } => (2, serde_json::to_vec(submission)?),
        Slot::Frozen { submission: None } => (2, Vec::new()),
        Slot::Discarded => (3, Vec::new()),
    };
    let mut raw = vec![phase];
    raw.extend(p.encrypt(&plaintext, format!("{key}:{phase}").as_bytes())?);
    Ok(raw)
}

pub fn read_slot<P: Platform>(p: &P, key: &str, raw: &[u8]) -> Result<Slot> {
    let Some((&phase, ciphertext)) = raw.split_first() else {
        return Err(Error("stored response is unreadable".into()));
    };
    if !(1..=3).contains(&phase) {
        return Err(Error("stored response is unreadable".into()));
    }
    let plaintext = p.decrypt(ciphertext, format!("{key}:{phase}").as_bytes())?;
    match phase {
        1 => Ok(Slot::Open {
            submission: serde_json::from_slice(&plaintext)
                .map_err(|_| Error("stored response is unreadable".into()))?,
        }),
        2 => Ok(Slot::Frozen {
            submission: if plaintext.is_empty() {
                None
            } else {
                Some(
                    serde_json::from_slice(&plaintext)
                        .map_err(|_| Error("stored response is unreadable".into()))?,
                )
            },
        }),
        3 if plaintext.is_empty() => Ok(Slot::Discarded),
        _ => Err(Error("stored response is unreadable".into())),
    }
}

/// Freeze all slots before inference. Repeated finalizations see the same
/// immutable contributions. Empty registered slots receive a frozen marker.
pub fn freeze<P: Platform>(p: &P, id: &str) -> Result<Vec<Submission>> {
    let mut submissions = Vec::new();
    for key in close(p, id)? {
        let mut frozen = false;
        for _ in 0..RETRIES {
            let raw = p.get(&key)?;
            let submission = match &raw {
                Some(raw) => match read_slot(p, &key, raw)? {
                    Slot::Frozen { submission } => {
                        submissions.extend(submission);
                        frozen = true;
                        break;
                    }
                    Slot::Open { submission } => Some(submission),
                    Slot::Discarded => {
                        return Err(Error(
                            "survey was already released; retry finalization".into(),
                        ))
                    }
                },
                None => None,
            };
            let slot = Slot::Frozen { submission };
            let bytes = seal_slot(p, &key, &slot)?;
            let changed = match raw {
                Some(raw) => p.compare_and_swap(&key, &raw, &bytes)?,
                None => p.set_if_absent(&key, &bytes)?,
            };
            if changed {
                if let Slot::Frozen { submission } = slot {
                    submissions.extend(submission);
                }
                frozen = true;
                break;
            }
        }
        if !frozen {
            return Err(Error(
                "response changed repeatedly; retry finalization".into(),
            ));
        }
    }
    Ok(submissions)
}

/// Replace response contents and key material with permanent empty markers.
/// Never delete these fences: a delayed writer may still hold an old snapshot.
pub fn discard<P: Platform>(p: &P, id: &str) -> Result<()> {
    for key in p.list(&responses_prefix(id))? {
        let marker = seal_slot(p, &key, &Slot::Discarded)?;
        let mut discarded = false;
        for _ in 0..RETRIES {
            match p.get(&key)? {
                Some(raw) => {
                    if matches!(read_slot(p, &key, &raw)?, Slot::Discarded)
                        || p.compare_and_swap(&key, &raw, &marker)?
                    {
                        discarded = true;
                        break;
                    }
                }
                None => {
                    if p.set_if_absent(&key, &marker)? {
                        discarded = true;
                        break;
                    }
                }
            }
        }
        if !discarded {
            return Err(Error("cleanup interrupted; retry finalization".into()));
        }
    }
    replace_with_marker(p, &secret_key(id), b"")?;
    Ok(())
}

fn replace_with_marker<P: Platform>(p: &P, key: &str, marker: &[u8]) -> Result<()> {
    let updated = p.encrypt(marker, key.as_bytes())?;
    for _ in 0..RETRIES {
        match p.get(key)? {
            Some(raw) => {
                if p.decrypt(&raw, key.as_bytes())? == marker {
                    return Ok(());
                }
                if p.compare_and_swap(key, &raw, &updated)? {
                    return Ok(());
                }
            }
            None => {
                if p.set_if_absent(key, &updated)? {
                    return Ok(());
                }
            }
        }
    }
    Err(Error("cleanup interrupted; retry finalization".into()))
}

#[cfg(test)]
pub fn assert_discarded<P: Platform>(p: &P, id: &str) {
    for key in p.list(&responses_prefix(id)).unwrap() {
        let raw = p.get(&key).unwrap().unwrap();
        assert!(matches!(read_slot(p, &key, &raw).unwrap(), Slot::Discarded));
    }
    assert_eq!(
        Records(p).require(&secret_key(id), "key marker").unwrap(),
        b""
    );
}

#[cfg(test)]
pub fn test_submission<P: Platform>(p: &P, key: &str) -> Submission {
    match read_slot(p, key, &p.get(key).unwrap().unwrap()).unwrap() {
        Slot::Open { submission }
        | Slot::Frozen {
            submission: Some(submission),
        } => submission,
        _ => panic!("no submission in slot"),
    }
}

#[cfg(test)]
pub fn test_store<P: Platform>(p: &P, key: &str, submission: Submission) {
    p.set(key, &seal_slot(p, key, &Slot::Open { submission }).unwrap())
        .unwrap();
}
