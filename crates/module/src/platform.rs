//! The boundary between survey logic and OutLayer.
//!
//! [`Platform`] is what the logic needs from the host: run facts, the
//! project-bound key, and raw storage. [`Records`] layers sealing on top so
//! every stored value is encrypted under the project key and bound to its own
//! storage key.

use discovery_protocol::{Error, Result};

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
    fn list(&self, prefix: &str) -> Result<Vec<String>>;
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

#[cfg(test)]
pub mod memory {
    //! In-memory platform for native tests. "Encryption" is a reversible tag
    //! check so tests catch aad mismatches.

    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    pub struct Memory {
        pub mode: Mode,
        pub store: RefCell<BTreeMap<String, Vec<u8>>>,
    }

    impl Memory {
        pub fn new(mode: Mode) -> Self {
            Self {
                mode,
                store: RefCell::default(),
            }
        }
    }

    impl Platform for Memory {
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
            Ok(self.store.borrow().get(key).cloned())
        }
        fn set(&self, key: &str, value: &[u8]) -> Result<()> {
            self.store.borrow_mut().insert(key.into(), value.into());
            Ok(())
        }
        fn set_if_absent(&self, key: &str, value: &[u8]) -> Result<bool> {
            let mut s = self.store.borrow_mut();
            if s.contains_key(key) {
                return Ok(false);
            }
            s.insert(key.into(), value.into());
            Ok(true)
        }
        fn compare_and_swap(&self, key: &str, expected: &[u8], value: &[u8]) -> Result<bool> {
            let mut store = self.store.borrow_mut();
            if store.get(key).map(Vec::as_slice) != Some(expected) {
                return Ok(false);
            }
            store.insert(key.into(), value.into());
            Ok(true)
        }
        fn list(&self, prefix: &str) -> Result<Vec<String>> {
            Ok(self
                .store
                .borrow()
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect())
        }
        fn delete(&self, key: &str) -> Result<bool> {
            Ok(self.store.borrow_mut().remove(key).is_some())
        }
    }
}

#[cfg(test)]
pub mod interleaving {
    use super::memory::Memory;
    use super::*;
    use std::cell::{Cell, RefCell};

    /// Share one storage cell across modes and pause one response write.
    pub struct View<'a> {
        pub inner: &'a Memory,
        pub mode: Mode,
        pub prefix: String,
        pub failures: Cell<usize>,
        pub before_write: RefCell<Option<Box<dyn FnOnce() + 'a>>>,
    }
    impl<'a> View<'a> {
        pub fn new(inner: &'a Memory, mode: Mode) -> Self {
            Self {
                inner,
                mode,
                prefix: String::new(),
                failures: Cell::new(0),
                before_write: RefCell::new(None),
            }
        }
        fn pause(&self, key: &str) -> Result<()> {
            if !self.prefix.is_empty() && key.starts_with(&self.prefix) {
                let hook = self.before_write.borrow_mut().take();
                if let Some(hook) = hook {
                    hook();
                }
                if self.failures.get() > 0 {
                    self.failures.set(self.failures.get() - 1);
                    return Err(Error("injected storage failure".into()));
                }
            }
            Ok(())
        }
    }
    impl Platform for View<'_> {
        fn mode(&self) -> Result<Mode> {
            Ok(self.mode)
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
            self.inner.get(k)
        }
        fn set(&self, k: &str, v: &[u8]) -> Result<()> {
            self.inner.set(k, v)
        }
        fn set_if_absent(&self, k: &str, v: &[u8]) -> Result<bool> {
            self.pause(k)?;
            self.inner.set_if_absent(k, v)
        }
        fn compare_and_swap(&self, k: &str, e: &[u8], v: &[u8]) -> Result<bool> {
            self.pause(k)?;
            self.inner.compare_and_swap(k, e, v)
        }
        fn list(&self, p: &str) -> Result<Vec<String>> {
            self.inner.list(p)
        }
        fn delete(&self, k: &str) -> Result<bool> {
            self.inner.delete(k)
        }
    }
}
