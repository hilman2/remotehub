//! Master keys from a file (mounted as a Docker secret).
//!
//! The file holds one key per line as `<version>:<base64 of 32 bytes>`;
//! empty lines and lines starting with `#` are ignored. The highest version
//! is current. Rotating means appending a line with a new version
//! (`remotehub generate-key`) and restarting; older versions stay for opening
//! existing values until they are rewrapped.

use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use zeroize::Zeroizing;

use crate::{KEY_LEN, Key, KeyProvider, VaultError, random_key};

/// The provider id stored with every value sealed under a file key.
pub const FILE_KEYRING_ID: &str = "file";

pub struct FileKeyring {
    keys: BTreeMap<i32, Key>,
    current: i32,
}

impl FileKeyring {
    pub fn load(path: &Path) -> Result<Self, VaultError> {
        let content = Zeroizing::new(
            std::fs::read_to_string(path)
                .map_err(|e| VaultError::KeyFile(format!("{}: {e}", path.display())))?,
        );
        Self::parse(&content)
    }

    pub fn parse(content: &str) -> Result<Self, VaultError> {
        let mut keys = BTreeMap::new();
        for (number, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let invalid = || {
                VaultError::KeyFile(format!("line {} is not <version>:<base64 key>", number + 1))
            };
            let (version, encoded) = line.split_once(':').ok_or_else(invalid)?;
            let version: i32 = version.trim().parse().map_err(|_| invalid())?;
            if version < 1 {
                return Err(invalid());
            }
            let bytes = Zeroizing::new(STANDARD.decode(encoded.trim()).map_err(|_| invalid())?);
            let key: [u8; KEY_LEN] = bytes.as_slice().try_into().map_err(|_| {
                VaultError::KeyFile(format!("line {}: a key has {KEY_LEN} bytes", number + 1))
            })?;
            if keys.insert(version, Zeroizing::new(key)).is_some() {
                return Err(VaultError::KeyFile(format!(
                    "version {version} appears twice"
                )));
            }
        }
        let current = *keys
            .keys()
            .next_back()
            .ok_or_else(|| VaultError::KeyFile("no key found".into()))?;
        Ok(FileKeyring { keys, current })
    }
}

impl KeyProvider for FileKeyring {
    fn current(&self) -> (&str, i32) {
        (FILE_KEYRING_ID, self.current)
    }

    fn master_key(&self, id: &str, version: i32) -> Result<&Key, VaultError> {
        self.keys
            .get(&version)
            .filter(|_| id == FILE_KEYRING_ID)
            .ok_or(VaultError::UnknownKey(id.to_owned(), version))
    }

    fn all(&self) -> Vec<(&str, i32)> {
        self.keys
            .keys()
            .map(|version| (FILE_KEYRING_ID, *version))
            .collect()
    }
}

/// A new line for the key file: `<version>:<base64 key>`.
pub fn generate_key_line(version: i32) -> Zeroizing<String> {
    let key = random_key();
    Zeroizing::new(format!("{version}:{}", STANDARD.encode(key.as_slice())))
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::{Context, Vault};

    #[test]
    fn the_highest_version_is_current() {
        let file = format!(
            "# remotehub master keys\n{}\n\n{}\n",
            generate_key_line(1).as_str(),
            generate_key_line(2).as_str()
        );
        let keyring = FileKeyring::parse(&file).unwrap();
        assert_eq!(keyring.current(), ("file", 2));
        assert!(keyring.master_key("file", 1).is_ok());
        assert!(keyring.master_key("file", 3).is_err());
        assert!(keyring.master_key("other", 1).is_err());
    }

    #[test]
    fn rejects_broken_files() {
        let short = format!("1:{}", STANDARD.encode([0u8; 16]));
        let twice = format!("{0}\n{0}", generate_key_line(1).as_str());
        for bad in [
            "",
            "# only a comment",
            "1:not base64!",
            "x:AAAA",
            "0:AAAA",
            &short,
            &twice,
        ] {
            assert!(FileKeyring::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn loads_from_disk_and_seals() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), generate_key_line(1).as_str()).unwrap();
        let vault = Vault::new(FileKeyring::load(file.path()).unwrap());
        let context = Context {
            owner: Uuid::nil(),
            version: 1,
            field: "password",
        };
        let sealed = vault.seal(context, b"x").unwrap();
        assert_eq!(sealed.kek_id, "file");
        assert_eq!(vault.open(context, &sealed).unwrap().as_slice(), b"x");
        assert!(FileKeyring::load(Path::new("/does/not/exist")).is_err());
    }
}
