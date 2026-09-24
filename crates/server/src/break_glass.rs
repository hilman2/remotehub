//! Break-glass accounts (ADR 0005): local accounts that work when the
//! directory does not — otherwise nobody gets at the passwords needed to
//! repair it.
//!
//! - Created, reset and deleted only via the CLI (`remotehub break-glass …`),
//!   never through the web UI. The CLI generates the password and the TOTP
//!   secret and shows them once.
//! - Sign-in needs the password (argon2id) and a TOTP code; a code is only
//!   accepted once (the last used time step is stored).
//! - The TOTP secret is sealed in the vault (owner = user, field `totp`).
//! - Break-glass users are administrators; every action is audited.

use std::sync::LazyLock;

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, KeyInit, Mac};
use remotehub_vault::DynVault;
use secrecy::{ExposeSecret, SecretString};
use sha1::Sha1;
use sqlx::PgPool;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::secrets::{self, SecretError};

const TOTP_FIELD: &str = "totp";
const TOTP_PERIOD: u64 = 30;
const TOTP_DIGITS: u32 = 6;
/// Alphabet for generated passwords: no look-alikes (0/O, 1/l/I).
const PASSWORD_ALPHABET: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789-_.!";
const PASSWORD_LENGTH: usize = 24;

#[derive(Debug, thiserror::Error)]
pub enum BreakGlassError {
    #[error("a break-glass account {0:?} already exists")]
    Exists(String),
    #[error("there is no break-glass account {0:?}")]
    Unknown(String),
    #[error("user names consist of 1 to 64 letters, digits, '.', '-' or '_'")]
    InvalidName,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error("password hashing failed: {0}")]
    Hash(String),
}

/// What the CLI shows once after creating or resetting an account.
pub struct Issued {
    pub username: String,
    pub password: Zeroizing<String>,
    /// Base32, for typing into an authenticator app.
    pub totp_secret: Zeroizing<String>,
    /// `otpauth://` URI, for a QR code.
    pub totp_uri: Zeroizing<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub user_id: Uuid,
    pub username: String,
    pub display_name: String,
}

fn valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn random_password() -> Zeroizing<String> {
    let mut bytes = Zeroizing::new([0u8; PASSWORD_LENGTH]);
    getrandom::fill(bytes.as_mut_slice()).expect("the operating system provides randomness");
    // 60 symbols: the modulo bias is negligible for a 24-character password.
    Zeroizing::new(
        bytes
            .iter()
            .map(|b| PASSWORD_ALPHABET[usize::from(*b) % PASSWORD_ALPHABET.len()] as char)
            .collect(),
    )
}

fn hash_password(password: &str) -> Result<String, BreakGlassError> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|e| BreakGlassError::Hash(e.to_string()))
}

fn password_matches(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

/// RFC 6238 code for a time step (HMAC-SHA1, 6 digits).
fn totp_code(secret: &[u8], step: u64) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("HMAC takes any key length");
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let value = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]) % 10u32.pow(TOTP_DIGITS);
    format!("{value:0width$}", width = TOTP_DIGITS as usize)
}

/// The time step a code belongs to, allowing one step of clock drift each way.
fn totp_step(secret: &[u8], code: &str, unix_seconds: u64) -> Option<u64> {
    let now = unix_seconds / TOTP_PERIOD;
    [now, now.saturating_sub(1), now + 1]
        .into_iter()
        .find(|step| {
            let expected = totp_code(secret, *step);
            // Constant-time comparison of equal-length ASCII codes.
            expected.len() == code.len()
                && expected
                    .bytes()
                    .zip(code.bytes())
                    .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                    == 0
        })
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is after 1970")
        .as_secs()
}

fn issue(username: &str, password: Zeroizing<String>, secret: &[u8]) -> Issued {
    let encoded = Zeroizing::new(BASE32_NOPAD.encode(secret));
    let uri = Zeroizing::new(format!(
        "otpauth://totp/remotehub:{username}?secret={}&issuer=remotehub&algorithm=SHA1&digits={TOTP_DIGITS}&period={TOTP_PERIOD}",
        encoded.as_str()
    ));
    Issued {
        username: username.to_owned(),
        password,
        totp_secret: encoded,
        totp_uri: uri,
    }
}

fn random_totp_secret() -> Zeroizing<[u8; 20]> {
    let mut secret = Zeroizing::new([0u8; 20]);
    getrandom::fill(secret.as_mut_slice()).expect("the operating system provides randomness");
    secret
}

