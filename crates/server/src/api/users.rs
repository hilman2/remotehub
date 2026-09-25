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
use crate::kratos::{self, Invitation, Kratos, KratosError};
use crate::mail;
use crate::principal::PrincipalId;
use crate::session::Session;
use remotehub_i18n::{Locale, Message};

/// How long an invitation's or a recovery's code lasts.
pub(super) const CODE_LIFETIME: Duration = Duration::from_secs(48 * 3600);

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
    /// Signs in with a second factor: local and break-glass accounts
    /// always, directory users once they set one up (#107).
    second_factor: bool,
    /// Sessions that have not run out.
    sessions: i64,
}

/// The one-time code an administrator hands to the account's owner, and
/// whether it went out by mail too (#145).
#[derive(Serialize)]
pub struct Code {
    link: String,
    code: String,
    expires_at: String,
    mailed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    mail_failure: Option<mail::Failure>,
}

impl From<Invitation> for Code {
    fn from(invitation: Invitation) -> Self {
        Code {
            link: invitation.recovery_link,
            code: invitation.recovery_code,
            expires_at: invitation.expires_at,
            mailed: false,
            mail_failure: None,
        }
    }
}

/// Whether and in which language a code goes out by mail.
#[derive(Deserialize, Default)]
pub struct MailRequest {
    #[serde(default)]
    pub(super) send_mail: bool,
    /// `en` or `de`, as the administrator chose.
    #[serde(default)]
    pub(super) language: String,
}

/// Mails `code` to `to` if `request` asks for it, and notes the outcome in
/// it. `invited` picks the text: an invitation, or a new sign-in code.
async fn mail_code(
    state: &AppState,
    request: &MailRequest,
    to: &str,
    name: &str,
    invited: bool,
    code: &mut Code,
) {
    if !request.send_mail {
        return;
    }
    let locale = Locale::from_accept_language(&request.language);
    let (link, value, expires) = (
        code.link.clone(),
        code.code.clone(),
        mail::expiry(&code.expires_at),
    );
    let name = if name.is_empty() { to } else { name }.to_owned();
    let (subject, body) = if invited {
        (
            Message::MailInvitationSubject {},
            Message::MailInvitationBody {
                name,
                link,
                code: value,
                expires,
            },
        )
    } else {
        (
            Message::MailSignInCodeSubject {},
            Message::MailSignInCodeBody {
                name,
                link,
                code: value,
                expires,
            },
        )
    };
    match mail::send(
        &state.db,
        &state.vault,
        mail::Outgoing::new(to, locale, &subject, &body),
    )
    .await
    {
        Ok(()) => code.mailed = true,
        Err(failure) => {
            tracing::warn!(step = ?failure.step, detail = %failure.detail, "cannot mail a code");
            code.mail_failure = Some(failure);
        }
    }
}

fn require_admin(session: &Session) -> Result<(), Problem> {
    if session.is_admin() {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::Forbidden))
    }
}

pub(super) fn kratos(state: &AppState) -> Result<&Kratos, Problem> {
    state
        .settings
        .kratos
        .as_ref()
        .ok_or(Problem::new(ErrorCode::AccountsUnavailable))
}

pub(super) fn unavailable(error: KratosError) -> Problem {
    tracing::warn!(%error, "Kratos failed");
    Problem::new(ErrorCode::AccountsUnavailable)
}

/// The address and the name of a new local account, checked: the address
/// in lower case, the name trimmed.
pub(super) fn new_account(input: &NewAccount) -> Result<(String, &str), Problem> {
    let email = input.email.trim().to_lowercase();
    if email.len() > 320 || !email.contains('@') || email.chars().any(char::is_whitespace) {
        return Err(invalid("email"));
    }
    let name = input.name.trim();
    if name.chars().count() > 200 || name.chars().any(char::is_control) {
        return Err(invalid("name"));
    }
    Ok((email, name))
}

/// Kratos' answer to an invitation, as a problem: an address it knows
/// already is taken.
pub(super) fn invite_failed(error: KratosError) -> Problem {
    match error {
        KratosError::Unexpected {
            status: StatusCode::CONFLICT,
            ..
        } => Problem::new(ErrorCode::NameTaken),
        other => unavailable(other),
    }
}

