//! The users of the connector's web interface (#165): the customer's people
//! who open and close access. They exist only on this connector, in
//! `users.json` in the data directory, and are managed on the command line.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use data_encoding::BASE32_NOPAD;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct User {
    /// Argon2id in PHC form.
    password: String,
    /// Base32; none without a second factor.
    totp: Option<String>,
}

/// What a new or reset user gets, shown once.
pub struct Issued {
    pub password: Zeroizing<String>,
    /// The TOTP secret in base32 and its `otpauth://` URI.
    pub totp: Option<(Zeroizing<String>, Zeroizing<String>)>,
}

#[derive(Debug, thiserror::Error)]
pub enum UserError {
    #[error("{0} exists already")]
    Exists(String),
    #[error("there is no user {0}")]
    Unknown(String),
    #[error("a name has 1 to 64 letters, digits, dots, dashes or underscores")]
    InvalidName,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Why a sign-in failed; the web interface says the same for both.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    WrongCredentials,
    /// Name and password are right, the TOTP code is missing or wrong.
    WrongCode,
}

pub struct Users {
    path: PathBuf,
}

impl Users {
    pub fn new(dir: &Path) -> Users {
        Users {
            path: dir.join("users.json"),
        }
    }

    fn load(&self) -> io::Result<BTreeMap<String, User>> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(error) => Err(error),
        }
    }

    fn save(&self, users: &BTreeMap<String, User>) -> io::Result<()> {
        let temporary = self.path.with_extension("json.new");
        crate::files::write_private(&temporary, &serde_json::to_vec_pretty(users)?)?;
        std::fs::rename(&temporary, &self.path)
    }

    /// The names, and whether each has a second factor.
    pub fn list(&self) -> io::Result<Vec<(String, bool)>> {
        Ok(self
            .load()?
            .into_iter()
            .map(|(name, user)| (name, user.totp.is_some()))
            .collect())
    }

    pub fn add(&self, name: &str, with_totp: bool) -> Result<Issued, UserError> {
        valid(name)?;
        let mut users = self.load()?;
        if users.contains_key(name) {
            return Err(UserError::Exists(name.to_owned()));
        }
        let (user, issued) = issue(name, with_totp);
        users.insert(name.to_owned(), user);
        self.save(&users)?;
        Ok(issued)
    }

    /// A new password, and a new TOTP secret or none.
    pub fn reset(&self, name: &str, with_totp: bool) -> Result<Issued, UserError> {
        let mut users = self.load()?;
        let Some(user) = users.get_mut(name) else {
            return Err(UserError::Unknown(name.to_owned()));
        };
        let (replaced, issued) = issue(name, with_totp);
        *user = replaced;
        self.save(&users)?;
        Ok(issued)
    }

    pub fn delete(&self, name: &str) -> Result<(), UserError> {
        let mut users = self.load()?;
        if users.remove(name).is_none() {
            return Err(UserError::Unknown(name.to_owned()));
        }
        self.save(&users)?;
        Ok(())
    }

    /// Checks name, password and, for a user with a second factor, the code.
    pub fn verify(&self, name: &str, password: &str, code: &str) -> Result<(), Refused> {
        let users = self.load().unwrap_or_default();
        let Some(user) = users.get(name) else {
            // As long as a real check, so the time does not tell which names
            // exist.
            let _ = check_password(&DUMMY_HASH, password);
            return Err(Refused::WrongCredentials);
        };
        if !check_password(&user.password, password) {
            return Err(Refused::WrongCredentials);
        }
        let Some(secret) = &user.totp else {
            return Ok(());
        };
        let secret = remotehub_totp::decode(secret).ok_or(Refused::WrongCode)?;
        remotehub_totp::step(&secret, code.trim(), remotehub_totp::unix_now())
            .map(|_| ())
            .ok_or(Refused::WrongCode)
    }
}

/// Argon2id of a random password nobody knows, for unknown names.
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| hash(&random_password()));

fn hash(password: &str) -> String {
    Argon2::default()
        .hash_password(password.as_bytes())
        .expect("Argon2 hashes any password")
        .to_string()
}

/// 120 bits in 24 characters that survive being read out on the phone.
fn random_password() -> Zeroizing<String> {
    let mut random = Zeroizing::new([0u8; 15]);
    getrandom::fill(random.as_mut_slice()).expect("the OS has randomness");
    Zeroizing::new(BASE32_NOPAD.encode(random.as_slice()).to_lowercase())
}

fn check_password(hash: &str, password: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

fn valid(name: &str) -> Result<(), UserError> {
    let fits = (1..=64).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    fits.then_some(()).ok_or(UserError::InvalidName)
}

fn issue(name: &str, with_totp: bool) -> (User, Issued) {
    let password = random_password();
    let totp = with_totp.then(|| {
        let secret = remotehub_totp::encode(remotehub_totp::random_secret().as_slice());
        let uri = remotehub_totp::uri("remotehub-connector", name, &secret);
        (secret, uri)
    });
    let user = User {
        password: hash(&password),
        totp: totp.as_ref().map(|(secret, _)| secret.to_string()),
    };
    (user, Issued { password, totp })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn users_sign_in_with_what_they_were_given() {
        let dir = tempfile::tempdir().unwrap();
        let users = Users::new(dir.path());
        let plain = users.add("alice", false).unwrap();
        assert!(plain.totp.is_none());
        assert_eq!(users.verify("alice", &plain.password, ""), Ok(()));
        assert_eq!(
            users.verify("alice", "wrong", ""),
            Err(Refused::WrongCredentials)
        );
        assert_eq!(
            users.verify("nobody", &plain.password, ""),
            Err(Refused::WrongCredentials)
        );
        assert!(matches!(
            users.add("alice", false),
            Err(UserError::Exists(_))
        ));
        assert!(matches!(
            users.add("a b", false),
            Err(UserError::InvalidName)
        ));

        let second = users.add("bob", true).unwrap();
        let (secret, uri) = second.totp.as_ref().unwrap();
        assert!(uri.starts_with("otpauth://totp/remotehub-connector:bob?"));
        assert_eq!(
            users.verify("bob", &second.password, ""),
            Err(Refused::WrongCode)
        );
        let secret = remotehub_totp::decode(secret).unwrap();
        let code =
            remotehub_totp::code(&secret, remotehub_totp::unix_now() / remotehub_totp::PERIOD);
        assert_eq!(users.verify("bob", &second.password, &code), Ok(()));

        let reset = users.reset("alice", false).unwrap();
        assert_eq!(
            users.verify("alice", &plain.password, ""),
            Err(Refused::WrongCredentials)
        );
        assert_eq!(users.verify("alice", &reset.password, ""), Ok(()));
        assert_eq!(
            users.list().unwrap(),
            [("alice".to_owned(), false), ("bob".to_owned(), true)]
        );
        users.delete("alice").unwrap();
        assert!(matches!(users.delete("alice"), Err(UserError::Unknown(_))));
    }
}
