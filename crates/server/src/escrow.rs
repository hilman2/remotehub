//! The master keys, kept in the database for the organisation recovery key
//! (#96, ADR 0009). A database backup and the recovery key's private key
//! then restore the master key file (`remotehub recover-master-key`); the
//! key file is no longer a second secret that must survive on its own.
//!
//! The cryptography is `remotehub_vault::escrow`; here is when it happens:
//! at startup, and whenever an administrator creates a recovery key.

use anyhow::Context;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use remotehub_vault::escrow::{self, Escrowed};
use remotehub_vault::{DynVault, KeyProvider};
use sqlx::PgPool;
use uuid::Uuid;
use zeroize::Zeroizing;

/// Seals every master key the server holds for the newest recovery key,
/// unless it is already. Returns how many it sealed now.
pub async fn keep(db: &PgPool, vault: &DynVault) -> anyhow::Result<usize> {
    let Some((key_id, public_key)): Option<(Uuid, Vec<u8>)> = sqlx::query_as(
        "SELECT id, public_key FROM recovery_keys ORDER BY created_at DESC, id LIMIT 1",
    )
    .fetch_optional(db)
    .await?
    else {
        return Ok(0);
    };
    let mut sealed = 0;
    for (kek_id, version) in vault.keys().all() {
        let master = vault.keys().master_key(kek_id, version)?;
        let escrowed = escrow::seal_for(&public_key, kek_id, version, master)
            .with_context(|| format!("cannot seal the master key for recovery key {key_id}"))?;
        let stored = sqlx::query(
            "INSERT INTO master_key_escrow (recovery_key_id, kek_id, kek_version, ephemeral, sealed)
             VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
        )
        .bind(key_id)
        .bind(kek_id)
        .bind(version)
        .bind(&escrowed.ephemeral)
        .bind(&escrowed.sealed)
        .execute(db)
        .await?;
        sealed += stored.rows_affected() as usize;
    }
    Ok(sealed)
}

/// The master key file as `FileKeyring` reads it, recovered with the
/// private key of a recovery key (its scalar). Fails if no recovery key in
/// the database belongs to it.
pub async fn recover(db: &PgPool, private_key: &[u8]) -> anyhow::Result<Zeroizing<String>> {
    let public_key = escrow::public_key_of(private_key).context("not a private key")?;
    let key_id: Uuid = sqlx::query_scalar("SELECT id FROM recovery_keys WHERE public_key = $1")
        .bind(&public_key)
        .fetch_optional(db)
        .await?
        .context("no recovery key in this database belongs to this private key")?;
    let rows: Vec<(String, i32, Vec<u8>, Vec<u8>)> = sqlx::query_as(
        "SELECT kek_id, kek_version, ephemeral, sealed FROM master_key_escrow
         WHERE recovery_key_id = $1 ORDER BY kek_id, kek_version",
    )
    .bind(key_id)
    .fetch_all(db)
    .await?;
    anyhow::ensure!(
        !rows.is_empty(),
        "the recovery key {key_id} holds no master key"
    );
    let mut file = Zeroizing::new(String::from("# remotehub master keys\n"));
    for (kek_id, version, ephemeral, sealed) in rows {
        // Only the key file exists as a provider; another one would need
        // its own way back.
        anyhow::ensure!(
            kek_id == remotehub_vault::FILE_KEYRING_ID,
            "master key {kek_id} is not from a key file"
        );
        let master = escrow::open_with(
            private_key,
            &Escrowed { ephemeral, sealed },
            &kek_id,
            version,
        )
        .with_context(|| format!("cannot open master key version {version}"))?;
        file.push_str(&format!(
            "{version}:{}\n",
            STANDARD.encode(master.as_slice())
        ));
    }
    Ok(file)
}
