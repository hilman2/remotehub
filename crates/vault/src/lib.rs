//! Encryption of stored secrets (ADR 0004).
//!
//! Every secret field is sealed with its own random 256-bit data key using
//! XChaCha20-Poly1305. The data key is wrapped by the current master key of a
//! [`KeyProvider`]. The associated data binds a ciphertext to its scheme,
//! owner, version and field and to the master key version, so it cannot be
//! moved to another row or field without failing to open.
//!
//! The server needs to open secrets to inject them into sessions, so this is
//! not zero-knowledge: whoever holds the master key and the database can read
//! them. Protect the key file accordingly.

mod keyring;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use thiserror::Error;
use uuid::Uuid;
use zeroize::Zeroizing;

pub use keyring::{FileKeyring, generate_key_line};

/// Secrets sealed with a key the server does not keep — for example one that
/// lives only in the user's cookie. The context (e.g. a session's token hash)
/// is bound in as associated data.
pub mod detached {
    use zeroize::Zeroizing;

    use crate::{Key, VaultError};

    fn aad(context: &[u8]) -> Vec<u8> {
        [b"remotehub|detached|".as_slice(), context].concat()
    }

    pub fn new_key() -> Key {
        crate::random_key()
    }

    pub fn seal(key: &Key, context: &[u8], plaintext: &[u8]) -> Vec<u8> {
        crate::encrypt(key, &aad(context), plaintext)
    }

    pub fn open(
        key: &Key,
        context: &[u8],
        sealed: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, VaultError> {
        crate::decrypt(key, &aad(context), sealed)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn opens_only_with_the_same_key_and_context() {
            let key = new_key();
            let sealed = seal(&key, b"session-a", b"Passw0rd!");
            assert!(!sealed.windows(9).any(|w| w == b"Passw0rd!"));
            assert_eq!(
                open(&key, b"session-a", &sealed).unwrap().as_slice(),
                b"Passw0rd!"
            );
            assert_eq!(open(&key, b"session-b", &sealed), Err(VaultError::Open));
            assert_eq!(
                open(&new_key(), b"session-a", &sealed),
                Err(VaultError::Open)
            );
        }
    }
}

/// Name of the sealing scheme stored with every sealed value.
pub const SCHEME: &str = "xchacha20poly1305-v1";

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

/// A 256-bit key that is wiped from memory when dropped.
pub type Key = Zeroizing<[u8; KEY_LEN]>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VaultError {
    /// Wrong key, tampered ciphertext, or a ciphertext from another place.
    #[error("cannot open the sealed value")]
    Open,
    #[error("unknown scheme {0}")]
    UnknownScheme(String),
    #[error("master key {0} version {1} is not available")]
    UnknownKey(String, i32),
    #[error("key file: {0}")]
    KeyFile(String),
}

/// Source of master keys: wraps and unwraps data keys. Keys have an id (the
/// provider) and a version; new data is always wrapped with the current one.
pub trait KeyProvider: Send + Sync {
    fn current(&self) -> (&str, i32);
    fn master_key(&self, id: &str, version: i32) -> Result<&Key, VaultError>;
}

impl<T: KeyProvider + ?Sized> KeyProvider for Box<T> {
    fn current(&self) -> (&str, i32) {
        (**self).current()
    }

    fn master_key(&self, id: &str, version: i32) -> Result<&Key, VaultError> {
        (**self).master_key(id, version)
    }
}

/// A vault over whichever key provider is configured.
pub type DynVault = Vault<Box<dyn KeyProvider>>;

/// Where a sealed value belongs; part of the associated data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context<'a> {
    /// The entry the secret belongs to (credential, user …).
    pub owner: Uuid,
    /// Version of the entry; every change creates a new version.
    pub version: i32,
    /// Field within the entry, e.g. `password`.
    pub field: &'a str,
}

/// A sealed value as stored in the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    pub scheme: String,
    pub kek_id: String,
    pub kek_version: i32,
    /// Data key encrypted with the master key: nonce ‖ ciphertext.
    pub wrapped_key: Vec<u8>,
    /// Value encrypted with the data key: nonce ‖ ciphertext.
    pub ciphertext: Vec<u8>,
}

