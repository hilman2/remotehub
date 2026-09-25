//! `/api/users`: everyone who has signed in, for administrators (#104).
//!
//! - `GET /api/users`: the list, with source, state, last sign-in and open
//!   sessions.
//! - `POST /api/users/invite`: a new local account and its one-time code.
//! - `POST`/`DELETE /api/users/{id}/block`: blocks or unblocks anyone; a
//!   blocked user has no session and cannot sign in.
//! - `DELETE /api/users/{id}/sessions`: ends a user's sessions.
//! - `POST /api/users/{id}/recovery`: a local account's second factors go,
//!   and a new code lets its owner set up everything again.
//! - `DELETE /api/users/{id}`: deletes a local account.
//!
//! Every change is audited. For local accounts, Kratos changes first: if it
//! is not reachable, nothing changes in remotehub either.

use std::time::Duration;

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
use crate::kratos::{Invitation, Kratos, KratosError};
use crate::session::Session;

/// How long an invitation's or a recovery's code lasts.
const CODE_LIFETIME: Duration = Duration::from_secs(48 * 3600);

#[derive(Serialize, sqlx::FromRow)]
pub struct UserRow {
    id: Uuid,
    /// `directory`, `local` or `break_glass`.
    kind: String,
    username: String,
    display_name: String,
    email: Option<String>,
    last_sign_in_at: Option<String>,
    blocked: bool,
    /// Sessions that have not run out.
    sessions: i64,
}

/// The one-time code an administrator hands to the account's owner.
#[derive(Serialize)]
pub struct Code {
    link: String,
    code: String,
    expires_at: String,
}

impl From<Invitation> for Code {
    fn from(invitation: Invitation) -> Self {
        Code {
            link: invitation.recovery_link,
            code: invitation.recovery_code,
            expires_at: invitation.expires_at,
        }
    }
}

fn require_admin(state: &AppState, session: &Session) -> Result<(), Problem> {
    if session.is_admin(&state.settings) {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::Forbidden))
    }
}

fn kratos(state: &AppState) -> Result<&Kratos, Problem> {
    state
        .settings
        .kratos
        .as_ref()
        .ok_or(Problem::new(ErrorCode::AccountsUnavailable))
}

fn unavailable(error: KratosError) -> Problem {
    tracing::warn!(%error, "Kratos failed");
    Problem::new(ErrorCode::AccountsUnavailable)
}

/// A user the request names, as far as the changes here need it.
#[derive(sqlx::FromRow)]
struct Target {
    id: Uuid,
    kind: String,
    username: String,
    identity_id: Option<Uuid>,
}

impl Target {
    /// The Kratos identity of a local account; anything else has none.
    fn identity(&self) -> Result<Uuid, Problem> {
        self.identity_id
            .filter(|_| self.kind == "local")
            .ok_or_else(|| invalid("kind"))
    }
}

async fn target(state: &AppState, id: Uuid) -> Result<Target, Problem> {
    sqlx::query_as(
        "SELECT id, kind, username, identity_id FROM users WHERE id = $1 AND kind <> 'deleted'",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(Problem::new(ErrorCode::NotFound))
}

async fn record(
    state: &AppState,
    session: &Session,
    action: Action,
    user: Uuid,
    details: Value,
    address: &str,
) -> Result<(), Problem> {
    audit::record(
        &state.db,
        Entry {
            actor: Actor {
                id: Some(session.user_id),
                name: &session.username,
            },
            action,
            object: Some(("user", user)),
            details,
            address: Some(address),
        },
    )
    .await?;
    Ok(())
}

async fn end_sessions(state: &AppState, user: Uuid) -> Result<u64, Problem> {
    Ok(sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user)
        .execute(&state.db)
        .await?
        .rows_affected())
}

