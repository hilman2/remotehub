//! Storage of sealed secret fields (table `secret_fields`, ADR 0004).
//!
//! Plaintext only exists in memory, in `Zeroizing` buffers, between opening
//! a field and using it; it is never logged or written anywhere.

use remotehub_vault::{Context, DynVault, Sealed, VaultError};
use sqlx::PgExecutor;
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Vault(#[from] VaultError),
}

#[derive(sqlx::FromRow)]
struct SealedRow {
    scheme: String,
    kek_id: String,
    kek_version: i32,
    wrapped_key: Vec<u8>,
    ciphertext: Vec<u8>,
}

/// Seals `plaintext` and stores it as `field` of `owner` in `version`.
pub async fn store<'e>(
    db: impl PgExecutor<'e>,
    vault: &DynVault,
    owner: Uuid,
    version: i32,
    field: &str,
    plaintext: &[u8],
) -> Result<(), SecretError> {
    let sealed = vault.seal(
        Context {
            owner,
            version,
            field,
        },
        plaintext,
    )?;
    sqlx::query(
        "INSERT INTO secret_fields
             (owner_id, version, field, scheme, kek_id, kek_version, wrapped_key, ciphertext)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(owner)
    .bind(version)
    .bind(field)
    .bind(&sealed.scheme)
    .bind(&sealed.kek_id)
    .bind(sealed.kek_version)
    .bind(&sealed.wrapped_key)
    .bind(&sealed.ciphertext)
    .execute(db)
    .await?;
    Ok(())
}

/// Opens `field` of `owner` in `version`; `None` if it was never stored.
pub async fn load<'e>(
    db: impl PgExecutor<'e>,
    vault: &DynVault,
    owner: Uuid,
    version: i32,
    field: &str,
) -> Result<Option<Zeroizing<Vec<u8>>>, SecretError> {
    let row: Option<SealedRow> = sqlx::query_as(
        "SELECT scheme, kek_id, kek_version, wrapped_key, ciphertext
         FROM secret_fields WHERE owner_id = $1 AND version = $2 AND field = $3",
    )
    .bind(owner)
    .bind(version)
    .bind(field)
    .fetch_optional(db)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let sealed = Sealed {
        scheme: row.scheme,
        kek_id: row.kek_id,
        kek_version: row.kek_version,
        wrapped_key: row.wrapped_key,
        ciphertext: row.ciphertext,
    };
    Ok(Some(vault.open(
        Context {
            owner,
            version,
            field,
        },
        &sealed,
    )?))
}
