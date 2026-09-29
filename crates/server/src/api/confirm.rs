//! Confirming with the second factor that it is still them (#241, #242,
//! #243): a locked session is unlocked this way, and showing a secret or
//! connecting to a device that asks for it takes a confirmation of the last
//! minute (`session::CONFIRMED_FOR`).
//!
//! - `POST /api/session/confirm/start`: what the user can confirm with, an
//!   authenticator app or keys, and a challenge for the keys.
//! - `POST /api/session/confirm`: a code of the app, or a key's answer.
//! - `GET /api/session/factors`: whether the user has an app and keys.
//!
//! A key stands alone here, without the password, so it must check its
//! holder too (PIN, fingerprint or face). Directory users' factors are
//! remotehub's own, local accounts' are read from Kratos, and break-glass
//! accounts have their code. Five wrong answers in a row end the session.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{AppendHeaders, IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use super::accounts::{kratos, unavailable};
use super::catalog::body;
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::break_glass;
use crate::kratos::SecondFactors;
use crate::second_factor::{self, KeyAnswer, KeyUse};
use crate::session::{self, AnySession, Session};
use crate::totp;
use crate::webauthn::{self, RelyingParty};

/// Wrong answers in a row after which the session ends.
const MAX_FAILURES: i32 = 5;

/// What the user can confirm with.
#[derive(Debug, Serialize)]
pub struct Start {
    /// An authenticator app's code.
    app: bool,
    /// A key's challenge and the browser's options, if the user has keys.
    key: Option<KeyChallenge>,
}

#[derive(Debug, Serialize)]
pub struct KeyChallenge {
    challenge_id: Uuid,
    options: Value,
}

/// A confirmation: one of the two.
#[derive(Deserialize)]
pub struct Confirmation {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    key: Option<KeyAnswer>,
}

/// The factors of the user, as far as a confirmation goes.
enum Factors {
    /// remotehub's own (#107, #129).
    Directory,
    /// The break-glass account's code.
    BreakGlass,
    /// Kratos' (#103).
    Local(SecondFactors),
}

async fn factors(state: &AppState, session: &Session) -> Result<Factors, Problem> {
    match (session.kind.as_str(), session.identity_id) {
        ("directory", _) => Ok(Factors::Directory),
        ("break_glass", _) => Ok(Factors::BreakGlass),
        ("local", Some(identity)) => kratos(state)?
            .second_factors(identity)
            .await
            .map(Factors::Local)
            .map_err(unavailable),
        _ => Err(Problem::new(ErrorCode::Forbidden)),
    }
}

/// Whether the user has an authenticator app, and their keys' credential
/// IDs (base64url).
async fn available(state: &AppState, session: &Session) -> Result<(bool, Vec<String>), Problem> {
    let user = session.user_id;
    Ok(match factors(state, session).await? {
        Factors::Directory => (
            second_factor::has_app(&state.db, user).await?,
            second_factor::key_ids(&state.db, user).await?,
        ),
        Factors::BreakGlass => (true, Vec::new()),
        Factors::Local(factors) => (
            factors.app.is_some(),
            factors
                .keys
                .iter()
                .map(|key| URL_SAFE_NO_PAD.encode(&key.credential_id))
                .collect(),
        ),
    })
}

/// What the user has to confirm with, without a challenge: for the page's
/// hint to set up a passkey (#244).
#[derive(Debug, Serialize)]
pub struct Available {
    app: bool,
    keys: bool,
}

