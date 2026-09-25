//! `/api/roles`: who has which role for remotehub itself (#106), for
//! administrators.
//!
//! - `GET /api/roles`: every role with the users and groups that have it.
//! - `PUT`/`DELETE /api/roles/{role}/members/{sid}`: give or take a role.
//!
//! The setup wizard gives the first administrator the role (#143). The last
//! user or group with the role keeps it, so there is always someone besides
//! the break-glass accounts. Every change is audited.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::catalog::{body, invalid, name};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::principal::PrincipalId;
use crate::session::{Role, Session};

#[derive(Serialize)]
pub struct Assignments {
    role: &'static str,
    members: Vec<Member>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Member {
    #[serde(skip)]
    role: String,
    sid: String,
    kind: String,
    name: String,
}

#[derive(Deserialize)]
pub struct MemberInput {
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

fn role(name: &str) -> Result<Role, Problem> {
    Role::parse(name).ok_or(Problem::new(ErrorCode::NotFound))
}

fn entry<'a>(
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

pub async fn list(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<Assignments>>, Problem> {
    require_admin(&session)?;
    let members: Vec<Member> = sqlx::query_as(
        "SELECT role, principal_sid AS sid, principal_kind AS kind, principal_name AS name
         FROM role_assignments ORDER BY principal_kind DESC, lower(principal_name)",
    )
    .fetch_all(&state.db)
    .await?;
    let mut members = members;
    let roles = Role::ALL
        .iter()
        .map(|role| {
            let (mine, rest) = members.drain(..).partition(|m| m.role == role.as_str());
            members = rest;
            Assignments {
                role: role.as_str(),
                members: mine,
            }
        })
        .collect();
    Ok(Json(roles))
}

pub async fn assign(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path((role_name, sid)): Path<(String, String)>,
    input: Result<Json<MemberInput>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    require_admin(&session)?;
    let role = role(&role_name)?;
    let input = body(input)?;
    let principal: PrincipalId = sid.parse().map_err(|_| invalid("principal_sid"))?;
    if !principal.check(&state.db, &input.principal_kind).await? {
        return Err(invalid("principal_sid"));
    }
    let member_name = name(&input.principal_name, "principal_name")?;
    let mut tx = state.db.begin().await?;
    let added = sqlx::query(
        "INSERT INTO role_assignments (role, principal_sid, principal_kind, principal_name, created_by)
         VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
    )
    .bind(role.as_str())
    .bind(principal.to_string())
    .bind(&input.principal_kind)
    .bind(&member_name)
    .bind(session.user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if added == 1 {
        let details = json!({
            "role": role.as_str(), "principal_sid": principal.to_string(),
            "principal_kind": input.principal_kind, "principal_name": member_name,
        });
        audit::record(
            &mut *tx,
            entry(&session, Action::RoleAssigned, details, &address),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn revoke(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path((role_name, sid)): Path<(String, String)>,
) -> Result<StatusCode, Problem> {
    require_admin(&session)?;
    let role = role(&role_name)?;
    let mut tx = state.db.begin().await?;
    if role == Role::Administrator {
        // Two administrators taking the role from each other at once must
        // not both see the other one still there.
        sqlx::query("SELECT 1 FROM instance FOR UPDATE")
            .execute(&mut *tx)
            .await?;
    }
    let removed: Option<String> = sqlx::query_scalar(
        "DELETE FROM role_assignments WHERE role = $1 AND principal_sid = $2
         RETURNING principal_name",
    )
    .bind(role.as_str())
    .bind(&sid)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(member_name) = removed else {
        return Err(Problem::new(ErrorCode::NotFound));
    };
    if role == Role::Administrator {
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM role_assignments WHERE role = $1")
            .bind(role.as_str())
            .fetch_one(&mut *tx)
            .await?;
        if left == 0 {
            return Err(Problem::new(ErrorCode::LastAdministrator));
        }
    }
    let details =
        json!({ "role": role.as_str(), "principal_sid": sid, "principal_name": member_name });
    audit::record(
        &mut *tx,
        entry(&session, Action::RoleRevoked, details, &address),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
