//! Sealed secret fields in the database.

use remotehub_server::secrets::{self, SecretError};
use remotehub_vault::VaultError;
use sqlx::PgPool;
use uuid::Uuid;

const OWNER: Uuid = Uuid::from_u128(7);

#[sqlx::test(migrations = "../../migrations")]
async fn stores_only_ciphertext_and_opens_it_again(pool: PgPool) {
    let vault = crate::common::vault();
    secrets::store(&pool, &vault, OWNER, 1, "password", b"Sup3r-Secret!")
        .await
        .unwrap();

    let raw: Vec<u8> = sqlx::query_scalar("SELECT ciphertext FROM secret_fields")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!raw.windows(13).any(|w| w == b"Sup3r-Secret!"));

    let opened = secrets::load(&pool, &vault, OWNER, 1, "password")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(opened.as_slice(), b"Sup3r-Secret!");
    assert!(
        secrets::load(&pool, &vault, OWNER, 1, "notes")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        secrets::load(&pool, &vault, OWNER, 2, "password")
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_row_moved_to_another_owner_or_field_cannot_be_opened(pool: PgPool) {
    let vault = crate::common::vault();
    secrets::store(&pool, &vault, OWNER, 1, "password", b"secret")
        .await
        .unwrap();

    let thief = Uuid::from_u128(8);
    sqlx::query("UPDATE secret_fields SET owner_id = $1")
        .bind(thief)
        .execute(&pool)
        .await
        .unwrap();
    let error = secrets::load(&pool, &vault, thief, 1, "password")
        .await
        .unwrap_err();
    assert!(
        matches!(error, SecretError::Vault(VaultError::Open)),
        "{error:?}"
    );

    sqlx::query("UPDATE secret_fields SET owner_id = $1, field = 'notes'")
        .bind(OWNER)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        secrets::load(&pool, &vault, OWNER, 1, "notes")
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn another_master_key_cannot_open_the_database(pool: PgPool) {
    secrets::store(
        &pool,
        &crate::common::vault(),
        OWNER,
        1,
        "password",
        b"secret",
    )
    .await
    .unwrap();
    let error = secrets::load(&pool, &crate::common::vault(), OWNER, 1, "password")
        .await
        .unwrap_err();
    assert!(
        matches!(error, SecretError::Vault(VaultError::Open)),
        "{error:?}"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_field_is_stored_once_per_version(pool: PgPool) {
    let vault = crate::common::vault();
    secrets::store(&pool, &vault, OWNER, 1, "password", b"one")
        .await
        .unwrap();
    assert!(
        secrets::store(&pool, &vault, OWNER, 1, "password", b"two")
            .await
            .is_err()
    );
    secrets::store(&pool, &vault, OWNER, 2, "password", b"two")
        .await
        .unwrap();
    let v1 = secrets::load(&pool, &vault, OWNER, 1, "password")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(v1.as_slice(), b"one");
}