/// A user the request names, as far as the changes here need it.
#[derive(sqlx::FromRow)]
struct Target {
    id: Uuid,
    username: String,
    identity_id: Option<Uuid>,
}

impl Target {
    /// The Kratos identity of a local account; anything else has none.
    fn identity(&self) -> Result<Uuid, Problem> {
        self.identity_id.ok_or_else(|| invalid("kind"))
    }
}

async fn target(state: &AppState, id: Uuid) -> Result<Target, Problem> {
    sqlx::query_as(
        "SELECT id, username, identity_id FROM users WHERE id = $1 AND kind <> 'deleted'",
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
    require_admin(&session)?;
    let users = sqlx::query_as(
        "SELECT u.id, u.kind, u.username, u.display_name, u.email,
                to_char(u.last_sign_in_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                    AS last_sign_in_at,
                u.blocked_at IS NOT NULL AS blocked,
                (u.kind <> 'directory'
                 OR EXISTS (SELECT 1 FROM second_factors f WHERE f.user_id = u.id)
                 OR EXISTS (SELECT 1 FROM security_keys k WHERE k.user_id = u.id))
                    AS second_factor,
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
    pub(super) email: String,
    #[serde(default)]
    pub(super) name: String,
    #[serde(flatten)]
    pub(super) mail: MailRequest,
}

pub async fn invite(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<NewAccount>, JsonRejection>,
) -> Result<(StatusCode, Json<Code>), Problem> {
    require_admin(&session)?;
    let input = body(input)?;
    let (email, name) = new_account(&input)?;
    let invitation = kratos(&state)?
        .invite(&email, name, CODE_LIFETIME)
        .await
        .map_err(invite_failed)?;
    let mut tx = state.db.begin().await?;
    let user = kratos::add_invited(&mut *tx, &invitation, &email, name).await?;
    audit::record(
        &mut *tx,
        Entry {
            actor: Actor {
                id: Some(session.user_id),
                name: &session.username,
            },
            action: Action::AccountInvited,
            object: Some(("user", user)),
            details: json!({ "email": email, "identity_id": invitation.identity_id }),
            address: Some(&address),
        },
    )
    .await?;
    tx.commit().await?;
    let mut code = Code::from(invitation);
    mail_code(&state, &input.mail, &email, name, true, &mut code).await;
    Ok((StatusCode::CREATED, Json(code)))
}

pub async fn block(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    require_admin(&session)?;
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
    require_admin(&session)?;
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
    require_admin(&session)?;
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
    request: Option<Json<MailRequest>>,
) -> Result<Json<Code>, Problem> {
    require_admin(&session)?;
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
    let mut code = Code::from(code);
    let request = request.map(|Json(r)| r).unwrap_or_default();
    // A local account's user name is its e-mail address.
    mail_code(&state, &request, &user.username, "", false, &mut code).await;
    Ok(Json(code))
}

pub async fn delete(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    require_admin(&session)?;
    let user = target(&state, id).await?;
    let identity = user.identity()?;
    if user.id == session.user_id {
        return Err(Problem::new(ErrorCode::Forbidden));
    }
    kratos(&state)?
        .delete(identity)
        .await
        .map_err(unavailable)?;
    // The row stays, because the audit log refers to it. What names the
    // account goes: nobody can ever sign in as it again.
    let principal = PrincipalId::Local(identity).to_string();
    let mut tx = state.db.begin().await?;
    for statement in [
        "DELETE FROM grants WHERE principal_sid = $1",
        "DELETE FROM purpose_principals WHERE principal_sid = $1",
        "DELETE FROM group_members WHERE principal_sid = $1",
        "DELETE FROM role_assignments WHERE principal_sid = $1",
        "DELETE FROM second_factor_principals WHERE principal_sid = $1",
    ] {
        sqlx::query(statement)
            .bind(&principal)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE users SET kind = 'deleted', identity_id = NULL WHERE id = $1")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
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
