//! The directory connection (#144): stored in the table `directory`, the
//! service account's password sealed like a vault entry, and held while
//! running in [`Current`], which a saved change replaces without a restart.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use remotehub_directory::ldap::{LdapConfig, LdapDirectory};
use remotehub_vault::DynVault;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sqlx::{PgExecutor, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::auth::Authenticator;
use crate::secrets::{self, SecretError};

/// The field of the sealed password in `secret_fields`.
const PASSWORD_FIELD: &str = "bind_password";
pub const DEFAULT_TIMEOUT_SECONDS: i32 = 10;

/// The directory in use; none without a connection.
#[derive(Default)]
pub struct Current(RwLock<Option<Arc<dyn Authenticator>>>);

impl Current {
    pub fn new(directory: Option<Arc<dyn Authenticator>>) -> Self {
        Current(RwLock::new(directory))
    }

    pub fn get(&self) -> Option<Arc<dyn Authenticator>> {
        self.0.read().expect("directory lock").clone()
    }

    pub fn set(&self, directory: Option<Arc<dyn Authenticator>>) {
        *self.0.write().expect("directory lock") = directory;
    }
}

/// Everything about the connection but the password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Connection {
    pub url: String,
    pub starttls: bool,
    pub ca_pem: Option<String>,
    pub bind_dn: String,
    pub base_dn: String,
    pub user_filter: Option<String>,
    pub timeout_seconds: i32,
}

impl Connection {
    pub fn config(&self, password: SecretString) -> LdapConfig {
        LdapConfig {
            url: self.url.clone(),
            starttls: self.starttls,
            ca_pem: self.ca_pem.clone(),
            bind_dn: self.bind_dn.clone(),
            bind_password: password,
            base_dn: self.base_dn.clone(),
            user_filter: self.user_filter.clone(),
            timeout: Duration::from_secs(self.timeout_seconds.unsigned_abs().into()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DirectoryError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error("the sealed password of the directory is missing")]
    NoPassword,
    #[error("invalid directory settings: {0}")]
    Invalid(String),
}

#[derive(sqlx::FromRow)]
struct Stored {
    id: Uuid,
    password_version: i32,
    #[sqlx(flatten)]
    connection: Connection,
}

macro_rules! select {
    () => {
        "SELECT id, password_version, url, starttls, ca_pem, bind_dn, base_dn, user_filter,
                timeout_seconds
         FROM directory"
    };
}
const SELECT: &str = select!();

/// The stored connection, without its password.
pub async fn connection<'e>(db: impl PgExecutor<'e>) -> Result<Option<Connection>, sqlx::Error> {
    Ok(sqlx::query_as::<_, Stored>(SELECT)
        .fetch_optional(db)
        .await?
        .map(|stored| stored.connection))
}

/// The directory as stored, ready to sign in against; none without one.
pub async fn open(
    db: &PgPool,
    vault: &DynVault,
) -> Result<Option<Arc<dyn Authenticator>>, DirectoryError> {
    let Some(stored) = sqlx::query_as::<_, Stored>(SELECT)
        .fetch_optional(db)
        .await?
    else {
        return Ok(None);
    };
    let password = sealed_password(db, vault, stored.id, stored.password_version).await?;
    let directory = LdapDirectory::new(stored.connection.config(password))
        .map_err(|e| DirectoryError::Invalid(e.to_string()))?;
    Ok(Some(Arc::new(directory)))
}

async fn sealed_password<'e>(
    db: impl PgExecutor<'e>,
    vault: &DynVault,
    id: Uuid,
    version: i32,
) -> Result<SecretString, DirectoryError> {
    let plain = secrets::load(db, vault, id, version, PASSWORD_FIELD)
        .await?
        .ok_or(DirectoryError::NoPassword)?;
    let text = String::from_utf8(plain.to_vec()).map_err(|_| DirectoryError::NoPassword)?;
    Ok(SecretString::from(text))
}

