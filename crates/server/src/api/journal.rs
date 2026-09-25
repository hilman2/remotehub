//! The journal of a device (#90) and who states a purpose before connecting.
//!
//! - `GET /api/devices/{id}/journal`: the newest entries, connections with
//!   their purpose and notes, for anyone who may connect to the device
//! - `POST /api/devices/{id}/journal`: leaves a note; the journal is only
//!   ever added to, so there is nothing to change or delete
//! - `GET /api/purpose-principals`, `PUT` and `DELETE
//!   /api/purpose-principals/{sid}`: the users and groups who state a purpose
//!   before every connection; administrators only, changes audited

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use remotehub_model::{ObjectId, Role};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::catalog::{body, context, invalid, name, require};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::principal::PrincipalId;
use crate::session::Session;

/// Entries one request returns, newest first.
const LIMIT: i64 = 200;
const NOTE_MAX: usize = 2000;

#[derive(Serialize, sqlx::FromRow)]
pub struct JournalEntry {
    id: Uuid,
    /// `connection` or `note`.
    kind: String,
    /// The purpose of a connection (may be empty), or the note.
    text: String,
    protocol: Option<String>,
    username: String,
    display_name: String,
    created_at: String,
    /// When the session ended; none while it runs or if the server stopped.
    ended_at: Option<String>,
}

pub async fn journal(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<JournalEntry>>, Problem> {
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Connect, ObjectId::Device(id))?;
    let entries = sqlx::query_as(
        "SELECT j.id, j.kind, j.text, j.protocol, u.username, u.display_name,
                to_char(j.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS created_at,
                to_char(j.ended_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS ended_at
         FROM device_journal j JOIN users u ON u.id = j.user_id
         WHERE j.device_id = $1
         ORDER BY j.created_at DESC, j.id
         LIMIT $2",
    )
    .bind(id)
    .bind(LIMIT)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(entries))
}

#[derive(Deserialize)]
pub struct NewNote {
    text: String,
}

pub async fn add_note(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    input: Result<Json<NewNote>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let input = body(input)?;
    let text = input.text.trim();
    if text.is_empty() || text.chars().count() > NOTE_MAX {
        return Err(invalid("text"));
    }
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Connect, ObjectId::Device(id))?;
    sqlx::query(
        "INSERT INTO device_journal (device_id, user_id, kind, text) VALUES ($1, $2, 'note', $3)",
    )
    .bind(id)
    .bind(session.user_id)
    .bind(text)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct PurposePrincipal {
    principal_sid: String,
    principal_kind: String,
    principal_name: String,
}

fn require_admin(session: &Session) -> Result<(), Problem> {
    if session.is_admin() {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::Forbidden))
    }
}

pub async fn purpose_principals(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<PurposePrincipal>>, Problem> {
    require_admin(&session)?;
    let principals = sqlx::query_as(
        "SELECT principal_sid, principal_kind, principal_name FROM purpose_principals
         ORDER BY lower(principal_name)",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(principals))
}

#[derive(Deserialize)]
pub struct NewPurposePrincipal {
    principal_kind: String,
    principal_name: String,
}

fn audit_entry<'a>(
    session: &'a Session,
    action: Action,
    details: serde_json::Value,
    address: &'a str,
) -> Entry<'a> {
    Entry {
        actor: Actor {
            id: Some(session.user_id),
            name: &session.username,
        },
        action,
        object: None,
        details,
        address: Some(address),
    }
}

/// Makes a user or group state a purpose before every connection; adding
/// one twice keeps the first.
pub async fn require_purpose(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(sid): Path<String>,
    input: Result<Json<NewPurposePrincipal>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    require_admin(&session)?;
    let input = body(input)?;
    let sid: PrincipalId = sid.parse().map_err(|_| invalid("principal_sid"))?;
    if !matches!(input.principal_kind.as_str(), "user" | "group") {
        return Err(invalid("principal_kind"));
    }
    if !sid.check(&state.db, &input.principal_kind).await? {
        return Err(invalid("principal_sid"));
    }
    let principal_name = name(&input.principal_name, "principal_name")?;
    let mut tx = state.db.begin().await?;
    let added = sqlx::query(
        "INSERT INTO purpose_principals (principal_sid, principal_kind, principal_name, created_by)
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(sid.to_string())
    .bind(&input.principal_kind)
    .bind(&principal_name)
    .bind(session.user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if added == 1 {
        let details = json!({
            "principal_kind": input.principal_kind, "principal_sid": sid.to_string(),
            "principal_name": principal_name,
        });
        audit::record(
            &mut *tx,
            audit_entry(&session, Action::PurposeRequired, details, &address),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn waive_purpose(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(sid): Path<String>,
) -> Result<StatusCode, Problem> {
    require_admin(&session)?;
    let mut tx = state.db.begin().await?;
    let removed: Option<PurposePrincipal> = sqlx::query_as(
        "DELETE FROM purpose_principals WHERE principal_sid = $1
         RETURNING principal_sid, principal_kind, principal_name",
    )
    .bind(&sid)
    .fetch_optional(&mut *tx)
    .await?;
    let removed = removed.ok_or(Problem::new(ErrorCode::NotFound))?;
    let details = json!({
        "principal_kind": removed.principal_kind, "principal_sid": removed.principal_sid,
        "principal_name": removed.principal_name,
    });
    audit::record(
        &mut *tx,
        audit_entry(&session, Action::PurposeWaived, details, &address),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