/// `GET /api/session/factors`
pub async fn factors_of(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Available>, Problem> {
    let (app, keys) = available(&state, &session).await?;
    Ok(Json(Available {
        app,
        keys: !keys.is_empty(),
    }))
}

pub async fn start(
    State(state): State<AppState>,
    AnySession(session): AnySession,
) -> Result<Json<Start>, Problem> {
    let user = session.user_id;
    let (app, keys) = available(&state, &session).await?;
    let key = if keys.is_empty() {
        None
    } else {
        let rp = RelyingParty::of(&state.settings.public_origin);
        let usage = KeyUse::Confirm;
        let (challenge_id, bytes) =
            second_factor::challenge(&state.db, user, usage.purpose()).await?;
        Some(KeyChallenge {
            challenge_id,
            options: second_factor::assertion_options(&rp, &bytes, &keys, usage),
        })
    };
    Ok(Json(Start { app, key }))
}

pub async fn confirm(
    State(state): State<AppState>,
    AnySession(session): AnySession,
    ClientAddress(address): ClientAddress,
    input: Result<Json<Confirmation>, JsonRejection>,
) -> Result<Response, Problem> {
    let input = body(input)?;
    let method = match (&input.code, &input.key) {
        (Some(_), None) => "app",
        (None, Some(_)) => "key",
        _ => return Err(Problem::new(ErrorCode::InvalidRequest)),
    };
    let factors = factors(&state, &session).await?;
    let rp = RelyingParty::of(&state.settings.public_origin);
    let now = totp::unix_now();
    let user = session.user_id;
    let mut tx = state.db.begin().await?;
    let right = match (factors, input.code.as_deref(), input.key.as_ref()) {
        (Factors::Directory, Some(code), _) => {
            second_factor::verify(&mut tx, &state.vault, user, code, now).await?
        }
        (Factors::Directory, _, Some(key)) => {
            second_factor::verify_key(&mut tx, &rp, user, key, KeyUse::Confirm).await?
        }
        (Factors::BreakGlass, Some(code), _) => {
            break_glass::verify_code(&state.db, &state.vault, user, code, now)
                .await
                .map_err(|error| {
                    tracing::error!(%error, "a break-glass code cannot be checked");
                    Problem::new(ErrorCode::Internal)
                })?
        }
        (Factors::Local(factors), Some(code), _) => {
            local_code(&mut tx, user, &factors, code, now).await?
        }
        (Factors::Local(factors), _, Some(key)) => {
            local_key(&mut tx, &rp, user, &factors, key).await?
        }
        _ => false,
    };
    let actor = Actor {
        id: Some(user),
        name: &session.username,
    };
    if right {
        sqlx::query(
            "UPDATE sessions SET confirmed_at = now(), last_seen_at = now(), confirm_failures = 0
             WHERE token_hash = $1",
        )
        .bind(&session.token_hash)
        .execute(&mut *tx)
        .await?;
        let details = json!({ "method": method, "unlocked": session.locked });
        audit::record(
            &mut *tx,
            entry(actor, Action::SessionConfirmed, details, &address),
        )
        .await?;
        tx.commit().await?;
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    // What the checks wrote stays: a used challenge is gone for good.
    let failures: i32 = sqlx::query_scalar(
        "UPDATE sessions SET confirm_failures = confirm_failures + 1
         WHERE token_hash = $1 RETURNING confirm_failures",
    )
    .bind(&session.token_hash)
    .fetch_one(&mut *tx)
    .await?;
    let ended = failures >= MAX_FAILURES;
    if ended {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(&session.token_hash)
            .execute(&mut *tx)
            .await?;
    }
    let details = json!({ "method": method, "ended": ended });
    audit::record(
        &mut *tx,
        entry(actor, Action::SessionConfirmFailed, details, &address),
    )
    .await?;
    tx.commit().await?;
    tracing::warn!(user = %session.username, %address, ended, "a confirmation failed");
    if ended {
        return Ok((
            AppendHeaders([session::clear_cookie(), session::clear_login_key_cookie()]),
            Problem::new(ErrorCode::Unauthenticated),
        )
            .into_response());
    }
    Err(Problem::new(ErrorCode::SecondFactorInvalid))
}

fn entry<'a>(actor: Actor<'a>, action: Action, details: Value, address: &'a str) -> Entry<'a> {
    Entry {
        actor,
        action,
        object: None,
        details,
        address: Some(address),
    }
}

/// A code of a local account's app: Kratos keeps the secret, remotehub the
/// last step it took, so that each code confirms once.
async fn local_code(
    tx: &mut PgConnection,
    user: Uuid,
    factors: &SecondFactors,
    code: &str,
    now: u64,
) -> Result<bool, Problem> {
    let Some(app) = &factors.app else {
        return Ok(false);
    };
    // Kratos' apps make codes as every app does: SHA-1, six digits, 30 s.
    if (app.algorithm, app.digits, app.period)
        != (totp::Algorithm::Sha1, totp::DIGITS, totp::PERIOD)
    {
        tracing::error!(
            "a local account's authenticator app has settings remotehub does not check"
        );
        return Ok(false);
    }
    let Some(step) = totp::step(&app.secret, code.trim(), now) else {
        return Ok(false);
    };
    let step = i64::try_from(step).unwrap_or(i64::MAX);
    let taken = sqlx::query(
        "INSERT INTO confirmation_codes (user_id, last_step) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET last_step = EXCLUDED.last_step
         WHERE confirmation_codes.last_step < EXCLUDED.last_step",
    )
    .bind(user)
    .bind(step)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    Ok(taken == 1)
}

/// A local account's key: Kratos keeps it, remotehub the counter of its
/// confirmations, and the higher of the two counts.
async fn local_key(
    tx: &mut PgConnection,
    rp: &RelyingParty,
    user: Uuid,
    factors: &SecondFactors,
    answer: &KeyAnswer,
) -> Result<bool, Problem> {
    let usage = KeyUse::Confirm;
    let Some(challenge) =
        second_factor::take_challenge(&mut *tx, user, usage.purpose(), answer.challenge_id).await?
    else {
        return Ok(false);
    };
    let Ok(credential_id) = answer.credential.credential_id() else {
        return Ok(false);
    };
    let Some(key) = factors
        .keys
        .iter()
        .find(|key| key.credential_id == credential_id)
    else {
        return Ok(false);
    };
    if !usage.accepts(&answer.credential) {
        return Ok(false);
    }
    let ours: Option<i64> = sqlx::query_scalar(
        "SELECT sign_count FROM confirmation_keys
         WHERE credential_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(&credential_id)
    .bind(user)
    .fetch_optional(&mut *tx)
    .await?;
    let ours = ours.map_or(0, |count| u32::try_from(count).unwrap_or(u32::MAX));
    let stored = key.counter.max(ours);
    match webauthn::verify(rp, &challenge, &key.public_key, stored, &answer.credential) {
        Ok(counter) => {
            sqlx::query(
                "INSERT INTO confirmation_keys (credential_id, user_id, sign_count)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (credential_id) DO UPDATE SET sign_count = EXCLUDED.sign_count
                 WHERE confirmation_keys.user_id = EXCLUDED.user_id",
            )
            .bind(&credential_id)
            .bind(user)
            .bind(i64::from(counter))
            .execute(&mut *tx)
            .await?;
            Ok(true)
        }
        Err(error) => {
            tracing::warn!(%error, %user, "a key's confirmation was refused");
            Ok(false)
        }
    }
}
