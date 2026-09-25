//! The organisation recovery key and the recovery of personal vaults (#95,
//! ADR 0009). The private key never reaches this server: it keeps public
//! keys, and hands the wrapped vault key of an approved recovery to the
//! administrator who asked for it, whose browser does the rest.
//!
//! - `GET /api/recovery-keys`: the keys and which vaults they cover
//!   (administrators)
//! - `POST /api/recovery-keys`: a new public key; `DELETE
//!   /api/recovery-keys/{id}`: an old key no vault depends on
//! - `GET /api/vault-recoveries`: every recovery (administrators and
//!   security officers)
//! - `POST /api/vault-recoveries`: ask for one (administrators)
//! - `POST /api/vault-recoveries/{id}/approve`: a security officer who did
//!   not ask
//! - `DELETE /api/vault-recoveries/{id}`: take back one not completed
//! - `GET /api/vault-recoveries/{id}/vault`, `GET …/attachments/{file}`: the
//!   sealed vault, for the requester, for a day after the approval
//! - `POST /api/vault-recoveries/{id}/complete`: done; for a forgotten
//!   passphrase with the one-time recovery key for the owner

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::catalog::{body, invalid};
use super::personal::{StoredAttachment, StoredEntry, bytes};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::session::{Role, Session};

fn entry<'a>(
    session: &'a Session,
    action: Action,
    user: Option<Uuid>,
    details: Value,
    address: &'a str,
) -> Entry<'a> {
    Entry {
        actor: Actor {
            id: Some(session.user_id),
            name: &session.username,
        },
        action,
        object: user.map(|id| ("user", id)),
        details,
        address: Some(address),
    }
}

fn require_admin(state: &AppState, session: &Session) -> Result<(), Problem> {
    if session.is_admin(&state.settings) {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::Forbidden))
    }
}

