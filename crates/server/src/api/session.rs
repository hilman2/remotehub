//! `/api/session`: sign in (POST), who am I (GET), sign out (DELETE).

use std::net::SocketAddr;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{AppendHeaders, IntoResponse};
use remotehub_directory::{AuthError, Identity};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgExecutor;
use uuid::Uuid;

use super::problem::{ErrorCode, Problem};
use crate::audit::{self, Action, Actor, Entry};
use crate::auth::{PER_ADDRESS, PER_USER};
use crate::break_glass;
use crate::session::{self, Session};
use crate::{AppState, Settings};

#[derive(Deserialize)]
pub struct SignIn {
    username: String,
    password: SecretString,
}

/// The signed-in user as the UI sees them.
#[derive(Debug, Serialize)]
pub struct Me {
    username: String,
    display_name: String,
    kind: String,
    /// May manage remotehub: folders at the top, grants, the audit log.
    admin: bool,
}

impl Me {
    pub fn of(session: &Session, settings: &Settings) -> Self {
        Me {
            username: session.username.clone(),
            display_name: session.display_name.clone(),
            kind: session.kind.clone(),
            admin: session.is_admin(settings),
        }
    }
}

/// The client's address as seen by this server (a reverse proxy's address
/// when there is one).
pub struct ClientAddress(pub String);

impl<S: Send + Sync> FromRequestParts<S> for ClientAddress {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let address = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip().to_string())
            .unwrap_or_else(|| "unknown".to_owned());
        Ok(ClientAddress(address))
    }
}

pub async fn sign_in(
    State(state): State<AppState>,
    ClientAddress(address): ClientAddress,
    body: Result<Json<SignIn>, JsonRejection>,
) -> Result<impl IntoResponse, Problem> {
    let Json(SignIn { username, password }) =
        body.map_err(|_| Problem::new(ErrorCode::InvalidRequest))?;
    let keys = [
        (PER_USER, username.as_str()),
        (PER_ADDRESS, address.as_str()),
    ];
    state.limiter.check(&keys).map_err(|wait| {
        Problem::new(ErrorCode::TooManyAttempts).param("retry_after_seconds", wait.as_secs())
    })?;

    let directory = state
        .directory
        .as_ref()
        .ok_or(Problem::new(ErrorCode::DirectoryUnavailable))?;
    let identity = match directory.authenticate(&username, &password).await {
        Ok(identity) => identity,
        Err(error) => {
            if !matches!(error, AuthError::Unavailable(_) | AuthError::Directory(_)) {
                state.limiter.failed(&keys);
            }
            let problem = problem_for(&error);
            tracing::warn!(username = ?username, %address, reason = %error, "sign-in failed");
            audit::record(
                &state.db,
                Entry {
                    actor: Actor {
                        id: None,
                        name: typed_name(&username),
                    },
                    action: Action::SignInFailed,
                    object: None,
                    details: json!({ "reason": problem.code }),
                    address: Some(&address),
                },
            )
            .await?;
            return Err(problem);
        }
    };
    state.limiter.succeeded(&username);

    let groups: Vec<String> = identity.groups.iter().map(ToString::to_string).collect();
    let mut tx = state.db.begin().await?;
    let user_id = upsert_directory_user(&mut *tx, &identity).await?;
    let token = session::create(&mut *tx, user_id, &groups, state.settings.session.max).await?;
    let mut cookies = vec![session::set_cookie(&token)];
    if state.settings.own_account_connections {
        let key =
            session::keep_sign_in_password(&mut *tx, &token, password.expose_secret().as_bytes())
                .await?;
        cookies.push(session::set_login_key_cookie(&key));
    }
    audit::record(
        &mut *tx,
        Entry {
            actor: Actor {
                id: Some(user_id),
                name: &identity.username,
            },
            action: Action::SignIn,
            object: None,
            details: json!({ "sid": identity.sid, "kind": "directory" }),
            address: Some(&address),
        },
    )
    .await?;
    tx.commit().await?;
    tracing::info!(username = %identity.username, sid = %identity.sid, %address, "signed in");

    let session = Session {
        user_id,
        username: identity.username,
        display_name: identity.display_name,
        kind: "directory".to_owned(),
        sid: Some(identity.sid.to_string()),
        groups,
    };
    let me = Me::of(&session, &state.settings);
    Ok((AppendHeaders(cookies), Json(me)))
}

#[derive(Deserialize)]
pub struct BreakGlassSignIn {
    username: String,
    password: SecretString,
    code: String,
}

