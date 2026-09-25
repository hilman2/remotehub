//! The personal vault (#22): entries that only their owner can read. The
//! browser encrypts them with a vault key (scheme `e2e_user_v1`, ADR 0004)
//! and keeps that key wrapped once per way to unlock it: a passkey (WebAuthn
//! PRF), a passphrase, the recovery key. This server stores ciphertext and
//! wrapped keys only, for their owner only, and can decrypt neither.
//!
//! - `GET /api/personal/vault`: unlocks and entries
//! - `POST /api/personal/unlocks`, `DELETE /api/personal/unlocks/{id}`
//! - `PUT /api/personal/entries/{id}`, `DELETE /api/personal/entries/{id}`
//! - `PUT /api/personal/search`: what the owner picked after searching,
//!   sealed like an entry
//! - `DELETE /api/personal/vault`: start over, everything is gone

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::catalog::{body, invalid};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::session::Session;

const SCHEME: &str = "e2e_user_v1";

/// Bytes as base64 in JSON.
mod bytes {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Unlock {
    id: Uuid,
    /// `passkey`, `passphrase` or `recovery`.
    kind: String,
    params: Value,
    #[serde(with = "bytes")]
    wrapped_key: Vec<u8>,
    label: String,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct StoredEntry {
    id: Uuid,
    #[serde(with = "bytes")]
    nonce: Vec<u8>,
    #[serde(with = "bytes")]
    ciphertext: Vec<u8>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct StoredSearch {
    #[serde(with = "bytes")]
    nonce: Vec<u8>,
    #[serde(with = "bytes")]
    ciphertext: Vec<u8>,
}

#[derive(Serialize)]
pub struct Vault {
    scheme: &'static str,
    unlocks: Vec<Unlock>,
    entries: Vec<StoredEntry>,
    /// What the owner picked after searching, sealed; none before the first.
    search: Option<StoredSearch>,
}

fn entry<'a>(session: &'a Session, action: Action, details: Value, address: &'a str) -> Entry<'a> {
    Entry {
        actor: Actor {
            id: Some(session.user_id),
            name: &session.username,
        },
        action,
        object: Some(("user", session.user_id)),
        details,
        address: Some(address),
    }
}

pub async fn vault(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vault>, Problem> {
    let unlocks = sqlx::query_as(
        "SELECT id, kind, params, wrapped_key, label FROM personal_vault_unlocks
         WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(session.user_id)
    .fetch_all(&state.db)
    .await?;
    let entries = sqlx::query_as(
        "SELECT id, nonce, ciphertext FROM personal_entries WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(session.user_id)
    .fetch_all(&state.db)
    .await?;
    let search = sqlx::query_as("SELECT nonce, ciphertext FROM personal_search WHERE user_id = $1")
        .bind(session.user_id)
        .fetch_optional(&state.db)
        .await?;
    Ok(Json(Vault {
        scheme: SCHEME,
        unlocks,
        entries,
        search,
    }))
}

#[derive(Deserialize)]
pub struct NewUnlock {
    kind: String,
    params: Value,
    #[serde(with = "bytes")]
    wrapped_key: Vec<u8>,
    #[serde(default)]
    label: String,
}

/// Adds a way to unlock the vault. A new passphrase or recovery key replaces
/// the old one.
pub async fn add_unlock(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<NewUnlock>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), Problem> {
    let input = body(input)?;
    if !matches!(input.kind.as_str(), "passkey" | "passphrase" | "recovery") {
        return Err(invalid("kind"));
    }
    if !input.params.is_object() || input.params.to_string().len() > 2048 {
        return Err(invalid("params"));
    }
    if !(16..=256).contains(&input.wrapped_key.len()) {
        return Err(invalid("wrapped_key"));
    }
    let label = input.label.trim();
    if label.chars().count() > 100 {
        return Err(invalid("label"));
    }
    let mut tx = state.db.begin().await?;
    if input.kind != "passkey" {
        sqlx::query("DELETE FROM personal_vault_unlocks WHERE user_id = $1 AND kind = $2")
            .bind(session.user_id)
            .bind(&input.kind)
            .execute(&mut *tx)
            .await?;
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO personal_vault_unlocks (user_id, kind, params, wrapped_key, label)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(session.user_id)
    .bind(&input.kind)
    .bind(&input.params)
    .bind(&input.wrapped_key)
    .bind(label)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::PersonalUnlockAdded,
            json!({ "unlock_id": id, "kind": input.kind }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// Removes a way to unlock the vault, but never the last one: that would
/// lock the owner out of their own entries for good.
pub async fn remove_unlock(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    let mut tx = state.db.begin().await?;
    let kinds: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, kind FROM personal_vault_unlocks WHERE user_id = $1 FOR UPDATE")
            .bind(session.user_id)
            .fetch_all(&mut *tx)
            .await?;
    let kind = kinds
        .iter()
        .find(|(unlock, _)| *unlock == id)
        .map(|(_, kind)| kind.clone())
        .ok_or(Problem::new(ErrorCode::NotFound))?;
    if kinds.len() == 1 {
        return Err(Problem::new(ErrorCode::LastUnlock));
    }
    sqlx::query("DELETE FROM personal_vault_unlocks WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::PersonalUnlockRemoved,
            json!({ "unlock_id": id, "kind": kind }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct EntryInput {
    #[serde(with = "bytes")]
    nonce: Vec<u8>,
    #[serde(with = "bytes")]
    ciphertext: Vec<u8>,
}

/// Stores an entry under the ID the browser chose (it is part of the
/// entry's associated data). An ID that belongs to someone else is not
/// found.
pub async fn save_entry(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    input: Result<Json<EntryInput>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let input = body(input)?;
    if input.nonce.len() != 12 {
        return Err(invalid("nonce"));
    }
    if !(16..=65536).contains(&input.ciphertext.len()) {
        return Err(invalid("ciphertext"));
    }
    let mut tx = state.db.begin().await?;
    let has_unlock: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM personal_vault_unlocks WHERE user_id = $1)",
    )
    .bind(session.user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !has_unlock {
        // Without a way to unlock, nobody could ever read the entry.
        return Err(invalid("vault"));
    }
    let saved = sqlx::query(
        "INSERT INTO personal_entries (id, user_id, scheme, nonce, ciphertext)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (id) DO UPDATE
             SET nonce = EXCLUDED.nonce, ciphertext = EXCLUDED.ciphertext, updated_at = now()
             WHERE personal_entries.user_id = EXCLUDED.user_id",
    )
    .bind(id)
    .bind(session.user_id)
    .bind(SCHEME)
    .bind(&input.nonce)
    .bind(&input.ciphertext)
    .execute(&mut *tx)
    .await?;
    if saved.rows_affected() == 0 {
        return Err(Problem::new(ErrorCode::NotFound));
    }
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::PersonalEntrySaved,
            json!({ "entry_id": id }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_entry(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    let mut tx = state.db.begin().await?;
    let deleted = sqlx::query("DELETE FROM personal_entries WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(session.user_id)
        .execute(&mut *tx)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(Problem::new(ErrorCode::NotFound));
    }
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::PersonalEntryDeleted,
            json!({ "entry_id": id }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Stores what the owner picked after searching the vault (#81), sealed in
/// the browser like an entry (associated data `search`). A preference, not
/// a secret: saved on every pick and not audited.
pub async fn save_search(
    State(state): State<AppState>,
    session: Session,
    input: Result<Json<EntryInput>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let input = body(input)?;
    if input.nonce.len() != 12 {
        return Err(invalid("nonce"));
    }
    if !(16..=65536).contains(&input.ciphertext.len()) {
        return Err(invalid("ciphertext"));
    }
    sqlx::query(
        "INSERT INTO personal_search (user_id, nonce, ciphertext) VALUES ($1, $2, $3)
         ON CONFLICT (user_id) DO UPDATE
             SET nonce = EXCLUDED.nonce, ciphertext = EXCLUDED.ciphertext, updated_at = now()",
    )
    .bind(session.user_id)
    .bind(&input.nonce)
    .bind(&input.ciphertext)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Starts over: every entry and every way to unlock is gone. For an owner
/// who has lost all of them; nobody can read the entries anyway.
pub async fn reset(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
) -> Result<StatusCode, Problem> {
    let mut tx = state.db.begin().await?;
    let entries = sqlx::query("DELETE FROM personal_entries WHERE user_id = $1")
        .bind(session.user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    sqlx::query("DELETE FROM personal_search WHERE user_id = $1")
        .bind(session.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM personal_vault_unlocks WHERE user_id = $1")
        .bind(session.user_id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::PersonalVaultReset,
            json!({ "entries": entries }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    use super::*;

    #[test]
    fn bytes_travel_as_base64() {
        let entry: EntryInput = serde_json::from_value(
            json!({ "nonce": STANDARD.encode([1u8; 12]), "ciphertext": "AAEC" }),
        )
        .unwrap();
        assert_eq!(entry.nonce, [1u8; 12]);
        assert_eq!(entry.ciphertext, [0, 1, 2]);
        assert!(
            serde_json::from_value::<EntryInput>(json!({ "nonce": "!", "ciphertext": "" }))
                .is_err()
        );
    }
}