pub struct Vault<K: KeyProvider> {
    keys: K,
}

impl<K: KeyProvider> Vault<K> {
    pub fn new(keys: K) -> Self {
        Vault { keys }
    }

    pub fn keys(&self) -> &K {
        &self.keys
    }

    pub fn seal(&self, context: Context<'_>, plaintext: &[u8]) -> Result<Sealed, VaultError> {
        let (kek_id, kek_version) = self.keys.current();
        let master = self.keys.master_key(kek_id, kek_version)?;
        let data_key = random_key();
        let aad = associated_data(context, kek_id, kek_version);
        Ok(Sealed {
            scheme: SCHEME.to_owned(),
            kek_id: kek_id.to_owned(),
            kek_version,
            wrapped_key: encrypt(master, &aad, data_key.as_slice()),
            ciphertext: encrypt(&data_key, &aad, plaintext),
        })
    }

    /// The plaintext, wiped from memory when the returned buffer is dropped.
    pub fn open(
        &self,
        context: Context<'_>,
        sealed: &Sealed,
    ) -> Result<Zeroizing<Vec<u8>>, VaultError> {
        if sealed.scheme != SCHEME {
            return Err(VaultError::UnknownScheme(sealed.scheme.clone()));
        }
        let master = self.keys.master_key(&sealed.kek_id, sealed.kek_version)?;
        let aad = associated_data(context, &sealed.kek_id, sealed.kek_version);
        let data_key = decrypt(master, &aad, &sealed.wrapped_key)?;
        let data_key: Key = Zeroizing::new(
            data_key
                .as_slice()
                .try_into()
                .map_err(|_| VaultError::Open)?,
        );
        decrypt(&data_key, &aad, &sealed.ciphertext)
    }

    /// Wraps the data key with the current master key, without touching the
    /// value itself (key rotation). Returns `None` if nothing changes.
    pub fn rewrap(
        &self,
        context: Context<'_>,
        sealed: &Sealed,
    ) -> Result<Option<Sealed>, VaultError> {
        let (kek_id, kek_version) = self.keys.current();
        if sealed.kek_id == kek_id && sealed.kek_version == kek_version {
            return Ok(None);
        }
        // The associated data includes the master key version, so the value
        // is sealed afresh under the new key.
        let plaintext = self.open(context, sealed)?;
        self.seal(context, &plaintext).map(Some)
    }
}

fn associated_data(context: Context<'_>, kek_id: &str, kek_version: i32) -> Vec<u8> {
    format!(
        "remotehub|{SCHEME}|{}|{}|{}|{kek_id}|{kek_version}",
        context.owner, context.version, context.field
    )
    .into_bytes()
}

fn random_key() -> Key {
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    getrandom::fill(key.as_mut_slice()).expect("the operating system provides randomness");
    key
}

fn encrypt(key: &Key, aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new_from_slice(key.as_slice()).expect("32-byte key");
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).expect("the operating system provides randomness");
    let ciphertext = cipher
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("encryption of in-memory data does not fail");
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    out
}

