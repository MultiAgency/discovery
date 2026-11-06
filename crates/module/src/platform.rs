//! The boundary between survey logic and OutLayer.
//!
//! [`Platform`] is what the logic needs from the host: run facts, the
//! project-bound key, and raw storage. [`Records`] layers sealing on top so
//! every stored value is encrypted under the project key and bound to its own
//! storage key.

use discovery_protocol::{Error, Result};
use outlayer::{encryption_keys, storage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// On-chain run, with the trusted block timestamp in nanoseconds.
    Near { block_ns: u64 },
    /// HTTPS call. No trusted time.
    Https,
}

pub trait Platform {
    fn mode(&self) -> Result<Mode>;
    fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>>;
    fn decrypt(&self, ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>>;
    fn mac(&self, data: &[u8]) -> Result<Vec<u8>>;
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    #[cfg(test)]
    fn set(&self, key: &str, value: &[u8]) -> Result<()>;
    fn set_if_absent(&self, key: &str, value: &[u8]) -> Result<bool>;
    /// Compare the ciphertext read from this key, never re-encrypted plaintext.
    fn compare_and_swap(&self, key: &str, expected: &[u8], value: &[u8]) -> Result<bool>;
    fn delete(&self, key: &str) -> Result<bool>;
}

/// Sealed records: value = encrypt(plaintext, aad = storage key).
pub struct Records<'a, P: Platform>(pub &'a P);

impl<P: Platform> Records<'_, P> {
    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match self.0.get(key)? {
            Some(sealed) => Ok(Some(self.0.decrypt(&sealed, key.as_bytes())?)),
            None => Ok(None),
        }
    }

    pub fn set_if_absent(&self, key: &str, plaintext: &[u8]) -> Result<bool> {
        self.0
            .set_if_absent(key, &self.0.encrypt(plaintext, key.as_bytes())?)
    }

    pub fn require(&self, key: &str, what: &str) -> Result<Vec<u8>> {
        self.get(key)?
            .ok_or_else(|| Error(format!("{what} not found")))
    }
}

/// OutLayer worker implementation of the survey platform.
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

    fn delete(&self, key: &str) -> Result<bool> {
        storage::delete(key).map_err(storage_err)
    }
}

#[cfg(test)]
pub mod memory {
    //! In-memory platform for native tests. "Encryption" is a reversible tag
    //! check so tests catch aad mismatches. Views share one store, each with
    //! its own mode and write hook, so tests can interleave concurrent runs.

    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;
    use std::rc::Rc;

    pub struct Memory<'a> {
        pub mode: Mode,
        pub store: Rc<RefCell<BTreeMap<String, Vec<u8>>>>,
        /// Storage calls made, the resource OutLayer limits per run.
        pub calls: Rc<Cell<usize>>,
        /// A conditional write to a key under `prefix` first runs
        /// `before_write`, then fails while `failures` is positive.
        pub prefix: String,
        pub before_write: Box<dyn Fn(&str) + 'a>,
        pub failures: Cell<usize>,
    }

    impl Memory<'_> {
        pub fn new(mode: Mode) -> Self {
            Self::with_store(mode, BTreeMap::new())
        }

        /// Independent storage holding a copy of `store`.
        pub fn with_store(mode: Mode, store: BTreeMap<String, Vec<u8>>) -> Self {
            Self {
                mode,
                store: Rc::new(RefCell::new(store)),
                calls: Rc::default(),
                prefix: String::new(),
                before_write: Box::new(|_| {}),
                failures: Cell::new(0),
            }
        }

        /// Another run on the same storage.
        pub fn view<'b>(&self, mode: Mode) -> Memory<'b> {
            Memory {
                store: self.store.clone(),
                calls: self.calls.clone(),
                ..Memory::new(mode)
            }
        }

        fn call(&self) {
            self.calls.set(self.calls.get() + 1);
        }

        fn intercept(&self, key: &str) -> Result<()> {
            if self.prefix.is_empty() || !key.starts_with(&self.prefix) {
                return Ok(());
            }
            (self.before_write)(key);
            if self.failures.get() > 0 {
                self.failures.set(self.failures.get() - 1);
                return Err(Error("injected storage failure".into()));
            }
            Ok(())
        }

        /// Raw keys under `prefix`. Tests only: the module never lists storage.
        pub fn list(&self, prefix: &str) -> Vec<String> {
            self.store
                .borrow()
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect()
        }
    }

    impl Platform for Memory<'_> {
        fn mode(&self) -> Result<Mode> {
            Ok(self.mode)
        }
        fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
            let mut out = (aad.len() as u32).to_le_bytes().to_vec();
            out.extend_from_slice(aad);
            out.extend_from_slice(plaintext);
            Ok(out)
        }
        fn decrypt(&self, ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
            let n = u32::from_le_bytes(ciphertext[..4].try_into().unwrap()) as usize;
            if &ciphertext[4..4 + n] != aad {
                return Err(Error("decryption failed".into()));
            }
            Ok(ciphertext[4 + n..].to_vec())
        }
        fn mac(&self, data: &[u8]) -> Result<Vec<u8>> {
            use std::hash::{DefaultHasher, Hash, Hasher};
            let mut h = DefaultHasher::new();
            data.hash(&mut h);
            Ok(h.finish().to_le_bytes().to_vec())
        }
        fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
            self.call();
            Ok(self.store.borrow().get(key).cloned())
        }
        fn set(&self, key: &str, value: &[u8]) -> Result<()> {
            self.store.borrow_mut().insert(key.into(), value.into());
            Ok(())
        }
        fn set_if_absent(&self, key: &str, value: &[u8]) -> Result<bool> {
            self.intercept(key)?;
            self.call();
            let mut s = self.store.borrow_mut();
            if s.contains_key(key) {
                return Ok(false);
            }
            s.insert(key.into(), value.into());
            Ok(true)
        }
        fn compare_and_swap(&self, key: &str, expected: &[u8], value: &[u8]) -> Result<bool> {
            self.intercept(key)?;
            self.call();
            let mut store = self.store.borrow_mut();
            if store.get(key).map(Vec::as_slice) != Some(expected) {
                return Ok(false);
            }
            store.insert(key.into(), value.into());
            Ok(true)
        }
        fn delete(&self, key: &str) -> Result<bool> {
            self.call();
            Ok(self.store.borrow_mut().remove(key).is_some())
        }
    }
}
