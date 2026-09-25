//! `/api/setup`: the setup wizard (#143); see [`crate::setup`].
//!
//! - `GET /api/setup`: the phase and the last step done, for everyone.
//! - `POST /api/setup/code`: checks a setup code, so the wizard can say at
//!   once that its link is stale.
//! - `POST /api/setup/administrator`: step 1, with the code: a local account
//!   with the administrator role, answered with its invitation.
//! - `PUT /api/setup/step`: the administrator's progress through the other
//!   steps, so the wizard continues there after a new sign-in.
//! - `POST /api/setup/break-glass`: the break-glass account, once and only
//!   during setup; otherwise only the CLI creates one.
//! - `POST /api/setup/complete`: ends setup for good.
//!
//! Wrong codes count against the limit on failed sign-ins of the client's
//! address. Every change is audited, before step 1 with the actor `setup`.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::catalog::{body, invalid};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use super::users::{self, CODE_LIFETIME, Code, NewAccount};
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::auth::PER_ADDRESS;
use crate::break_glass::{self, BreakGlassError};
use crate::kratos;
use crate::principal::PrincipalId;
use crate::session::{self, Role, Session};
use crate::setup::{self, Phase};

/// The break-glass account setup creates.
const BREAK_GLASS_NAME: &str = "emergency";
/// Steps after the first, as the wizard counts them; the last is the end.
const LAST_STEP: i16 = 6;

#[derive(Serialize)]
pub struct Status {
    phase: Phase,
    step: i16,
}

pub async fn status(State(state): State<AppState>) -> Result<Json<Status>, Problem> {
    let progress = setup::progress(&state.db).await?;
    Ok(Json(Status {
        phase: progress.phase(),
        step: progress.step,
    }))
}

#[derive(Deserialize)]
pub struct CodeInput {
    code: String,
}

/// Whether `code` is the current setup code; a wrong one counts as a
/// failed attempt of `address`.
fn check_code(
    state: &AppState,
    stored: Option<&[u8]>,
    code: &str,
    address: &str,
) -> Result<(), Problem> {
    if stored == Some(session::hash(code.trim()).as_slice()) {
        return Ok(());
    }
    state.limiter.failed(&[(PER_ADDRESS, address)]);
    tracing::warn!(%address, "wrong setup code");
    Err(Problem::new(ErrorCode::SetupCodeInvalid))
}

fn limit(state: &AppState, address: &str) -> Result<(), Problem> {
    state
        .limiter
        .check(&[(PER_ADDRESS, address)])
        .map_err(|wait| {
            Problem::new(ErrorCode::TooManyAttempts).param("retry_after_seconds", wait.as_secs())
        })
}

#[derive(sqlx::FromRow)]
struct Open {
    code_hash: Option<Vec<u8>>,
    administrator: Option<Uuid>,
    completed: bool,
}

macro_rules! open {
    () => {
        "SELECT setup_code_hash AS code_hash, setup_administrator AS administrator,
                setup_completed_at IS NOT NULL AS completed
         FROM instance"
    };
}

pub async fn code(
    State(state): State<AppState>,
    ClientAddress(address): ClientAddress,
    input: Result<Json<CodeInput>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let input = body(input)?;
    limit(&state, &address)?;
    let open: Open = sqlx::query_as(open!()).fetch_one(&state.db).await?;
    if open.completed {
        return Err(Problem::new(ErrorCode::SetupDone));
    }
    check_code(&state, open.code_hash.as_deref(), &input.code, &address)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct NewAdministrator {
    code: String,
    #[serde(flatten)]
    account: NewAccount,
}

const SETUP: Actor<'static> = Actor {
    id: None,
    name: "setup",
};