pub async fn create(
    db: &PgPool,
    vault: &DynVault,
    username: &str,
) -> Result<Issued, BreakGlassError> {
    if !valid_name(username) {
        return Err(BreakGlassError::InvalidName);
    }
    let password = random_password();
    let secret = random_totp_secret();
    let hash = hash_password(&password)?;

    let mut tx = db.begin().await?;
    let user_id: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO users (kind, username, display_name) VALUES ('local', $1, $1)
         ON CONFLICT (lower(username)) WHERE kind = 'local' DO NOTHING
         RETURNING id",
    )
    .bind(username)
    .fetch_optional(&mut *tx)
    .await?;
    let user_id = user_id.ok_or_else(|| BreakGlassError::Exists(username.to_owned()))?;
    sqlx::query("INSERT INTO local_accounts (user_id, password_hash) VALUES ($1, $2)")
        .bind(user_id)
        .bind(&hash)
        .execute(&mut *tx)
        .await?;
    secrets::store(&mut *tx, vault, user_id, 1, TOTP_FIELD, secret.as_slice()).await?;
    tx.commit().await?;
    Ok(issue(username, password, secret.as_slice()))
}

/// New password and new TOTP secret; open sessions of the account end.
pub async fn reset(
    db: &PgPool,
    vault: &DynVault,
    username: &str,
) -> Result<Issued, BreakGlassError> {
    let account = find(db, username)
        .await?
        .ok_or_else(|| BreakGlassError::Unknown(username.to_owned()))?;
    let password = random_password();
    let secret = random_totp_secret();
    let hash = hash_password(&password)?;

    let mut tx = db.begin().await?;
    let version: i32 = sqlx::query_scalar(
        "UPDATE local_accounts
         SET password_hash = $2, totp_version = totp_version + 1, last_totp_step = 0
         WHERE user_id = $1 RETURNING totp_version",
    )
    .bind(account.user_id)
    .bind(&hash)
    .fetch_one(&mut *tx)
    .await?;
    secrets::store(
        &mut *tx,
        vault,
        account.user_id,
        version,
        TOTP_FIELD,
        secret.as_slice(),
    )
    .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(account.user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(issue(&account.username, password, secret.as_slice()))
}

/// Removes the account, its sessions and its sealed TOTP secrets.
pub async fn delete(db: &PgPool, username: &str) -> Result<Account, BreakGlassError> {
    let account = find(db, username)
        .await?
        .ok_or_else(|| BreakGlassError::Unknown(username.to_owned()))?;
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM secret_fields WHERE owner_id = $1 AND field = $2")
        .bind(account.user_id)
        .bind(TOTP_FIELD)
        .execute(&mut *tx)
        .await?;
    // Sessions and local_accounts go with the user (ON DELETE CASCADE); the
    // user row stays referenced by the audit log, so it is only emptied.
    sqlx::query("DELETE FROM local_accounts WHERE user_id = $1")
        .bind(account.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(account.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE users SET kind = 'deleted' WHERE id = $1")
        .bind(account.user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(account)
}

pub async fn list(db: &PgPool) -> Result<Vec<Account>, sqlx::Error> {
    sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT u.id, u.username, u.display_name FROM users u
         JOIN local_accounts l ON l.user_id = u.id ORDER BY lower(u.username)",
    )
    .fetch_all(db)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(user_id, username, display_name)| Account {
                user_id,
                username,
                display_name,
            })
            .collect()
    })
}

async fn find(db: &PgPool, username: &str) -> Result<Option<Account>, sqlx::Error> {
    let row: Option<(Uuid, String, String)> = sqlx::query_as(
        "SELECT u.id, u.username, u.display_name FROM users u
         JOIN local_accounts l ON l.user_id = u.id
         WHERE u.kind = 'local' AND lower(u.username) = lower($1)",
    )
    .bind(username.trim())
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(user_id, username, display_name)| Account {
        user_id,
        username,
        display_name,
    }))
}

/// Checks password and TOTP code; `None` for any mismatch (unknown account,
/// wrong password, wrong or reused code) — the caller learns nothing more.
pub async fn authenticate(
    db: &PgPool,
    vault: &DynVault,
    username: &str,
    password: &SecretString,
    code: &str,
) -> Result<Option<Account>, BreakGlassError> {
    authenticate_at(db, vault, username, password, code, unix_now()).await
}

