//! Monotone collection closure using single-record CAS, without leases.
//!
//! Register a slot before writing a response. Closing fixes the registration
//! list, which names every slot ever written. Writers only insert, so a stored
//! response is already immutable; freezing fences each empty registered slot
//! so its delayed writer can never insert. Discard replaces contents with the
//! same empty fence, which late CAS/inserts cannot overwrite. Neither depends
//! on storage listing.
//!
//! OutLayer runs fail on their 126th storage call: twice on testnet, call
//! #126 hung for the 30 s client timeout after 125 calls had succeeded, once a
//! read and once a CAS. The cause is unresolved. Finalization therefore stays
//! within 100 storage calls, which bounds admitted [`REGISTRATIONS`].

use crate::platform::{Platform, Records};
use crate::survey::{collection_key, responses_prefix, secret_key};
use discovery_protocol::response::Response;
use discovery_protocol::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const RETRIES: usize = 8;

/// Registrations one survey admits. A run must stay at or under 100 storage
/// calls: 125 succeeded on testnet and the 126th failed, so a quarter stays in
/// reserve. Finalization costs 7 calls plus 3 per registration, or 5 for a slot
/// whose delayed writer inserts while it is being frozen; 18 keeps even every
/// slot racing within 97. Registrations, not distinct responses, are counted,
/// because each costs finalization the same work: racing identical submissions
/// and interrupted ones each use a place.
pub const REGISTRATIONS: u64 = 18;

#[derive(Serialize, Deserialize)]
struct Collection {
    closed: bool,
    head: Option<String>,
    /// Registrations admitted, against [`REGISTRATIONS`].
    registrations: u64,
}

#[derive(Serialize, Deserialize)]
struct Registration {
    slot: String,
    next: Option<String>,
}

pub enum Slot {
    Open {
        submission: Response,
    },
    /// Empty and fenced: frozen at closure, or emptied after release. A
    /// fence only ever follows a stored report or a closed collection.
    Fenced,
}

/// What freezing read: the responses, and each slot's stored bytes, so cleanup
/// can replace them with one conditional write instead of reading them again.
pub struct Frozen {
    pub submissions: Vec<Response>,
    slots: Vec<(String, Vec<u8>)>,
}

