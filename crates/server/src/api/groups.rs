//! `/api/groups`: groups of remotehub's own, for administrators (#105).
//!
//! - `GET /api/groups`: every group with its members.
//! - `POST /api/groups`, `PATCH`/`DELETE /api/groups/{id}`: create, rename,
//!   delete. Deleting a group also removes the grants and purpose rules that
//!   name it.
//! - `PUT`/`DELETE /api/groups/{id}/members/{sid}`: add or remove a member:
//!   a directory user or group, or a local account.
//!
//! A group is named elsewhere as `group:<id>`; the session lookup resolves
//! memberships at every request (`crate::session`). Every change is audited.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use super::catalog::{body, invalid, name};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::principal::PrincipalId;
use crate::session::Session;

#[derive(Serialize)]
pub struct Group {
    id: Uuid,
    name: String,
    description: String,
    members: Vec<Member>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Member {
    #[serde(skip)]
    group_id: Uuid,
    /// Named like the principal of a grant.
    sid: String,
    kind: String,
    name: String,
}

#[derive(Serialize)]
pub struct Created {
    id: Uuid,
}

#[derive(Deserialize)]
pub struct GroupInput {
    name: String,
    #[serde(default)]
    description: String,
}

#[derive(Deserialize)]
pub struct MemberInput {
    principal_kind: String,
    principal_name: String,
}

fn require_admin(state: &AppState, session: &Session) -> Result<(), Problem> {
    if session.is_admin(&state.settings) {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::Forbidden))
    }
}

/// A trimmed description of at most 500 characters, possibly empty.
fn description(value: &str) -> Result<String, Problem> {
    let value = value.trim();
    if value.chars().count() > 500 || value.chars().any(char::is_control) {
        return Err(invalid("description"));
    }
    Ok(value.to_owned())
}

/// A name another group has already, in any case, is taken.
fn taken(error: sqlx::Error) -> Problem {
    match &error {
        sqlx::Error::Database(e) if e.constraint() == Some("groups_name") => {
            Problem::new(ErrorCode::NameTaken)
        }
        _ => error.into(),
    }
}

async fn record(
    tx: &mut PgConnection,
    session: &Session,
    action: Action,
    group: Uuid,
    details: Value,
    address: &str,
) -> Result<(), Problem> {
    audit::record(
        tx,
        Entry {
            actor: Actor {
                id: Some(session.user_id),
                name: &session.username,
            },
            action,
            object: Some(("group", group)),
            details,
            address: Some(address),
        },
    )
    .await?;
    Ok(())
}

/// The group's name, if it exists.
async fn group_name(tx: &mut PgConnection, id: Uuid) -> Result<String, Problem> {
    sqlx::query_scalar("SELECT name FROM groups WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(tx)
        .await?
        .ok_or(Problem::new(ErrorCode::NotFound))
}

pub async fn list(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<Group>>, Problem> {
    require_admin(&state, &session)?;
    let groups: Vec<(Uuid, String, String)> =
        sqlx::query_as("SELECT id, name, description FROM groups ORDER BY lower(name)")
            .fetch_all(&state.db)
            .await?;
    let mut members: Vec<Member> = sqlx::query_as(
        "SELECT group_id, principal_sid AS sid, principal_kind AS kind, principal_name AS name
         FROM group_members ORDER BY principal_kind DESC, lower(principal_name)",
    )
    .fetch_all(&state.db)
    .await?;
    let groups = groups
        .into_iter()
        .map(|(id, name, description)| {
            let (mine, rest) = members.drain(..).partition(|m| m.group_id == id);
            members = rest;
            Group {
                id,
                name,
                description,
                members: mine,
            }
        })
        .collect();
    Ok(Json(groups))
}

pub async fn create(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<GroupInput>, JsonRejection>,
) -> Result<(StatusCode, Json<Created>), Problem> {
    require_admin(&state, &session)?;
    let input = body(input)?;
    let name = name(&input.name, "name")?;
    let description = description(&input.description)?;
    let mut tx = state.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO groups (name, description, created_by) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&name)
    .bind(&description)
    .bind(session.user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(taken)?;
    let details = json!({ "name": name });
    record(
        &mut tx,
        &session,
        Action::GroupCreated,
        id,
        details,
        &address,
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(Created { id })))
}

pub async fn update(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    input: Result<Json<GroupInput>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    require_admin(&state, &session)?;
    let input = body(input)?;
    let name = name(&input.name, "name")?;
    let description = description(&input.description)?;
    let mut tx = state.db.begin().await?;
    let before = group_name(&mut tx, id).await?;
    sqlx::query("UPDATE groups SET name = $2, description = $3 WHERE id = $1")
        .bind(id)
        .bind(&name)
        .bind(&description)
        .execute(&mut *tx)
        .await
        .map_err(taken)?;
    let details = json!({ "name": name, "before": before });
    record(
        &mut tx,
        &session,
        Action::GroupUpdated,
        id,
        details,
        &address,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    require_admin(&state, &session)?;
    let mut tx = state.db.begin().await?;
    let name = group_name(&mut tx, id).await?;
    // A later group could never get the same ID, but a grant to nobody only
    // confuses whoever reads the permissions.
    let principal = PrincipalId::Group(id).to_string();
    let grants = sqlx::query("DELETE FROM grants WHERE principal_sid = $1")
        .bind(&principal)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    sqlx::query("DELETE FROM purpose_principals WHERE principal_sid = $1")
        .bind(&principal)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM groups WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let details = json!({ "name": name, "grants_removed": grants });
    record(
        &mut tx,
        &session,
        Action::GroupDeleted,
        id,
        details,
        &address,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn add_member(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path((id, sid)): Path<(Uuid, String)>,
    input: Result<Json<MemberInput>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    require_admin(&state, &session)?;
    let input = body(input)?;
    let principal: PrincipalId = sid.parse().map_err(|_| invalid("principal_sid"))?;
    // Groups of remotehub's own do not nest: a member is a user or a
    // directory group.
    if matches!(principal, PrincipalId::Group(_))
        || !principal.check(&state.db, &input.principal_kind).await?
    {
        return Err(invalid("principal_sid"));
    }
    let member_name = name(&input.principal_name, "principal_name")?;
    let mut tx = state.db.begin().await?;
    let group = group_name(&mut tx, id).await?;
    let added = sqlx::query(
        "INSERT INTO group_members (group_id, principal_sid, principal_kind, principal_name, added_by)
         VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(principal.to_string())
    .bind(&input.principal_kind)
    .bind(&member_name)
    .bind(session.user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if added == 1 {
        let details = json!({
            "group": group, "principal_sid": principal.to_string(),
            "principal_kind": input.principal_kind, "principal_name": member_name,
        });
        record(
            &mut tx,
            &session,
            Action::GroupMemberAdded,
            id,
            details,
            &address,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_member(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path((id, sid)): Path<(Uuid, String)>,
) -> Result<StatusCode, Problem> {
    require_admin(&state, &session)?;
    let mut tx = state.db.begin().await?;
    let group = group_name(&mut tx, id).await?;
    let removed: Option<String> = sqlx::query_scalar(
        "DELETE FROM group_members WHERE group_id = $1 AND principal_sid = $2
         RETURNING principal_name",
    )
    .bind(id)
    .bind(&sid)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(member_name) = removed else {
        return Err(Problem::new(ErrorCode::NotFound));
    };
    let details = json!({ "group": group, "principal_sid": sid, "principal_name": member_name });
    record(
        &mut tx,
        &session,
        Action::GroupMemberRemoved,
        id,
        details,
        &address,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
