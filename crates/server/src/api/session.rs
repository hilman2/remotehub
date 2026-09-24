//! `/api/session`: sign in (POST), who am I (GET), sign out (DELETE).

use std::net::SocketAddr;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use remotehub_directory::{AuthError, Identity};
use secrecy::SecretString;
use serde::Deserialize;
use uuid::Uuid;

use super::problem::{ErrorCode, Problem};
use crate::AppState;
use crate::auth::{PER_ADDRESS, PER_USER};
use crate::session::{self, Session};

#[derive(Deserialize)]
pub struct SignIn {
    username: String,
    password: SecretString,
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
            tracing::warn!(username = ?username, %address, reason = %error, "sign-in failed");
            return Err(problem_for(&error));
        }
    };
    state.limiter.succeeded(&username);

    let user_id = upsert_directory_user(&state, &identity).await?;
    let groups: Vec<String> = identity.groups.iter().map(ToString::to_string).collect();
    let token = session::create(&state.db, user_id, &groups, state.settings.session.max).await?;
    tracing::info!(username = %identity.username, sid = %identity.sid, %address, "signed in");

    let me = Session {
        user_id,
        username: identity.username,
        display_name: identity.display_name,
        kind: "directory".to_owned(),
        groups,
    };
    Ok(([session::set_cookie(&token)], Json(me)))
}

pub async fn current(session: Session) -> Json<Session> {
    Json(session)
}

pub async fn sign_out(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, Problem> {
    if let Some(token) = session::token(&headers) {
        session::delete(&state.db, &token).await?;
    }
    Ok((StatusCode::NO_CONTENT, [session::clear_cookie()]))
}

async fn upsert_directory_user(state: &AppState, identity: &Identity) -> Result<Uuid, sqlx::Error> {
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
    .fetch_one(&state.db)
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
