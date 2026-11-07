//! The release pinned by `releases.json`, compiled into the CLI.

use serde::Deserialize;

use crate::Result;

/// The audited build, the organizer it runs as, and the transactions that
/// prove its clean bootstrap (checked on-chain by [`crate::bootstrap`]).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    /// `owner/name`; the owner is a named subaccount.
    pub project: String,
    #[serde(rename = "release")]
    pub wasm_hash: String,
    pub organizer: String,
    pub bootstrap: Bootstrap,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bootstrap {
    pub owner_seed_block: String,
    pub owner_init_tx: String,
    pub project_init_tx: String,
    pub lock_tx: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    service: Release,
}

impl Release {
    /// The registered release, refused before any network or paid request
    /// unless every pin is well formed.
    pub fn registered(registry: &str) -> Result<Release> {
        let release = serde_json::from_str::<Registry>(registry)
            .map_err(|e| format!("releases.json does not register a release: {e}"))?
            .service;
        let (owner, name) = release.project.split_once('/').unwrap_or_default();
        if !account_id(owner)
            || !owner.contains('.')
            || name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || !account_id(&release.organizer)
            || owner == release.organizer
            || !is_hex(&release.wasm_hash, 32)
        {
            return Err("invalid registered project, build or organizer".into());
        }
        let proof = &release.bootstrap;
        if !tx_hash(&proof.owner_seed_block) {
            return Err("invalid registered bootstrap seed block".into());
        }
        let hashes = [&proof.owner_init_tx, &proof.project_init_tx, &proof.lock_tx];
        if hashes.iter().any(|hash| !tx_hash(hash))
            || hashes[0] == hashes[1]
            || hashes[0] == hashes[2]
            || hashes[1] == hashes[2]
        {
            return Err("invalid registered bootstrap transactions".into());
        }
        Ok(release)
    }

    /// The project's owner account.
    pub fn owner(&self) -> &str {
        self.project.split_once('/').unwrap_or_default().0
    }

    /// The project's name under its owner.
    pub fn name(&self) -> &str {
        self.project.split_once('/').unwrap_or_default().1
    }

    /// The account that created the owner.
    pub fn parent(&self) -> &str {
        self.owner().split_once('.').unwrap_or_default().1
    }
}

fn account_id(id: &str) -> bool {
    (2..=64).contains(&id.len())
        && id.split(['.', '-', '_']).all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

pub fn is_hex(s: &str, bytes: usize) -> bool {
    s.len() == 2 * bytes && s.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn tx_hash(hash: &str) -> bool {
    const BASE58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    (32..=44).contains(&hash.len()) && hash.bytes().all(|b| BASE58.contains(&b))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::{json, Value};

    pub const RELEASE: &str = "5f1e3c2a9b8d7e6f5f1e3c2a9b8d7e6f5f1e3c2a9b8d7e6f5f1e3c2a9b8d7e6f";

    pub fn registry() -> Value {
        json!({"service": {"project": "owner.testnet/discovery", "release": RELEASE,
        "organizer": "organizer.testnet", "bootstrap": {
            "owner_seed_block": "4".repeat(43),
            "owner_init_tx": "1".repeat(43),
            "project_init_tx": "2".repeat(43),
            "lock_tx": "3".repeat(43)
        }}})
    }

    fn registered(registry: &Value) -> Result<Release> {
        Release::registered(&registry.to_string())
    }

    #[test]
    fn the_compiled_registry_is_valid() {
        Release::registered(crate::REGISTRY).unwrap();
    }

    #[test]
    fn registry_entries_parse_strictly() {
        let release = registered(&registry()).unwrap();
        assert_eq!(release.wasm_hash, RELEASE);
        assert_eq!(
            (release.owner(), release.name(), release.parent()),
            ("owner.testnet", "discovery", "testnet")
        );
        for bad in [json!({"service": null}), json!({})] {
            assert!(registered(&bad).is_err());
        }
        for (path, extra) in [("", "unexpected"), ("/service", "network")] {
            let mut bad = registry();
            bad.pointer_mut(path).unwrap()[extra] = json!(true);
            let error = registered(&bad).err().unwrap().to_string();
            assert!(error.contains("unknown field"), "{error}");
        }
    }

    #[test]
    fn incomplete_or_ambiguous_release_provenance_is_refused_offline() {
        let valid = registry();
        for field in ["project", "release", "organizer"] {
            let mut bad = valid.clone();
            bad["service"][field] = json!("invalid//value");
            assert!(registered(&bad).is_err());
        }
        for (field, value) in [
            ("organizer", "owner.testnet"),
            // The bootstrap derives the owner's name under a parent account.
            ("project", "owner/discovery"),
        ] {
            let mut bad = valid.clone();
            bad["service"][field] = json!(value);
            assert!(registered(&bad).is_err(), "{field}: {value}");
        }
        let mut bad = valid.clone();
        bad["service"].as_object_mut().unwrap().remove("bootstrap");
        assert!(registered(&bad).is_err());
        let mut bad = valid.clone();
        bad["service"]["bootstrap"]
            .as_object_mut()
            .unwrap()
            .remove("owner_seed_block");
        assert!(registered(&bad).is_err());
        for seed in [
            "",
            "invalid",
            &"1".repeat(31),
            &"1".repeat(45),
            &"0".repeat(43),
        ] {
            let mut bad = valid.clone();
            bad["service"]["bootstrap"]["owner_seed_block"] = json!(seed);
            assert_eq!(
                registered(&bad).err().unwrap().to_string(),
                "invalid registered bootstrap seed block"
            );
        }
        for field in ["owner_init_tx", "project_init_tx", "lock_tx"] {
            let mut bad = valid.clone();
            bad["service"]["bootstrap"][field] = json!("invalid");
            assert!(registered(&bad).is_err());
        }
        let mut bad = valid;
        bad["service"]["bootstrap"]["lock_tx"] =
            bad["service"]["bootstrap"]["project_init_tx"].clone();
        assert!(registered(&bad).is_err());
    }
}