/// Sign-in with a break-glass account: password and TOTP code. Every
/// attempt is audited with `break_glass: true`, and a success is logged as
/// a warning, so emergency access never goes unnoticed.
pub async fn sign_in_break_glass(
    State(state): State<AppState>,
    ClientAddress(address): ClientAddress,
    body: Result<Json<BreakGlassSignIn>, JsonRejection>,
) -> Result<impl IntoResponse, Problem> {
    let Json(BreakGlassSignIn {
        username,
        password,
        code,
    }) = body.map_err(|_| Problem::new(ErrorCode::InvalidRequest))?;
    let keys = [
        (PER_USER, username.as_str()),
        (PER_ADDRESS, address.as_str()),
    ];
    state.limiter.check(&keys).map_err(|wait| {
        Problem::new(ErrorCode::TooManyAttempts).param("retry_after_seconds", wait.as_secs())
    })?;

    let account = break_glass::authenticate(&state.db, &state.vault, &username, &password, &code)
        .await
        .map_err(|error| {
            tracing::error!(%error, "break-glass sign-in failed internally");
            Problem::new(ErrorCode::Internal)
        })?;
    let Some(account) = account else {
        state.limiter.failed(&keys);
        tracing::warn!(username = ?username, %address, "break-glass sign-in failed");
        audit::record(
            &state.db,
            Entry {
                actor: Actor {
                    id: None,
                    name: typed_name(&username),
                },
                action: Action::SignInFailed,
                object: None,
                details: json!({ "reason": ErrorCode::InvalidCredentials, "break_glass": true }),
                address: Some(&address),
            },
        )
        .await?;
        return Err(Problem::new(ErrorCode::InvalidCredentials));
    };
    state.limiter.succeeded(&username);

    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE users SET last_sign_in_at = now() WHERE id = $1")
        .bind(account.user_id)
        .execute(&mut *tx)
        .await?;
    let token = session::create(&mut *tx, account.user_id, &[], state.settings.session.max).await?;
    audit::record(
        &mut *tx,
        Entry {
            actor: Actor {
                id: Some(account.user_id),
                name: &account.username,
            },
            action: Action::SignIn,
            object: None,
            details: json!({ "kind": "local", "break_glass": true }),
            address: Some(&address),
        },
    )
    .await?;
    tx.commit().await?;
    tracing::warn!(username = %account.username, %address, "BREAK-GLASS sign-in");

    let session = Session {
        user_id: account.user_id,
        username: account.username,
        display_name: account.display_name,
        kind: "local".to_owned(),
        sid: None,
        groups: Vec::new(),
    };
    let me = Me::of(&session, &state.settings);
    // Break-glass accounts have no directory account to connect with.
    Ok((AppendHeaders(vec![session::set_cookie(&token)]), Json(me)))
}

pub async fn current(State(state): State<AppState>, session: Session) -> Json<Me> {
    Json(Me::of(&session, &state.settings))
}

pub async fn sign_out(
    State(state): State<AppState>,
    ClientAddress(address): ClientAddress,
    headers: HeaderMap,
) -> Result<impl IntoResponse, Problem> {
    if let Some(token) = session::token(&headers)
        && let Some(ended) = session::lookup(&state.db, &token, state.settings.session.idle).await?
    {
        let mut tx = state.db.begin().await?;
        session::delete(&mut *tx, &token).await?;
        audit::record(
            &mut *tx,
            Entry {
                actor: Actor {
                    id: Some(ended.user_id),
                    name: &ended.username,
                },
                action: Action::SignOut,
                object: None,
                details: json!({}),
                address: Some(&address),
            },
        )
        .await?;
        tx.commit().await?;
    }
    Ok((
        StatusCode::NO_CONTENT,
        [session::clear_cookie(), session::clear_login_key_cookie()],
    ))
}

/// A typed user name for the audit log, cut to a sane length.
fn typed_name(username: &str) -> &str {
    let name = username.trim();
    match name.char_indices().nth(128) {
        Some((cut, _)) => &name[..cut],
        None => name,
    }
}

async fn upsert_directory_user<'e>(
    db: impl PgExecutor<'e>,
    identity: &Identity,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO users (kind, sid, guid, username, display_name, email, last_sign_in_at)
         VALUES ('directory', $1, $2, $3, $4, $5, now())
         ON CONFLICT (sid) DO UPDATE SET
             guid = EXCLUDED.guid,
             username = EXCLUDED.username,
             display_name = EXCLUDED.display_name,
             email = EXCLUDED.email,
             last_sign_in_at = now()
         RETURNING id",
    )
    .bind(identity.sid.as_str())
    .bind(identity.guid)
    .bind(&identity.username)
    .bind(&identity.display_name)
    .bind(&identity.email)
    .fetch_one(db)
    .await
}

/// What a client learns about a failed sign-in. The directory only reports
/// an account's state after a correct password, so these codes do not help
/// to guess accounts.
fn problem_for(error: &AuthError) -> Problem {
    Problem::new(match error {
        AuthError::InvalidCredentials => ErrorCode::InvalidCredentials,
        AuthError::AccountDisabled => ErrorCode::AccountDisabled,
        AuthError::AccountLocked => ErrorCode::AccountLocked,
        AuthError::AccountExpired => ErrorCode::AccountExpired,
        AuthError::PasswordExpired | AuthError::PasswordMustChange => {
            ErrorCode::PasswordChangeRequired
        }
        AuthError::Unavailable(_) | AuthError::Directory(_) => ErrorCode::DirectoryUnavailable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_names_are_cut_at_a_character_boundary() {
        assert_eq!(typed_name(" alice "), "alice");
        let long = "ä".repeat(200);
        assert_eq!(typed_name(&long).chars().count(), 128);
    }
}