fn decrypt(key: &Key, aad: &[u8], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    if sealed.len() < NONCE_LEN {
        return Err(VaultError::Open);
    }
    let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| VaultError::Open)?;
    let cipher = XChaCha20Poly1305::new_from_slice(key.as_slice()).expect("32-byte key");
    cipher
        .decrypt(
            &XNonce::from(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| VaultError::Open)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two key versions in memory; version 2 is current unless told otherwise.
    struct TestKeys {
        keys: Vec<Key>,
        current: i32,
    }

    impl TestKeys {
        fn new(current: i32) -> Self {
            TestKeys {
                keys: vec![random_key(), random_key()],
                current,
            }
        }
    }

    impl KeyProvider for TestKeys {
        fn current(&self) -> (&str, i32) {
            ("test", self.current)
        }

        fn master_key(&self, id: &str, version: i32) -> Result<&Key, VaultError> {
            usize::try_from(version - 1)
                .ok()
                .and_then(|i| self.keys.get(i))
                .filter(|_| id == "test")
                .ok_or(VaultError::UnknownKey(id.to_owned(), version))
        }
    }

    const OWNER: Uuid = Uuid::from_u128(42);

    fn context(field: &str) -> Context<'_> {
        Context {
            owner: OWNER,
            version: 1,
            field,
        }
    }

    #[test]
    fn seals_and_opens() {
        let vault = Vault::new(TestKeys::new(2));
        let sealed = vault.seal(context("password"), b"Passw0rd!").unwrap();
        assert_eq!(sealed.scheme, SCHEME);
        assert_eq!((sealed.kek_id.as_str(), sealed.kek_version), ("test", 2));
        assert!(!sealed.ciphertext.windows(9).any(|w| w == b"Passw0rd!"));
        assert_eq!(
            vault.open(context("password"), &sealed).unwrap().as_slice(),
            b"Passw0rd!"
        );
    }

    #[test]
    fn every_seal_uses_fresh_keys_and_nonces() {
        let vault = Vault::new(TestKeys::new(1));
        let a = vault.seal(context("password"), b"same").unwrap();
        let b = vault.seal(context("password"), b"same").unwrap();
        assert_ne!(a.ciphertext, b.ciphertext);
        assert_ne!(a.wrapped_key, b.wrapped_key);
    }

    #[test]
    fn refuses_values_moved_to_another_place() {
        let vault = Vault::new(TestKeys::new(1));
        let sealed = vault.seal(context("password"), b"secret").unwrap();
        let elsewhere = [
            context("notes"),
            Context {
                owner: Uuid::from_u128(43),
                ..context("password")
            },
            Context {
                version: 2,
                ..context("password")
            },
        ];
        for other in elsewhere {
            assert_eq!(
                vault.open(other, &sealed).unwrap_err(),
                VaultError::Open,
                "{other:?}"
            );
        }
        // Claiming another master key version fails too.
        let relabelled = Sealed {
            kek_version: 2,
            ..sealed
        };
        assert_eq!(
            vault.open(context("password"), &relabelled).unwrap_err(),
            VaultError::Open
        );
    }

    #[test]
    fn refuses_tampering_and_unknown_schemes() {
        let vault = Vault::new(TestKeys::new(1));
        let sealed = vault.seal(context("password"), b"secret").unwrap();

        let mut flipped = sealed.clone();
        *flipped.ciphertext.last_mut().unwrap() ^= 1;
        assert_eq!(
            vault.open(context("password"), &flipped).unwrap_err(),
            VaultError::Open
        );

        let mut short = sealed.clone();
        short.wrapped_key.truncate(10);
        assert_eq!(
            vault.open(context("password"), &short).unwrap_err(),
            VaultError::Open
        );

        let other_scheme = Sealed {
            scheme: "rot13".into(),
            ..sealed
        };
        assert!(matches!(
            vault.open(context("password"), &other_scheme),
            Err(VaultError::UnknownScheme(_))
        ));
    }

    #[test]
    fn another_master_key_cannot_open() {
        let sealed = Vault::new(TestKeys::new(1))
            .seal(context("password"), b"secret")
            .unwrap();
        let stranger = Vault::new(TestKeys::new(1));
        assert_eq!(
            stranger.open(context("password"), &sealed).unwrap_err(),
            VaultError::Open
        );
    }

    #[test]
    fn rotation_rewraps_to_the_current_key() {
        let mut keys = TestKeys::new(1);
        let sealed = Vault::new(TestKeys {
            keys: keys.keys.clone(),
            current: 1,
        })
        .seal(context("password"), b"secret")
        .unwrap();
        keys.current = 2;
        let vault = Vault::new(keys);
        // Old values stay readable after rotation …
        assert_eq!(
            vault.open(context("password"), &sealed).unwrap().as_slice(),
            b"secret"
        );
        // … and can be moved to the new key.
        let rewrapped = vault.rewrap(context("password"), &sealed).unwrap().unwrap();
        assert_eq!(rewrapped.kek_version, 2);
        assert_eq!(
            vault
                .open(context("password"), &rewrapped)
                .unwrap()
                .as_slice(),
            b"secret"
        );
        assert_eq!(vault.rewrap(context("password"), &rewrapped).unwrap(), None);
    }
}
