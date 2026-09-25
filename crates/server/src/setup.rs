//! First start (#143): a fresh installation has no administrator. The
//! installer prints a link with a one-time code (`remotehub setup-code`), and
//! the setup wizard in the browser creates the first administrator with it.
//! Until then, the API serves only the wizard ([`gate`]).
//!
//! The state lives in the `instance` row: the hash of the current code, the
//! administrator the wizard created, the last step done, and when setup was
//! completed. Once completed, setup never opens again, not even when every
//! administrator is gone later; the way back then is a break-glass account.

use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use sqlx::{PgExecutor, PgPool};

use crate::AppState;
use crate::api::problem::{ErrorCode, Problem};
use crate::session;

/// Where an installation stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// No administrator yet: only the wizard's first step is open.
    Pending,
    /// The wizard created the administrator, who goes through the other
    /// steps in their own session.
    Administrator,
    /// Done, for good.
    Complete,
}

/// What `instance` says about setup.
#[derive(Debug, Clone, Copy, sqlx::FromRow)]
pub struct Progress {
    pub administrator: Option<uuid::Uuid>,
    pub step: i16,
    pub completed: bool,
}

impl Progress {
    pub fn phase(&self) -> Phase {
        if self.completed {
            Phase::Complete
        } else if self.administrator.is_some() {
            Phase::Administrator
        } else {
            Phase::Pending
        }
    }
}

pub async fn progress<'e>(db: impl PgExecutor<'e>) -> Result<Progress, sqlx::Error> {
    sqlx::query_as(
        "SELECT setup_administrator AS administrator, setup_step AS step,
                setup_completed_at IS NOT NULL AS completed
         FROM instance",
    )
    .fetch_one(db)
    .await
}

/// Remembers that setup is complete, so that requests stop asking the
/// database once it is: it never opens again.
#[derive(Default)]
pub struct Completion(AtomicBool);

impl Completion {
    pub async fn phase(&self, db: &PgPool) -> Result<Phase, sqlx::Error> {
        if self.0.load(Ordering::Relaxed) {
            return Ok(Phase::Complete);
        }
        let phase = progress(db).await?.phase();
        if phase == Phase::Complete {
            self.0.store(true, Ordering::Relaxed);
        }
        Ok(phase)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("remotehub is set up already")]
    Complete,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Makes a new setup code, which replaces the previous one, and returns it.
/// 32 random bytes: nobody guesses it, so the database keeps only its hash.
pub async fn new_code(db: &PgPool) -> Result<String, SetupError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the OS has randomness");
    let code = URL_SAFE_NO_PAD.encode(bytes);
    let updated =
        sqlx::query("UPDATE instance SET setup_code_hash = $1 WHERE setup_completed_at IS NULL")
            .bind(session::hash(&code).as_slice())
            .execute(db)
            .await?;
    if updated.rows_affected() == 0 {
        return Err(SetupError::Complete);
    }
    Ok(code)
}

/// The wizard's address with the code in the fragment, which browsers never
/// send: the code reaches no proxy's log.
pub fn link(public_origin: &str, code: &str) -> String {
    format!("{}/setup#code={code}", public_origin.trim_end_matches('/'))
}

/// Requests allowed while no administrator exists: the wizard, the health
/// check, the SSH CA's public key, which devices fetch without signing in,
/// and signing in and out, with Kratos' flows for it. A session started now
/// reaches nothing else either.
fn open_while_pending(path: &str) -> bool {
    [
        ("/health", false),
        ("/ssh-ca.pub", false),
        ("/setup", true),
        ("/session", true),
        ("/auth", true),
    ]
    .iter()
    .any(|(open, below)| {
        path == *open || (*below && path.strip_prefix(open).is_some_and(|p| p.starts_with('/')))
    })
}

/// Middleware for `/api`: while no administrator exists, everything but the
/// wizard answers `setup_pending`.
pub async fn gate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    // Nested under `/api`, the path here lacks that prefix.
    if !open_while_pending(request.uri().path()) {
        match state.setup.phase(&state.db).await {
            Ok(Phase::Pending) => return Problem::new(ErrorCode::SetupPending).into_response(),
            Ok(_) => {}
            Err(error) => return Problem::from(error).into_response(),
        }
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_code_travels_in_the_fragment() {
        assert_eq!(
            link("https://remotehub.example.com/", "abc"),
            "https://remotehub.example.com/setup#code=abc"
        );
    }

    #[test]
    fn only_the_wizard_and_signing_in_are_open_while_pending() {
        for path in [
            "/health",
            "/ssh-ca.pub",
            "/setup",
            "/setup/administrator",
            "/session",
            "/session/break-glass",
            "/auth/self-service/login/browser",
        ] {
            assert!(open_while_pending(path), "{path}");
        }
        for path in [
            "/tree",
            "/users",
            "/health/x",
            "/setupx",
            "/sessions",
            "/authx",
        ] {
            assert!(!open_while_pending(path), "{path}");
        }
    }
}