/// The password to use with a change: the new one, or the stored one.
pub async fn password_for(
    db: &PgPool,
    vault: &DynVault,
    new: Option<SecretString>,
) -> Result<Option<SecretString>, DirectoryError> {
    if new.is_some() {
        return Ok(new);
    }
    let Some(stored) = sqlx::query_as::<_, Stored>(SELECT)
        .fetch_optional(db)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(
        sealed_password(db, vault, stored.id, stored.password_version).await?,
    ))
}

/// Stores `connection`, with a new password if one is given, in place of
/// the one before. Returns the connection before, if there was one.
pub async fn save(
    tx: &mut Transaction<'_, Postgres>,
    vault: &DynVault,
    connection: &Connection,
    password: Option<&SecretString>,
    by: Uuid,
) -> Result<Option<Connection>, DirectoryError> {
    let before = sqlx::query_as::<_, Stored>(concat!(select!(), " FOR UPDATE"))
        .fetch_optional(&mut **tx)
        .await?;
    let (id, version) = match (&before, password) {
        (Some(stored), None) => (stored.id, stored.password_version),
        (Some(stored), Some(_)) => (stored.id, stored.password_version + 1),
        (None, Some(_)) => {
            let id: Uuid = sqlx::query_scalar("SELECT gen_random_uuid()")
                .fetch_one(&mut **tx)
                .await?;
            (id, 1)
        }
        (None, None) => return Err(DirectoryError::NoPassword),
    };
    if let Some(password) = password {
        secrets::store(
            &mut **tx,
            vault,
            id,
            version,
            PASSWORD_FIELD,
            password.expose_secret().as_bytes(),
        )
        .await?;
    }
    sqlx::query(
        "INSERT INTO directory (id, url, starttls, ca_pem, bind_dn, password_version, base_dn,
                                user_filter, timeout_seconds, updated_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (singleton) DO UPDATE SET
             url = EXCLUDED.url, starttls = EXCLUDED.starttls, ca_pem = EXCLUDED.ca_pem,
             bind_dn = EXCLUDED.bind_dn, password_version = EXCLUDED.password_version,
             base_dn = EXCLUDED.base_dn, user_filter = EXCLUDED.user_filter,
             timeout_seconds = EXCLUDED.timeout_seconds, updated_at = now(),
             updated_by = EXCLUDED.updated_by",
    )
    .bind(id)
    .bind(&connection.url)
    .bind(connection.starttls)
    .bind(&connection.ca_pem)
    .bind(&connection.bind_dn)
    .bind(version)
    .bind(&connection.base_dn)
    .bind(&connection.user_filter)
    .bind(connection.timeout_seconds)
    .bind(by)
    .execute(&mut **tx)
    .await?;
    // Older versions of the password are of no use any more.
    sqlx::query("DELETE FROM secret_fields WHERE owner_id = $1 AND field = $2 AND version < $3")
        .bind(id)
        .bind(PASSWORD_FIELD)
        .bind(version)
        .execute(&mut **tx)
        .await?;
    Ok(before.map(|stored| stored.connection))
}

/// Removes the connection and its password; whether there was one.
pub async fn remove(tx: &mut Transaction<'_, Postgres>) -> Result<bool, sqlx::Error> {
    let removed: Option<Uuid> = sqlx::query_scalar("DELETE FROM directory RETURNING id")
        .fetch_optional(&mut **tx)
        .await?;
    if let Some(id) = removed {
        sqlx::query("DELETE FROM secret_fields WHERE owner_id = $1")
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(removed.is_some())
}

/// Ends every session of a directory user: after the directory changed,
/// their groups may mean something else, or nothing.
pub async fn end_directory_sessions<'e>(db: impl PgExecutor<'e>) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "DELETE FROM sessions WHERE user_id IN (SELECT id FROM users WHERE kind = 'directory')",
    )
    .execute(db)
    .await?
    .rows_affected())
}