#[derive(Serialize, sqlx::FromRow)]
pub struct RecoveryKey {
    id: Uuid,
    #[serde(with = "bytes")]
    public_key: Vec<u8>,
    created_by_name: String,
    /// RFC 3339, UTC.
    created_at: String,
    /// How many vaults are wrapped for it.
    vaults: i64,
    /// Whether it holds the master key in use (#96).
    master_key: bool,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct CoveredVault {
    user_id: Uuid,
    display_name: String,
    username: String,
    /// The recovery key the vault is wrapped for; none if it is not.
    key_id: Option<Uuid>,
}

#[derive(Serialize)]
pub struct Keys {
    /// Newest first; the first is the one vaults are wrapped for.
    keys: Vec<RecoveryKey>,
    /// Every user with a personal vault.
    vaults: Vec<CoveredVault>,
}

pub async fn keys(State(state): State<AppState>, session: Session) -> Result<Json<Keys>, Problem> {
    require_admin(&state, &session)?;
    let (kek_id, kek_version) = state.vault.keys().current();
    let keys = sqlx::query_as(
        r#"SELECT k.id, k.public_key, k.created_by_name,
                  to_char(k.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') AS created_at,
                  (SELECT count(*) FROM personal_vault_unlocks u
                   WHERE u.kind = 'organisation' AND u.params->>'key_id' = k.id::text) AS vaults,
                  EXISTS (SELECT 1 FROM master_key_escrow e WHERE e.recovery_key_id = k.id
                          AND e.kek_id = $1 AND e.kek_version = $2) AS master_key
           FROM recovery_keys k ORDER BY k.created_at DESC, k.id"#,
    )
    .bind(kek_id)
    .bind(kek_version)
    .fetch_all(&state.db)
    .await?;
    let vaults = sqlx::query_as(
        "SELECT u.id AS user_id, u.display_name, u.username,
                (SELECT (o.params->>'key_id')::uuid FROM personal_vault_unlocks o
                 WHERE o.user_id = u.id AND o.kind = 'organisation') AS key_id
         FROM users u
         WHERE EXISTS (SELECT 1 FROM personal_vault_unlocks p WHERE p.user_id = u.id)
         ORDER BY lower(u.display_name), u.id",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(Keys { keys, vaults }))
}

#[derive(Deserialize)]
pub struct NewKey {
    #[serde(with = "bytes")]
    public_key: Vec<u8>,
}

/// Stores the public half of a key pair made in the browser. From now on,
/// vaults are wrapped for it at their next unlock.
pub async fn create_key(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<NewKey>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), Problem> {
    require_admin(&state, &session)?;
    let input = body(input)?;
    if !remotehub_vault::escrow::is_public_key(&input.public_key) {
        return Err(invalid("public_key"));
    }
    let mut tx = state.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO recovery_keys (public_key, created_by_name) VALUES ($1, $2) RETURNING id",
    )
    .bind(&input.public_key)
    .bind(&session.username)
    .fetch_one(&mut *tx)
    .await?;
    let details = json!({ "key_id": id });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::RecoveryKeyCreated,
            None,
            details,
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    // The key exists either way; a master key not kept now is kept at the
    // next start, and the page shows which keys hold it.
    if let Err(error) = crate::escrow::keep(&state.db, &state.vault).await {
        tracing::error!(%error, "cannot keep the master key for the new recovery key");
    }
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// Deletes an old key, once no vault is wrapped for it any more. The newest
/// key stays: vaults are wrapped for it.
pub async fn delete_key(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    require_admin(&state, &session)?;
    let mut tx = state.db.begin().await?;
    // Locked, so no key is created meanwhile and no vault is wrapped for
    // this one (`personal::add_unlock` takes a share lock).
    sqlx::query("LOCK TABLE recovery_keys IN EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    let newest = super::personal::organisation_key(&mut *tx)
        .await?
        .ok_or(Problem::new(ErrorCode::NotFound))?;
    let used: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM personal_vault_unlocks
                        WHERE kind = 'organisation' AND params->>'key_id' = $1::text)",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if used || newest.id == id {
        return Err(Problem::new(ErrorCode::RecoveryKeyInUse));
    }
    let deleted = sqlx::query("DELETE FROM recovery_keys WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(Problem::new(ErrorCode::NotFound));
    }
    let details = json!({ "key_id": id });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::RecoveryKeyDeleted,
            None,
            details,
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A recovery's state as the UI reads it; `approved` holds for a day.
macro_rules! select {
    ($filter:literal) => {
        concat!(
            "SELECT r.id, r.user_id, u.display_name AS user_name, r.kind, r.reason,
                    r.requester_id, r.requester_name, r.approver_name,
                    CASE WHEN r.completed_at IS NOT NULL THEN 'completed'
                         WHEN r.approved_at IS NULL THEN 'pending'
                         WHEN r.approved_at > now() - interval '1 day' THEN 'approved'
                         ELSE 'expired' END AS status,
                    to_char(r.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS created_at
             FROM vault_recoveries r JOIN users u ON u.id = r.user_id ",
            $filter
        )
    };
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Recovery {
    id: Uuid,
    user_id: Uuid,
    user_name: String,
    /// `passphrase` or `handover`.
    kind: String,
    reason: String,
    #[serde(skip)]
    requester_id: Uuid,
    requester_name: String,
    approver_name: Option<String>,
    /// `pending`, `approved`, `expired` or `completed`.
    status: String,
    created_at: String,
    /// Whether the caller asked for it, and so may carry it out.
    #[sqlx(skip)]
    mine: bool,
}

async fn recovery(state: &AppState, id: Uuid) -> Result<Recovery, Problem> {
    sqlx::query_as(select!("WHERE r.id = $1"))
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(Problem::new(ErrorCode::NotFound))
}

pub async fn list(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<Recovery>>, Problem> {
    if !session.is_admin(&state.settings) && !session.has_role(Role::SecurityOfficer) {
        return Err(Problem::new(ErrorCode::Forbidden));
    }
    let mut recoveries: Vec<Recovery> =
        sqlx::query_as(select!("ORDER BY r.created_at DESC LIMIT 200"))
            .fetch_all(&state.db)
            .await?;
    for recovery in &mut recoveries {
        recovery.mine = recovery.requester_id == session.user_id;
    }
    Ok(Json(recoveries))
}

#[derive(Deserialize)]
pub struct NewRecovery {
    user_id: Uuid,
    kind: String,
    reason: String,
}

pub async fn create(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<NewRecovery>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), Problem> {
    require_admin(&state, &session)?;
    let input = body(input)?;
    if !matches!(input.kind.as_str(), "passphrase" | "handover") {
        return Err(invalid("kind"));
    }
    let reason = input.reason.trim();
    if reason.is_empty() || reason.chars().count() > 500 {
        return Err(invalid("reason"));
    }
    let mut tx = state.db.begin().await?;
    let covered: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM personal_vault_unlocks
                        WHERE user_id = $1 AND kind = 'organisation')",
    )
    .bind(input.user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !covered {
        return Err(Problem::new(ErrorCode::VaultNotCovered));
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO vault_recoveries (user_id, kind, reason, requester_id, requester_name)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(input.user_id)
    .bind(&input.kind)
    .bind(reason)
    .bind(session.user_id)
    .bind(&session.username)
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| match error.as_database_error() {
        Some(db) if db.is_unique_violation() => Problem::new(ErrorCode::RecoveryOpen),
        _ => error.into(),
    })?;
    let details = json!({ "recovery_id": id, "kind": input.kind, "reason": reason });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::VaultRecoveryRequested,
            Some(input.user_id),
            details,
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// The second person: a security officer who did not ask.
pub async fn approve(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    if !session.has_role(Role::SecurityOfficer) {
        return Err(Problem::new(ErrorCode::Forbidden));
    }
    let found = recovery(&state, id).await?;
    if found.requester_id == session.user_id {
        return Err(Problem::new(ErrorCode::OwnRequest));
    }
    let mut tx = state.db.begin().await?;
    let approved = sqlx::query(
        "UPDATE vault_recoveries SET approver_name = $2, approved_at = now()
         WHERE id = $1 AND approved_at IS NULL AND completed_at IS NULL",
    )
    .bind(id)
    .bind(&session.username)
    .execute(&mut *tx)
    .await?;
    if approved.rows_affected() == 0 {
        return Err(Problem::new(ErrorCode::RequestDecided));
    }
    let details = json!({ "recovery_id": id });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::VaultRecoveryApproved,
            Some(found.user_id),
            details,
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Takes back a recovery not carried out: the requester or another
/// administrator, e.g. for one whose day has passed.
pub async fn cancel(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    require_admin(&state, &session)?;
    let mut tx = state.db.begin().await?;
    let user: Uuid = sqlx::query_scalar(
        "DELETE FROM vault_recoveries WHERE id = $1 AND completed_at IS NULL RETURNING user_id",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Problem::new(ErrorCode::NotFound))?;
    let details = json!({ "recovery_id": id });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::VaultRecoveryCancelled,
            Some(user),
            details,
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A recovery the caller may carry out now: theirs, approved, within its day.
async fn ready(state: &AppState, session: &Session, id: Uuid) -> Result<Recovery, Problem> {
    require_admin(state, session)?;
    let found = recovery(state, id).await?;
    if found.requester_id != session.user_id {
        return Err(Problem::new(ErrorCode::Forbidden));
    }
    if found.status != "approved" {
        return Err(Problem::new(ErrorCode::RecoveryNotReady));
    }
    Ok(found)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct OrganisationUnlock {
    params: Value,
    #[serde(with = "bytes")]
    wrapped_key: Vec<u8>,
    /// The public key the vault key is wrapped for.
    #[serde(with = "bytes")]
    public_key: Vec<u8>,
}

#[derive(Serialize)]
pub struct SealedVault {
    unlock: OrganisationUnlock,
    entries: Vec<StoredEntry>,
}

/// What the requester's browser needs to open the vault with the private
/// key: the vault key wrapped for the organisation, and the sealed entries.
pub async fn vault(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, Problem> {
    let found = ready(&state, &session, id).await?;
    let unlock = sqlx::query_as(
        "SELECT u.params, u.wrapped_key, k.public_key FROM personal_vault_unlocks u
         JOIN recovery_keys k ON k.id::text = u.params->>'key_id'
         WHERE u.user_id = $1 AND u.kind = 'organisation'",
    )
    .bind(found.user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(Problem::new(ErrorCode::VaultNotCovered))?;
    let entries = sqlx::query_as(
        "SELECT id, nonce, ciphertext FROM personal_entries WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(found.user_id)
    .fetch_all(&state.db)
    .await?;
    let details = json!({ "recovery_id": id });
    audit::record(
        &state.db,
        entry(
            &session,
            Action::VaultRecoveryOpened,
            Some(found.user_id),
            details,
            &address,
        ),
    )
    .await?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(SealedVault { unlock, entries }),
    ))
}

/// A sealed file of the vault being recovered.
pub async fn attachment(
    State(state): State<AppState>,
    session: Session,
    Path((id, file)): Path<(Uuid, Uuid)>,
) -> Result<Json<StoredAttachment>, Problem> {
    let found = ready(&state, &session, id).await?;
    sqlx::query_as(
        "SELECT nonce, ciphertext FROM personal_attachments WHERE id = $1 AND user_id = $2",
    )
    .bind(file)
    .bind(found.user_id)
    .fetch_optional(&state.db)
    .await?
    .map(Json)
    .ok_or(Problem::new(ErrorCode::NotFound))
}

#[derive(Deserialize)]
pub struct Completion {
    /// For a forgotten passphrase: the vault key wrapped with the one-time
    /// recovery key the owner is given.
    #[serde(default, with = "optional_bytes")]
    wrapped_key: Option<Vec<u8>>,
}

mod optional_bytes {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Vec<u8>>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|text| STANDARD.decode(text).map_err(serde::de::Error::custom))
            .transpose()
    }
}

/// Ends a recovery. For a forgotten passphrase, the one-time recovery key's
/// wrap replaces the owner's recovery key; the owner's page then asks for a
/// new passphrase and recovery key.
pub async fn complete(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    input: Result<Json<Completion>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let input = body(input)?;
    let found = ready(&state, &session, id).await?;
    let mut tx = state.db.begin().await?;
    let done = sqlx::query(
        "UPDATE vault_recoveries SET completed_at = now() WHERE id = $1 AND completed_at IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(Problem::new(ErrorCode::RecoveryNotReady));
    }
    if found.kind == "passphrase" {
        let wrapped = input
            .wrapped_key
            .filter(|key| (16..=256).contains(&key.len()))
            .ok_or(invalid("wrapped_key"))?;
        sqlx::query("DELETE FROM personal_vault_unlocks WHERE user_id = $1 AND kind = 'recovery'")
            .bind(found.user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO personal_vault_unlocks (user_id, kind, params, wrapped_key)
             VALUES ($1, 'recovery', '{\"one_time\": true}', $2)",
        )
        .bind(found.user_id)
        .bind(wrapped)
        .execute(&mut *tx)
        .await?;
    }
    let details = json!({ "recovery_id": id, "kind": found.kind });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::VaultRecoveryCompleted,
            Some(found.user_id),
            details,
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