pub fn init<P: Platform>(p: &P, id: &str) -> Result<()> {
    Records(p).set_if_absent(
        &collection_key(id),
        &serde_json::to_vec(&Collection {
            closed: false,
            head: None,
            registrations: 0,
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

/// Whether admission is closed, and whether every registration is used.
pub fn admission<P: Platform>(p: &P, id: &str) -> Result<(bool, bool)> {
    let collection = state(p, id)?.1;
    Ok((collection.closed, collection.registrations >= REGISTRATIONS))
}

/// Admit a new slot. Callers skip slots that already exist, so an identical
/// retry uses no place. Identical submissions racing before either writes, or
/// a retry after a crash between registration and the write, register the
/// slot again and use another place each time; `close` deduplicates slots, so
/// only capacity is lost. Fixing that needs a lease or a list walk. An empty
/// registered slot is fenced by finalization.
pub fn register<P: Platform>(p: &P, id: &str, slot: &str) -> Result<()> {
    let key = collection_key(id);
    for _ in 0..RETRIES {
        let (raw, mut collection) = state(p, id)?;
        if collection.closed {
            return Err(Error("survey is closed".into()));
        }
        if collection.registrations >= REGISTRATIONS {
            return Err(Error("survey is full".into()));
        }
        let random = discovery_protocol::random_bytes::<32>()?;
        let node = format!("s/{id}/i/{}", hex::encode(random));
        let registration = Registration {
            slot: slot.into(),
            next: collection.head.clone(),
        };
        if !Records(p).set_if_absent(&node, &serde_json::to_vec(&registration)?)? {
            continue;
        }
        collection.head = Some(node.clone());
        collection.registrations += 1;
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
        Slot::Fenced => (2, Vec::new()),
    };
    let mut raw = vec![phase];
    raw.extend(p.encrypt(&plaintext, format!("{key}:{phase}").as_bytes())?);
    Ok(raw)
}

pub fn read_slot<P: Platform>(p: &P, key: &str, raw: &[u8]) -> Result<Slot> {
    let Some((&phase, ciphertext)) = raw.split_first() else {
        return Err(Error("stored response is unreadable".into()));
    };
    if !(1..=2).contains(&phase) {
        return Err(Error("stored response is unreadable".into()));
    }
    let plaintext = p.decrypt(ciphertext, format!("{key}:{phase}").as_bytes())?;
    match phase {
        1 => Ok(Slot::Open {
            submission: serde_json::from_slice(&plaintext)
                .map_err(|_| Error("stored response is unreadable".into()))?,
        }),
        2 if plaintext.is_empty() => Ok(Slot::Fenced),
        _ => Err(Error("stored response is unreadable".into())),
    }
}

/// Freeze all slots before inference: close admission, read every registered
/// slot once, and mark each empty one so its delayed writer cannot insert.
/// Stored responses are never rewritten. Repeated finalizations see the same
/// contributions.
pub fn freeze<P: Platform>(p: &P, id: &str) -> Result<Frozen> {
    let mut frozen = Frozen {
        submissions: Vec::new(),
        slots: Vec::new(),
    };
    for key in close(p, id)? {
        let mut stored = None;
        for _ in 0..RETRIES {
            if let Some(raw) = p.get(&key)? {
                match read_slot(p, &key, &raw)? {
                    Slot::Open { submission } => frozen.submissions.push(submission),
                    // Empty, or emptied after another finalizer stored its
                    // report, which this run will then return instead.
                    Slot::Fenced => {}
                }
                stored = Some(raw);
                break;
            }
            // A writer that inserts first wins and is read on the next pass.
            let marker = seal_slot(p, &key, &Slot::Fenced)?;
            if p.set_if_absent(&key, &marker)? {
                stored = Some(marker);
                break;
            }
        }
        let Some(raw) = stored else {
            return Err(Error(
                "response changed repeatedly; retry finalization".into(),
            ));
        };
        frozen.slots.push((key, raw));
    }
    Ok(frozen)
}

/// Replace response contents and key material with permanent empty markers.
/// Never delete these fences: a delayed writer may still hold an old snapshot.
/// An empty slot's freeze fence is left in place. With `frozen`, each slot is
/// replaced from the bytes freezing read; a slot changed since (another
/// finalizer's fence) falls back to reading it.
/// Without it, after an interrupted cleanup, the registration list is read.
pub fn discard<P: Platform>(p: &P, id: &str, frozen: Option<&Frozen>) -> Result<()> {
    let slots: Vec<(String, Option<&[u8]>)> = match frozen {
        Some(f) => f
            .slots
            .iter()
            .map(|(key, raw)| (key.clone(), Some(raw.as_slice())))
            .collect(),
        None => close(p, id)?.into_iter().map(|key| (key, None)).collect(),
    };
    for (key, known) in slots {
        let marker = seal_slot(p, &key, &Slot::Fenced)?;
        fence(p, &key, &marker, known, |raw| {
            Ok(matches!(read_slot(p, &key, raw)?, Slot::Fenced))
        })?;
    }
    let key = secret_key(id);
    let marker = p.encrypt(b"", key.as_bytes())?;
    fence(p, &key, &marker, None, |raw| {
        Ok(p.decrypt(raw, key.as_bytes())?.is_empty())
    })
}

/// Install `marker` at `key` unless `is_marker` recognizes one already there.
/// `known` is the value last read there: if it is still current, one
/// conditional write suffices.
fn fence<P: Platform>(
    p: &P,
    key: &str,
    marker: &[u8],
    known: Option<&[u8]>,
    is_marker: impl Fn(&[u8]) -> Result<bool>,
) -> Result<()> {
    if let Some(raw) = known {
        if is_marker(raw)? || p.compare_and_swap(key, raw, marker)? {
            return Ok(());
        }
    }
    for _ in 0..RETRIES {
        let fenced = match p.get(key)? {
            Some(raw) => is_marker(&raw)? || p.compare_and_swap(key, &raw, marker)?,
            None => p.set_if_absent(key, marker)?,
        };
        if fenced {
            return Ok(());
        }
    }
    Err(Error("cleanup interrupted; retry finalization".into()))
}

/// Checks raw storage, not the registration list, so an unindexed slot fails.
#[cfg(test)]
pub fn assert_discarded(m: &crate::platform::memory::Memory, id: &str) {
    for key in m.list(&responses_prefix(id)) {
        let raw = m.get(&key).unwrap().unwrap();
        assert!(matches!(read_slot(m, &key, &raw).unwrap(), Slot::Fenced));
    }
    assert_eq!(
        Records(m).require(&secret_key(id), "key marker").unwrap(),
        b""
    );
}