pub async fn list(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<UserRow>>, Problem> {
    require_admin(&state, &session)?;
    let users = sqlx::query_as(
        "SELECT u.id, u.kind, u.username, u.display_name, u.email,
                to_char(u.last_sign_in_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                    AS last_sign_in_at,
                u.blocked_at IS NOT NULL AS blocked,
                (SELECT count(*) FROM sessions s WHERE s.user_id = u.id AND s.expires_at > now())
                    AS sessions
         FROM users u WHERE u.kind <> 'deleted'
         ORDER BY lower(u.display_name), u.id",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(users))
}

#[derive(Deserialize)]
pub struct NewAccount {
    email: String,
    #[serde(default)]
    name: String,
}

pub async fn invite(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<NewAccount>, JsonRejection>,
) -> Result<(StatusCode, Json<Code>), Problem> {
    require_admin(&state, &session)?;
    let input = body(input)?;
    let email = input.email.trim().to_lowercase();
    if email.len() > 320 || !email.contains('@') || email.chars().any(char::is_whitespace) {
        return Err(invalid("email"));
    }
    let name = input.name.trim();
    if name.chars().count() > 200 || name.chars().any(char::is_control) {
        return Err(invalid("name"));
    }
    let invitation = kratos(&state)?
        .invite(&email, name, CODE_LIFETIME)
        .await
        .map_err(|error| match error {
            KratosError::Unexpected {
                status: StatusCode::CONFLICT,
                ..
            } => Problem::new(ErrorCode::NameTaken),
            other => unavailable(other),
        })?;
    audit::record(
        &state.db,
        Entry {
            actor: Actor {
                id: Some(session.user_id),
                name: &session.username,
            },
            action: Action::AccountInvited,
            object: None,
            details: json!({ "email": email, "identity_id": invitation.identity_id }),
            address: Some(&address),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(invitation.into())))
}

pub async fn block(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    require_admin(&state, &session)?;
    let user = target(&state, id).await?;
    // Nobody locks themselves out by a slip.
    if user.id == session.user_id {
        return Err(Problem::new(ErrorCode::Forbidden));
    }
    if let Ok(identity) = user.identity() {
        let kratos = kratos(&state)?;
        kratos
            .set_active(identity, false)
            .await
            .map_err(unavailable)?;
        kratos.revoke_all(identity).await.map_err(unavailable)?;
    }
    sqlx::query("UPDATE users SET blocked_at = coalesce(blocked_at, now()) WHERE id = $1")
        .bind(user.id)
        .execute(&state.db)
        .await?;
    let ended = end_sessions(&state, user.id).await?;
    record(
        &state,
        &session,
        Action::UserBlocked,
        user.id,
        json!({ "username": user.username, "sessions_ended": ended }),
        &address,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unblock(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    require_admin(&state, &session)?;
    let user = target(&state, id).await?;
    if let Ok(identity) = user.identity() {
        kratos(&state)?
            .set_active(identity, true)
            .await
            .map_err(unavailable)?;
    }
    sqlx::query("UPDATE users SET blocked_at = NULL WHERE id = $1")
        .bind(user.id)
        .execute(&state.db)
        .await?;
    record(
        &state,
        &session,
        Action::UserUnblocked,
        user.id,
        json!({ "username": user.username }),
        &address,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn end(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    require_admin(&state, &session)?;
    let user = target(&state, id).await?;
    if let Ok(identity) = user.identity() {
        kratos(&state)?
            .revoke_all(identity)
            .await
            .map_err(unavailable)?;
    }
    let ended = end_sessions(&state, user.id).await?;
    record(
        &state,
        &session,
        Action::UserSessionsEnded,
        user.id,
        json!({ "username": user.username, "sessions_ended": ended }),
        &address,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn recovery(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<Json<Code>, Problem> {
    require_admin(&state, &session)?;
    let user = target(&state, id).await?;
    let identity = user.identity()?;
    let kratos = kratos(&state)?;
    // Kratos lets a recovery through only without second factors; the
    // owner sets up a new one before the next session.
    kratos
        .remove_second_factors(identity)
        .await
        .map_err(unavailable)?;
    kratos.revoke_all(identity).await.map_err(unavailable)?;
    let code = kratos
        .recovery_code(identity, CODE_LIFETIME)
        .await
        .map_err(unavailable)?;
    end_sessions(&state, user.id).await?;
    record(
        &state,
        &session,
        Action::AccountRecoveryIssued,
        user.id,
        json!({ "username": user.username }),
        &address,
    )
    .await?;
    Ok(Json(code.into()))
}

pub async fn delete(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    require_admin(&state, &session)?;
    let user = target(&state, id).await?;
    let identity = user.identity()?;
    if user.id == session.user_id {
        return Err(Problem::new(ErrorCode::Forbidden));
    }
    kratos(&state)?
        .delete(identity)
        .await
        .map_err(unavailable)?;
    // The row stays, because the audit log refers to it.
    sqlx::query("UPDATE users SET kind = 'deleted', identity_id = NULL WHERE id = $1")
        .bind(user.id)
        .execute(&state.db)
        .await?;
    end_sessions(&state, user.id).await?;
    record(
        &state,
        &session,
        Action::AccountDeleted,
        user.id,
        json!({ "username": user.username, "identity_id": identity }),
        &address,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