pub async fn authenticate_at(
    db: &PgPool,
    vault: &DynVault,
    username: &str,
    password: &SecretString,
    code: &str,
    unix_seconds: u64,
) -> Result<Option<Account>, BreakGlassError> {
    let row: Option<(Uuid, String, String, String, i32)> = sqlx::query_as(
        "SELECT u.id, u.username, u.display_name, l.password_hash, l.totp_version
         FROM users u JOIN local_accounts l ON l.user_id = u.id
         WHERE u.kind = 'local' AND lower(u.username) = lower($1)",
    )
    .bind(username.trim())
    .fetch_optional(db)
    .await?;
    let Some((user_id, username, display_name, hash, totp_version)) = row else {
        // Same work as for a real account, so timing does not reveal names.
        let _ = password_matches(password.expose_secret(), &DUMMY_HASH);
        return Ok(None);
    };
    if !password_matches(password.expose_secret(), &hash) {
        return Ok(None);
    }
    let secret = secrets::load(db, vault, user_id, totp_version, TOTP_FIELD)
        .await?
        .ok_or(SecretError::Vault(remotehub_vault::VaultError::Open))?;
    let Some(step) = totp_step(&secret, code.trim(), unix_seconds) else {
        return Ok(None);
    };
    // Each code works once: the step must be newer than the last one used.
    let accepted = sqlx::query(
        "UPDATE local_accounts SET last_totp_step = $2 WHERE user_id = $1 AND last_totp_step < $2",
    )
    .bind(user_id)
    .bind(i64::try_from(step).unwrap_or(i64::MAX))
    .execute(db)
    .await?
    .rows_affected()
        == 1;
    Ok(accepted.then_some(Account {
        user_id,
        username,
        display_name,
    }))
}

/// An argon2id hash of a random password nobody knows.
static DUMMY_HASH: LazyLock<String> =
    LazyLock::new(|| hash_password(&random_password()).expect("argon2 hashes a random password"));

/// The TOTP code for a base32 secret at a point in time, as an
/// authenticator app would show it.
pub fn code_at(secret_base32: &str, unix_seconds: u64) -> Option<String> {
    let secret = Zeroizing::new(BASE32_NOPAD.decode(secret_base32.as_bytes()).ok()?);
    Some(totp_code(&secret, unix_seconds / TOTP_PERIOD))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238, appendix B (SHA-1, 8 digits there; the last 6 digits here).
    #[test]
    fn totp_matches_the_rfc_test_vectors() {
        let secret = b"12345678901234567890";
        for (time, expected) in [
            (59u64, "287082"),
            (1_111_111_109, "081804"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
        ] {
            assert_eq!(totp_code(secret, time / TOTP_PERIOD), expected, "{time}");
        }
    }

    #[test]
    fn codes_are_accepted_one_step_around_now() {
        let secret = b"12345678901234567890";
        let now = 1_234_567_890;
        let current = totp_code(secret, now / 30);
        assert_eq!(totp_step(secret, &current, now), Some(now / 30));
        let previous = totp_code(secret, now / 30 - 1);
        assert_eq!(totp_step(secret, &previous, now), Some(now / 30 - 1));
        let old = totp_code(secret, now / 30 - 2);
        assert_eq!(totp_step(secret, &old, now), None);
        assert_eq!(totp_step(secret, "12345", now), None);
    }

    #[test]
    fn passwords_are_long_and_hashed_with_argon2id() {
        let password = random_password();
        assert_eq!(password.len(), PASSWORD_LENGTH);
        assert_ne!(password.as_str(), random_password().as_str());
        let hash = hash_password(&password).unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(password_matches(&password, &hash));
        assert!(!password_matches("wrong", &hash));
        assert!(!password_matches(&password, &DUMMY_HASH));
    }

    #[test]
    fn names_are_restricted() {
        assert!(valid_name("emergency-1"));
        assert!(valid_name("bg.admin_2"));
        for bad in ["", "with space", "ümlaut", "a\\b", &"x".repeat(65)] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_uri_carries_secret_and_issuer() {
        let issued = issue(
            "emergency",
            Zeroizing::new("pw".into()),
            b"12345678901234567890",
        );
        assert_eq!(
            issued.totp_secret.as_str(),
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"
        );
        assert!(
            issued
                .totp_uri
                .starts_with("otpauth://totp/remotehub:emergency?secret=GEZDGNBV")
        );
        assert!(issued.totp_uri.contains("issuer=remotehub"));
    }
}
