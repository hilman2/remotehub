//! Server-side sessions (ADR 0005).
//!
//! A session is a random 256-bit token in the cookie `__Host-remotehub-session`
//! (HttpOnly, Secure, SameSite=Strict, host-only). The database keeps only its
//! SHA-256 hash, the user and the group SIDs from sign-in. A session ends
//! after the idle time without requests, after the maximum lifetime, or when
//! the user signs out.

use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::AppState;
use crate::api::problem::{ErrorCode, Problem};

pub const COOKIE_NAME: &str = "__Host-remotehub-session";

/// The signed-in user of a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Session {
    #[serde(skip)]
    pub user_id: Uuid,
    pub username: String,
    pub display_name: String,
    /// `directory` or `local` (break-glass).
    pub kind: String,
    /// Group SIDs from sign-in; they hold for the whole session.
    #[serde(skip)]
    pub groups: Vec<String>,
}

fn hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system provides randomness");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Starts a session and returns its token for the cookie.
pub async fn create(
    db: &PgPool,
    user_id: Uuid,
    groups: &[String],
    max: Duration,
) -> Result<String, sqlx::Error> {
    let token = new_token();
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, groups, expires_at)
         VALUES ($1, $2, $3, now() + make_interval(secs => $4))",
    )
    .bind(hash(&token).as_slice())
    .bind(user_id)
    .bind(groups)
    .bind(max.as_secs_f64())
    .execute(db)
    .await?;
    Ok(token)
}

/// The session for a token, if it is still valid; marks it as used.
pub async fn lookup(
    db: &PgPool,
    token: &str,
    idle: Duration,
) -> Result<Option<Session>, sqlx::Error> {
    let token_hash = hash(token);
    let row: Option<(Uuid, Vec<String>, String, String, String)> = sqlx::query_as(
        "SELECT s.user_id, s.groups, u.username, u.display_name, u.kind
         FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1
           AND s.expires_at > now()
           AND s.last_seen_at > now() - make_interval(secs => $2)",
    )
    .bind(token_hash.as_slice())
    .bind(idle.as_secs_f64())
    .fetch_optional(db)
    .await?;
    let Some((user_id, groups, username, display_name, kind)) = row else {
        return Ok(None);
    };
    // Keeps the session alive; at most one write per minute and session.
    sqlx::query(
        "UPDATE sessions SET last_seen_at = now()
         WHERE token_hash = $1 AND last_seen_at < now() - interval '1 minute'",
    )
    .bind(token_hash.as_slice())
    .execute(db)
    .await?;
    Ok(Some(Session {
        user_id,
        username,
        display_name,
        kind,
        groups,
    }))
}

pub async fn delete(db: &PgPool, token: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(hash(token).as_slice())
        .execute(db)
        .await?;
    Ok(())
}

/// Removes sessions that can no longer be used.
pub async fn purge(db: &PgPool, idle: Duration) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM sessions
         WHERE expires_at <= now() OR last_seen_at <= now() - make_interval(secs => $1)",
    )
    .bind(idle.as_secs_f64())
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

/// The session token from the request's cookies.
pub fn token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE_NAME)
        .map(|(_, value)| value.to_owned())
        .filter(|value| !value.is_empty())
}

/// `Set-Cookie` that stores the token for the browser session.
pub fn set_cookie(token: &str) -> (axum::http::HeaderName, HeaderValue) {
    let value = format!("{COOKIE_NAME}={token}; Path=/; HttpOnly; Secure; SameSite=Strict");
    (
        SET_COOKIE,
        HeaderValue::from_str(&value).expect("token is base64url"),
    )
}

/// `Set-Cookie` that removes the token.
pub fn clear_cookie() -> (axum::http::HeaderName, HeaderValue) {
    let value = format!("{COOKIE_NAME}=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0");
    (
        SET_COOKIE,
        HeaderValue::from_str(&value).expect("static cookie"),
    )
}

/// Handlers that take a [`Session`] only run for signed-in users; everyone
/// else gets the problem `unauthenticated`.
impl FromRequestParts<AppState> for Session {
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Problem> {
        let token = token(&parts.headers).ok_or(Problem::new(ErrorCode::Unauthenticated))?;
        lookup(&state.db, &token, state.settings.session.idle)
            .await?
            .ok_or(Problem::new(ErrorCode::Unauthenticated))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_long() {
        let a = new_token();
        assert_ne!(a, new_token());
        assert_eq!(URL_SAFE_NO_PAD.decode(&a).unwrap().len(), 32);
    }

    #[test]
    fn finds_the_token_among_other_cookies() {
        let mut headers = HeaderMap::new();
        headers.append(COOKIE, HeaderValue::from_static("theme=dark; other=1"));
        assert_eq!(token(&headers), None);
        headers.append(
            COOKIE,
            HeaderValue::from_static("a=b; __Host-remotehub-session=abc123; c=d"),
        );
        assert_eq!(token(&headers).as_deref(), Some("abc123"));
    }

    #[test]
    fn the_cookie_is_locked_down() {
        let (_, value) = set_cookie("abc");
        let value = value.to_str().unwrap();
        for attribute in ["__Host-", "Path=/", "HttpOnly", "Secure", "SameSite=Strict"] {
            assert!(value.contains(attribute), "{attribute} in {value}");
        }
        assert!(!value.contains("Domain"));
        assert!(clear_cookie().1.to_str().unwrap().contains("Max-Age=0"));
    }
}