pub async fn administrator(
    State(state): State<AppState>,
    ClientAddress(address): ClientAddress,
    input: Result<Json<NewAdministrator>, JsonRejection>,
) -> Result<(StatusCode, Json<Code>), Problem> {
    let input = body(input)?;
    let (email, name) = users::new_account(&input.account)?;
    limit(&state, &address)?;
    let kratos = users::kratos(&state)?;

    // The row lock makes a second browser with the same code wait, and then
    // find the code spent.
    let mut tx = state.db.begin().await?;
    let open: Open = sqlx::query_as(concat!(open!(), " FOR UPDATE"))
        .fetch_one(&mut *tx)
        .await?;
    if open.completed {
        return Err(Problem::new(ErrorCode::SetupDone));
    }
    check_code(&state, open.code_hash.as_deref(), &input.code, &address)?;

    // A new code (`remotehub setup-code`) while the administrator of an
    // earlier one never signed in: that account goes, e.g. because its
    // address was mistyped and the invitation never arrived.
    if let Some(previous) = open.administrator {
        let (identity, signed_in): (Option<Uuid>, bool) = sqlx::query_as(
            "SELECT identity_id, last_sign_in_at IS NOT NULL FROM users WHERE id = $1",
        )
        .bind(previous)
        .fetch_one(&mut *tx)
        .await?;
        if signed_in {
            return Err(Problem::new(ErrorCode::SetupDone));
        }
        if let Some(identity) = identity {
            match kratos.delete(identity).await {
                Ok(())
                | Err(kratos::KratosError::Unexpected {
                    status: StatusCode::NOT_FOUND,
                    ..
                }) => {}
                Err(error) => return Err(users::unavailable(error)),
            }
            sqlx::query("DELETE FROM role_assignments WHERE principal_sid = $1")
                .bind(PrincipalId::Local(identity).to_string())
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE users SET kind = 'deleted', identity_id = NULL WHERE id = $1")
            .bind(previous)
            .execute(&mut *tx)
            .await?;
    }

    let invitation = kratos
        .invite(&email, name, CODE_LIFETIME)
        .await
        .map_err(users::invite_failed)?;
    let user = kratos::add_invited(&mut *tx, &invitation, &email, name).await?;
    let principal = PrincipalId::Local(invitation.identity_id).to_string();
    let shown = if name.is_empty() {
        email.as_str()
    } else {
        name
    };
    sqlx::query(
        "INSERT INTO role_assignments (role, principal_sid, principal_kind, principal_name)
         VALUES ($1, $2, 'user', $3)",
    )
    .bind(Role::Administrator.as_str())
    .bind(&principal)
    .bind(shown)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE instance SET setup_code_hash = NULL, setup_administrator = $1, setup_step = 1",
    )
    .bind(user)
    .execute(&mut *tx)
    .await?;
    for (action, object, details) in [
        (
            Action::AccountInvited,
            Some(("user", user)),
            json!({ "email": email, "identity_id": invitation.identity_id }),
        ),
        (
            Action::RoleAssigned,
            None,
            json!({
                "role": Role::Administrator.as_str(),
                "principal_sid": principal,
                "principal_name": shown,
            }),
        ),
    ] {
        audit::record(
            &mut *tx,
            Entry {
                actor: SETUP,
                action,
                object,
                details,
                address: Some(&address),
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(invitation.into())))
}

/// The steps after the first are the administrator's, and only while setup
/// is open.
async fn require_setting_up(state: &AppState, session: &Session) -> Result<(), Problem> {
    if !session.is_admin() {
        return Err(Problem::new(ErrorCode::Forbidden));
    }
    match state.setup.phase(&state.db).await? {
        Phase::Administrator => Ok(()),
        Phase::Complete => Err(Problem::new(ErrorCode::SetupDone)),
        // Only a break-glass account from the CLI can be an administrator
        // now; the wizard's first step is still to come.
        Phase::Pending => Err(Problem::new(ErrorCode::SetupPending)),
    }
}

#[derive(Deserialize)]
pub struct StepInput {
    step: i16,
}

pub async fn step(
    State(state): State<AppState>,
    session: Session,
    input: Result<Json<StepInput>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let input = body(input)?;
    require_setting_up(&state, &session).await?;
    if !(2..LAST_STEP).contains(&input.step) {
        return Err(invalid("step"));
    }
    sqlx::query("UPDATE instance SET setup_step = greatest(setup_step, $1)")
        .bind(input.step)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Borrows from the zeroizing buffers of [`break_glass::Issued`], so the
/// secrets exist in no other copy than the response.
#[derive(Serialize)]
struct BreakGlass<'a> {
    username: &'a str,
    password: &'a str,
    totp_secret: &'a str,
    totp_uri: &'a str,
}

pub async fn create_break_glass(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
) -> Result<Response, Problem> {
    require_setting_up(&state, &session).await?;
    if !break_glass::list(&state.db).await?.is_empty() {
        return Err(Problem::new(ErrorCode::BreakGlassExists));
    }
    let issued = break_glass::create(&state.db, &state.vault, BREAK_GLASS_NAME)
        .await
        .map_err(|error| match error {
            BreakGlassError::Exists(_) => Problem::new(ErrorCode::BreakGlassExists),
            other => {
                tracing::error!(error = %other, "cannot create the break-glass account");
                Problem::new(ErrorCode::Internal)
            }
        })?;
    let account: Uuid = sqlx::query_scalar(
        "SELECT id FROM users WHERE kind = 'break_glass' AND lower(username) = lower($1)",
    )
    .bind(&issued.username)
    .fetch_one(&state.db)
    .await?;
    audit::record(
        &state.db,
        Entry {
            actor: Actor {
                id: Some(session.user_id),
                name: &session.username,
            },
            action: Action::BreakGlassCreated,
            object: Some(("user", account)),
            details: json!({ "username": issued.username }),
            address: Some(&address),
        },
    )
    .await?;
    // Shown once, like the CLI shows it; nothing may keep the answer.
    let shown = BreakGlass {
        username: &issued.username,
        password: &issued.password,
        totp_secret: &issued.totp_secret,
        totp_uri: &issued.totp_uri,
    };
    Ok((
        StatusCode::CREATED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(shown),
    )
        .into_response())
}

pub async fn complete(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
) -> Result<StatusCode, Problem> {
    require_setting_up(&state, &session).await?;
    let mut tx = state.db.begin().await?;
    let done = sqlx::query(
        "UPDATE instance SET setup_completed_at = now(), setup_code_hash = NULL, setup_step = $1
         WHERE setup_completed_at IS NULL",
    )
    .bind(LAST_STEP)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(Problem::new(ErrorCode::SetupDone));
    }
    audit::record(
        &mut *tx,
        Entry {
            actor: Actor {
                id: Some(session.user_id),
                name: &session.username,
            },
            action: Action::SetupCompleted,
            object: None,
            details: json!({}),
            address: Some(&address),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
