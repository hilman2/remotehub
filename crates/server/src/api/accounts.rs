//! Local accounts in Ory Kratos (#103, ADR 0010).
//!
//! - `/api/auth/…`: Kratos' public API for the browser, forwarded with the
//!   path after `/api/auth`. Only the self-service flows and the session
//!   check pass.
//! - `POST /api/session/local`: turns the browser's Kratos session into a
//!   remotehub session, once the account's second factor was used.
//! - `GET /api/session/methods`: which ways to sign in this instance offers.

use axum::Json;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{AppendHeaders, IntoResponse, Response};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use super::problem::{ErrorCode, Problem};
use super::session::{ClientAddress, Me};
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::auth::PER_ADDRESS;
use crate::kratos::{Kratos, KratosError, KratosSession, Whoami};
use crate::session::{self, Session};

/// The public API paths a browser needs; everything else stays hidden.
const PUBLIC_PATHS: [&str; 2] = ["self-service/", "sessions/whoami"];

fn kratos(state: &AppState) -> Result<&Kratos, Problem> {
    state
        .settings
        .kratos
        .as_ref()
        .ok_or(Problem::new(ErrorCode::NotFound))
}

fn unavailable(error: KratosError) -> Problem {
    tracing::warn!(%error, "Kratos failed");
    Problem::new(ErrorCode::AccountsUnavailable)
}

/// `ANY /api/auth/{*path}`: Kratos' public API. Sign-in attempts count
/// against the caller's address like directory sign-ins: Kratos itself
/// does not slow down guessing.
pub async fn forward(
    State(state): State<AppState>,
    ClientAddress(address): ClientAddress,
    Path(path): Path<String>,
    request: Request,
) -> Result<Response, Problem> {
    let kratos = kratos(&state)?;
    if !PUBLIC_PATHS.iter().any(|allowed| path.starts_with(allowed)) {
        return Err(Problem::new(ErrorCode::NotFound));
    }
    let signing_in = request.method() == Method::POST && path.starts_with("self-service/login");
    let keys = [(PER_ADDRESS, address.as_str())];
    if signing_in {
        state.limiter.check(&keys).map_err(|wait| {
            Problem::new(ErrorCode::TooManyAttempts).param("retry_after_seconds", wait.as_secs())
        })?;
    }
    let query = request
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let response = kratos
        .forward(&format!("/{path}{query}"), request)
        .await
        .map_err(unavailable)?;
    // A wrong password or code: the flow comes back with its messages.
    if signing_in && response.status() == StatusCode::BAD_REQUEST {
        state.limiter.failed(&keys);
    }
    Ok(response)
}

#[derive(Serialize)]
pub struct Methods {
    /// Active Directory through LDAP.
    directory: bool,
    /// Local accounts in Kratos.
    local: bool,
}

/// `GET /api/session/methods`, before anyone signs in.
pub async fn methods(State(state): State<AppState>) -> Json<Methods> {
    Json(Methods {
        directory: state.directory.is_some(),
        local: state.settings.kratos.is_some(),
    })
}

/// `POST /api/session/local`: a remotehub session for the local account
/// signed in to Kratos in this browser. Its second factor is required: an
/// account without one is sent to set it up first.
pub async fn sign_in(
    State(state): State<AppState>,
    ClientAddress(address): ClientAddress,
    headers: HeaderMap,
) -> Result<impl IntoResponse, Problem> {
    let kratos = kratos(&state)?;
    let found = match kratos.whoami(&headers).await.map_err(unavailable)? {
        Whoami::None => return Err(Problem::new(ErrorCode::Unauthenticated)),
        Whoami::SecondFactorPending => return Err(Problem::new(ErrorCode::SecondFactorRequired)),
        Whoami::Session(found) => found,
    };
    if !found.active || found.identity.state != "active" {
        return Err(Problem::new(ErrorCode::AccountDisabled));
    }
    // `highest_available`: aal1 means the account has no second factor.
    if found.authenticator_assurance_level != "aal2" {
        return Err(Problem::new(ErrorCode::SecondFactorSetupRequired));
    }

    let email = found.identity.traits.email.trim().to_lowercase();
    let name = Some(found.identity.traits.name.trim())
        .filter(|n| !n.is_empty())
        .unwrap_or(&email)
        .to_owned();
    let mut tx = state.db.begin().await?;
    let user_id = upsert_local_user(&mut tx, found.identity.id, &email, &name).await?;
    if session::blocked(&mut *tx, user_id).await? {
        drop(tx);
        return Err(super::session::refuse_blocked(&state, user_id, &email, &address).await);
    }
    let token = session::create(&mut *tx, user_id, &[], state.settings.session.max).await?;
    audit::record(
        &mut *tx,
        Entry {
            actor: Actor {
                id: Some(user_id),
                name: &email,
            },
            action: Action::SignIn,
            object: None,
            details: json!({ "kind": "local", "identity_id": found.identity.id }),
            address: Some(&address),
        },
    )
    .await?;
    tx.commit().await?;
    tracing::info!(username = %email, identity = %found.identity.id, %address, "signed in");

    let session = Session {
        user_id,
        username: email,
        display_name: name,
        kind: "local".to_owned(),
        sid: None,
        upn: None,
        groups: Vec::new(),
        identity_id: Some(found.identity.id),
    };
    let me = Me::of(&session, &state.settings);
    Ok((AppendHeaders([session::set_cookie(&token)]), Json(me)))
}

async fn upsert_local_user(
    tx: &mut sqlx::PgConnection,
    identity: Uuid,
    email: &str,
    name: &str,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO users (kind, identity_id, username, display_name, email, last_sign_in_at)
         VALUES ('local', $1, $2, $3, $2, now())
         ON CONFLICT (identity_id) DO UPDATE SET
             username = EXCLUDED.username,
             display_name = EXCLUDED.display_name,
             email = EXCLUDED.email,
             last_sign_in_at = now()
         RETURNING id",
    )
    .bind(identity)
    .bind(email)
    .bind(name)
    .fetch_one(&mut *tx)
    .await
}

/// Ends the Kratos session in this browser as well, when a local account
/// signs out of remotehub. Kratos being away does not keep anyone signed in
/// to remotehub, so failures are only logged.
pub async fn sign_out(state: &AppState, headers: &HeaderMap) {
    let Some(kratos) = state.settings.kratos.as_ref() else {
        return;
    };
    match kratos.whoami(headers).await {
        Ok(Whoami::Session(KratosSession { id, .. })) => {
            if let Err(error) = kratos.revoke(id).await {
                tracing::warn!(%error, "cannot end the Kratos session");
            }
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "cannot ask Kratos for the session"),
    }
}
