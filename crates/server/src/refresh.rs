//! Directory changes reach running sessions (#108).
//!
//! A session keeps the groups from its sign-in only for a while. Every few
//! minutes, and before every connection, remotehub asks the directory about
//! the user again: their groups replace those of all their sessions, and an
//! account that is disabled, expired or gone loses its sessions. A directory
//! that cannot be reached changes nothing, so an outage does not sign
//! everyone out; the next round asks again.
//!
//! Local accounts need none of this: blocking them in remotehub ends their
//! sessions at once (`api/users.rs`).

use std::time::Duration;

use remotehub_directory::{AuthError, Sid};
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::api::problem::{ErrorCode, Problem};
use crate::audit::{self, Action, Actor, Entry};
use crate::session::{self, Session};

/// How long the groups of a session hold before the directory is asked again.
pub const EVERY: Duration = Duration::from_secs(5 * 60);

/// How often [`run`] looks for sessions to check.
const ROUND: Duration = Duration::from_secs(60);

/// Asks the directory about the user with `sid` and applies the answer to
/// all their sessions. Returns whether they may keep them.
pub async fn user(state: &AppState, user_id: Uuid, sid: &str) -> Result<bool, sqlx::Error> {
    let (Some(directory), Ok(parsed)) = (state.directory.as_ref(), sid.parse::<Sid>()) else {
        return Ok(true);
    };
    let reason = match directory.refresh(&parsed).await {
        Ok(groups) => {
            let groups: Vec<String> = groups.iter().map(ToString::to_string).collect();
            sqlx::query("UPDATE sessions SET groups = $2, checked_at = now() WHERE user_id = $1")
                .bind(user_id)
                .bind(&groups)
                .execute(&state.db)
                .await?;
            return Ok(true);
        }
        Err(AuthError::Unavailable(error) | AuthError::Directory(error)) => {
            tracing::warn!(%error, %sid, "cannot check a signed-in user in the directory");
            return Ok(true);
        }
        Err(AuthError::AccountDisabled) => ErrorCode::AccountDisabled,
        Err(AuthError::AccountExpired) => ErrorCode::AccountExpired,
        Err(_) => ErrorCode::InvalidCredentials,
    };
    let mut tx = state.db.begin().await?;
    let ended = sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    audit::record(
        &mut *tx,
        Entry {
            actor: Actor {
                id: None,
                name: "directory",
            },
            action: Action::SessionsEndedByDirectory,
            object: Some(("user", user_id)),
            details: json!({ "reason": reason, "sid": sid, "sessions_ended": ended }),
            address: None,
        },
    )
    .await?;
    tx.commit().await?;
    tracing::warn!(%sid, ?reason, ended, "the directory ended a user's sessions");
    Ok(false)
}

/// The session with the directory's answer of now, before a connection:
/// its groups as they are, or `unauthenticated` if its user may no longer
/// sign in.
pub async fn before_connecting(state: &AppState, session: Session) -> Result<Session, Problem> {
    let Some(sid) = session.sid.as_deref() else {
        return Ok(session);
    };
    if !user(state, session.user_id, sid).await? {
        return Err(Problem::new(ErrorCode::Unauthenticated));
    }
    session::reload(&state.db, &session, state.settings.session.idle)
        .await?
        .ok_or(Problem::new(ErrorCode::Unauthenticated))
}

/// Checks every directory user whose sessions were last checked more than
/// [`EVERY`] ago.
pub async fn due(state: &AppState) -> Result<(), sqlx::Error> {
    let users: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT DISTINCT u.id, u.sid FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE u.kind = 'directory' AND u.sid IS NOT NULL AND s.expires_at > now()
           AND s.checked_at < now() - make_interval(secs => $1)",
    )
    .bind(EVERY.as_secs_f64())
    .fetch_all(&state.db)
    .await?;
    for (user_id, sid) in users {
        user(state, user_id, &sid).await?;
    }
    Ok(())
}

/// Runs [`due`] every minute, for as long as the server runs.
pub async fn run(state: AppState) {
    let mut ticks = tokio::time::interval(ROUND);
    loop {
        ticks.tick().await;
        if let Err(error) = due(&state).await {
            tracing::warn!(%error, "checking sessions against the directory failed");
        }
    }
}
