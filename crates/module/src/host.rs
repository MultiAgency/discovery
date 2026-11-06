//! [`Platform`] on an OutLayer worker.

use discovery_protocol::{Error, Result};
use outlayer::{encryption_keys, storage};

use crate::platform::{Mode, Platform};

/// The project-bound key declared in `manifest.json`.
const KEY: &str = "survey";

pub struct Host;

fn storage_err(e: storage::StorageError) -> Error {
    Error(format!("storage: {}", e.0))
}

fn key_err(e: encryption_keys::EncryptionKeyError) -> Error {
    Error(format!("key: {}", e.0))
}

impl Platform for Host {
    fn mode(&self) -> Result<Mode> {
        match std::env::var("OUTLAYER_EXECUTION_TYPE").as_deref() {
            Ok("NEAR") => {
                let ts = std::env::var("NEAR_BLOCK_TIMESTAMP").unwrap_or_default();
                let block_ns = ts
                    .parse()
                    .map_err(|_| Error("missing block timestamp".into()))?;
                Ok(Mode::Near { block_ns })
            }
            Ok("HTTPS") => Ok(Mode::Https),
            other => Err(Error(format!("unknown execution type {other:?}"))),
        }
    }

    fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        encryption_keys::encrypt(KEY, None, plaintext, aad).map_err(key_err)
    }

    fn decrypt(&self, ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        encryption_keys::decrypt(KEY, None, ciphertext, aad).map_err(key_err)
    }

    fn mac(&self, data: &[u8]) -> Result<Vec<u8>> {
        encryption_keys::mac(KEY, None, data).map_err(key_err)
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        storage::get_raw(key).map_err(storage_err)
    }

    #[cfg(test)]
    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        storage::set_raw(key, value).map_err(storage_err)
    }

    fn set_if_absent(&self, key: &str, value: &[u8]) -> Result<bool> {
        storage::set_if_absent_raw(key, value).map_err(storage_err)
    }

    fn compare_and_swap(&self, key: &str, expected: &[u8], value: &[u8]) -> Result<bool> {
        storage::set_if_equals_raw(key, expected, value)
            .map(|(swapped, _)| swapped)
            .map_err(storage_err)
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        storage::list_keys(prefix).map_err(storage_err)
    }

    fn delete(&self, key: &str) -> Result<bool> {
        storage::delete(key).map_err(storage_err)
    }
}
