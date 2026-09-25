//! The second factor of directory users (#107, `crate::second_factor`).
//!
//! - `GET /api/account/second-factor`: whether the caller has one and must.
//! - `POST /api/account/second-factor/offer`: a new secret to set up.
//! - `PUT`/`DELETE /api/account/second-factor`: set it up with a code of
//!   the app, or remove it with one; removing is refused while a rule asks
//!   for it.
//! - `GET /api/second-factor-principals`, `PUT`/`DELETE
//!   /api/second-factor-principals/{sid}`: who must have one (administrators).
//! - `DELETE /api/users/{id}/second-factor`: an administrator removes a
//!   user's factor, e.g. for a lost phone; with a rule, the user sets up a
//!   new one at the next sign-in.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::catalog::{body, invalid, name};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::principal::PrincipalId;
use crate::second_factor;
use crate::session::Session;
use crate::totp;

#[derive(Serialize)]
pub struct Status {
    /// Only directory users set one up here.
    available: bool,
    enrolled: bool,
    required: bool,
}

#[derive(Serialize)]
pub struct Offer {
    secret: String,
    uri: String,
}

#[derive(Deserialize)]
pub struct Enrollment {
    secret: String,
    code: String,
}

#[derive(Deserialize)]
pub struct Confirmation {
    code: String,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Rule {
    principal_sid: String,
    principal_kind: String,
    principal_name: String,
}

#[derive(Deserialize)]
pub struct NewRule {
    principal_kind: String,
    principal_name: String,
}

fn directory_user(session: &Session) -> Result<(), Problem> {
    if session.kind == "directory" {
        Ok(())
    } else {
        Err(invalid("kind"))
    }
}

fn require_admin(state: &AppState, session: &Session) -> Result<(), Problem> {
    if session.is_admin(&state.settings) {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::Forbidden))
    }
}

fn entry<'a>(
    session: &'a Session,
    action: Action,
    object: Option<(&'a str, Uuid)>,
    details: Value,
    address: &'a str,
) -> Entry<'a> {
    Entry {
        actor: Actor {
            id: Some(session.user_id),
            name: &session.username,
        },
        action,
        object,
        details,
        address: Some(address),
    }
}

pub async fn status(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Status>, Problem> {
    Ok(Json(Status {
        available: session.kind == "directory",
        enrolled: second_factor::enrolled(&state.db, session.user_id).await?,
        required: second_factor::required(&state.db, &session.sids()).await?,
    }))
}

pub async fn offer(session: Session) -> Result<Json<Offer>, Problem> {
    directory_user(&session)?;
    let (secret, uri) = second_factor::offer(&session.username);
    Ok(Json(Offer {
        secret: secret.as_str().to_owned(),
        uri: uri.as_str().to_owned(),
    }))
}

pub async fn enroll(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<Enrollment>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    directory_user(&session)?;
    let input = body(input)?;
    let mut tx = state.db.begin().await?;
    let user = session.user_id;
    let now = totp::unix_now();
    if !second_factor::enroll(&mut tx, &state.vault, user, &input.secret, &input.code, now).await? {
        return Err(Problem::new(ErrorCode::SecondFactorInvalid));
    }
    let object = Some(("user", user));
    let recorded = entry(
        &session,
        Action::SecondFactorEnrolled,
        object,
        json!({}),
        &address,
    );
    audit::record(&mut *tx, recorded).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<Confirmation>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    directory_user(&session)?;
    let input = body(input)?;
    if second_factor::required(&state.db, &session.sids()).await? {
        return Err(Problem::new(ErrorCode::Forbidden));
    }
    let mut tx = state.db.begin().await?;
    let user = session.user_id;
    let now = totp::unix_now();
    if !second_factor::verify(&mut tx, &state.vault, user, &input.code, now).await? {
        return Err(Problem::new(ErrorCode::SecondFactorInvalid));
    }
    second_factor::remove(&mut *tx, user).await?;
    let object = Some(("user", user));
    let recorded = entry(
        &session,
        Action::SecondFactorRemoved,
        object,
        json!({}),
        &address,
    );
    audit::record(&mut *tx, recorded).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn reset(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    require_admin(&state, &session)?;
    let mut tx = state.db.begin().await?;
    let username: Option<String> =
        sqlx::query_scalar("SELECT username FROM users WHERE id = $1 AND kind = 'directory'")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let username = username.ok_or(Problem::new(ErrorCode::NotFound))?;
    if !second_factor::remove(&mut *tx, id).await? {
        return Err(Problem::new(ErrorCode::NotFound));
    }
    let details = json!({ "username": username });
    let recorded = entry(
        &session,
        Action::SecondFactorReset,
        Some(("user", id)),
        details,
        &address,
    );
    audit::record(&mut *tx, recorded).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn rules(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<Rule>>, Problem> {
    require_admin(&state, &session)?;
    let rules = sqlx::query_as(
        "SELECT principal_sid, principal_kind, principal_name FROM second_factor_principals
         ORDER BY lower(principal_name)",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rules))
}

pub async fn require(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(sid): Path<String>,
    input: Result<Json<NewRule>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    require_admin(&state, &session)?;
    let input = body(input)?;
    let sid: PrincipalId = sid.parse().map_err(|_| invalid("principal_sid"))?;
    if !sid.check(&state.db, &input.principal_kind).await? {
        return Err(invalid("principal_sid"));
    }
    let principal_name = name(&input.principal_name, "principal_name")?;
    let mut tx = state.db.begin().await?;
    let added = sqlx::query(
        "INSERT INTO second_factor_principals (principal_sid, principal_kind, principal_name, created_by)
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
        let recorded = entry(
            &session,
            Action::SecondFactorRequired,
            None,
            details,
            &address,
        );
        audit::record(&mut *tx, recorded).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn waive(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(sid): Path<String>,
) -> Result<StatusCode, Problem> {
    require_admin(&state, &session)?;
    let mut tx = state.db.begin().await?;
    let removed: Option<Rule> = sqlx::query_as(
        "DELETE FROM second_factor_principals WHERE principal_sid = $1
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
    let recorded = entry(
        &session,
        Action::SecondFactorWaived,
        None,
        details,
        &address,
    );
    audit::record(&mut *tx, recorded).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
