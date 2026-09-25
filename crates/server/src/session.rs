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
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use crate::api::problem::{ErrorCode, Problem};
use remotehub_model::Subject;

use crate::{AppState, Settings};

pub const COOKIE_NAME: &str = "__Host-remotehub-session";
/// Holds the key to the sealed sign-in password (ADR 0005); never stored on the server.
pub const LOGIN_KEY_COOKIE: &str = "__Host-remotehub-login-key";

/// The signed-in user of a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct Session {
    #[serde(skip)]
    pub user_id: Uuid,
    pub username: String,
    pub display_name: String,
    /// `directory`, `break_glass`, or `local` (an account in Kratos, #103).
    pub kind: String,
    /// The user's own SID (directory users only).
    #[serde(skip)]
    pub sid: Option<String>,
    /// `userPrincipalName` (directory users only).
    #[serde(skip)]
    pub upn: Option<String>,
    /// Group SIDs from sign-in; they hold for the whole session.
    #[serde(skip)]
    pub groups: Vec<String>,
    /// The Kratos identity of a local account (#103).
    #[serde(skip)]
    pub identity_id: Option<Uuid>,
    /// Groups of remotehub's own (`group:<id>`, #105) the user is in, by
    /// their principal or a directory group. Read at every request, so a
    /// change holds at once.
    #[serde(skip)]
    #[sqlx(default)]
    pub memberships: Vec<String>,
    /// Roles given to the user or one of their groups (#106), read at every
    /// request like the memberships.
    #[serde(skip)]
    #[sqlx(default)]
    pub roles: Vec<String>,
}

impl Session {
    /// Administrators manage remotehub itself: break-glass accounts, members
    /// of the configured admin groups, the configured local accounts (their
    /// user name is their e-mail address), and whoever has the role.
    pub fn is_admin(&self, settings: &Settings) -> bool {
        self.kind == "break_glass"
            || (self.kind == "local"
                && settings
                    .admin_accounts
                    .contains(&self.username.to_lowercase()))
            || self
                .groups
                .iter()
                .any(|g| settings.admin_groups.contains(g))
            || self.has_role(Role::Administrator)
    }

    pub fn has_role(&self, role: Role) -> bool {
        self.roles.iter().any(|r| r == role.as_str())
    }

    /// Auditors read the audit log; administrators may too.
    pub fn is_auditor(&self, settings: &Settings) -> bool {
        self.has_role(Role::Auditor) || self.is_admin(settings)
    }

    /// The roles as the UI sees them: the configured administrators count
    /// as administrators.
    pub fn role_names(&self, settings: &Settings) -> Vec<&'static str> {
        Role::ALL
            .iter()
            .filter(|role| match role {
                Role::Administrator => self.is_admin(settings),
                other => self.has_role(**other),
            })
            .map(|role| role.as_str())
            .collect()
    }

    /// The principal grants name this user by: the SID of a directory user,
    /// `local:<identity>` for a local account; never a name.
    pub fn principal(&self) -> Option<String> {
        self.sid
            .clone()
            .or_else(|| self.identity_id.map(|id| format!("local:{id}")))
    }

    /// Everything grants and rules may name this user by: the own principal,
    /// the directory groups and the groups of remotehub's own.
    pub fn sids(&self) -> Vec<String> {
        self.groups
            .iter()
            .chain(&self.memberships)
            .cloned()
            .chain(self.principal())
            .collect()
    }

    /// Who asks, for `authorize()`: [`Session::sids`], and whether they are
    /// an administrator.
    pub fn subject(&self, settings: &Settings) -> Subject {
        Subject {
            sids: self.sids().into_iter().collect(),
            admin: self.is_admin(settings),
        }
    }
}

/// Roles for remotehub itself (#106), given in `role_assignments`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Administrator,
    /// Reads the audit log.
    Auditor,
    /// Approves the recovery of vaults (#95).
    SecurityOfficer,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Administrator, Role::Auditor, Role::SecurityOfficer];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Administrator => "administrator",
            Role::Auditor => "auditor",
            Role::SecurityOfficer => "security_officer",
        }
    }

    pub fn parse(name: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|role| role.as_str() == name)
    }
}

/// SHA-256 of a session token: its key in the database and the context the
/// sign-in password is sealed to.
pub fn hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system provides randomness");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Whether administrators blocked the user in remotehub (#104): no sign-in
/// then, whatever the directory or Kratos says.
pub async fn blocked<'e>(db: impl PgExecutor<'e>, user_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT blocked_at IS NOT NULL FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(db)
        .await
}

/// Starts a session and returns its token for the cookie.
pub async fn create<'e>(
    db: impl PgExecutor<'e>,
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
    let session: Option<Session> = sqlx::query_as(
        "SELECT s.user_id, s.groups, u.username, u.display_name, u.kind, u.sid, u.upn,
                u.identity_id, own.memberships,
                ARRAY(SELECT DISTINCT r.role FROM role_assignments r
                      WHERE r.principal_sid = ANY (s.groups || own.memberships)
                         OR r.principal_sid = u.sid
                         OR r.principal_sid = 'local:' || u.identity_id) AS roles
         FROM sessions s JOIN users u ON u.id = s.user_id
         CROSS JOIN LATERAL (
             SELECT ARRAY(SELECT DISTINCT 'group:' || m.group_id FROM group_members m
                          WHERE m.principal_sid = ANY (s.groups)
                             OR m.principal_sid = u.sid
                             OR m.principal_sid = 'local:' || u.identity_id) AS memberships
         ) own
         WHERE s.token_hash = $1
           AND u.blocked_at IS NULL
           AND s.expires_at > now()
           AND s.last_seen_at > now() - make_interval(secs => $2)",
    )
    .bind(token_hash.as_slice())
    .bind(idle.as_secs_f64())
    .fetch_optional(db)
    .await?;
    let Some(session) = session else {
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
    Ok(Some(session))
}

pub async fn delete<'e>(db: impl PgExecutor<'e>, token: &str) -> Result<(), sqlx::Error> {
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

/// Seals the sign-in password with a fresh key bound to this session and
/// returns the key for the user's cookie. Only the ciphertext is stored.
pub async fn keep_sign_in_password<'e>(
    db: impl PgExecutor<'e>,
    token: &str,
    password: &[u8],
) -> Result<String, sqlx::Error> {
    let key = remotehub_vault::detached::new_key();
    let sealed = remotehub_vault::detached::seal(&key, &hash(token), password);
    sqlx::query("UPDATE sessions SET login_secret = $2 WHERE token_hash = $1")
        .bind(hash(token).as_slice())
        .bind(sealed)
        .execute(db)
        .await?;
    Ok(URL_SAFE_NO_PAD.encode(key.as_slice()))
}

/// The sign-in password of the session, if it was kept and the request
/// carries the key cookie.
pub async fn sign_in_password(
    db: &PgPool,
    headers: &HeaderMap,
) -> Result<Option<zeroize::Zeroizing<Vec<u8>>>, sqlx::Error> {
    let (Some(token), Some(key)) = (token(headers), cookie(headers, LOGIN_KEY_COOKIE)) else {
        return Ok(None);
    };
    let Ok(key) = <[u8; 32]>::try_from(URL_SAFE_NO_PAD.decode(key).unwrap_or_default().as_slice())
    else {
        return Ok(None);
    };
    let sealed: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT login_secret FROM sessions WHERE token_hash = $1")
            .bind(hash(&token).as_slice())
            .fetch_optional(db)
            .await?
            .flatten();
    Ok(sealed.and_then(|sealed| {
        remotehub_vault::detached::open(&zeroize::Zeroizing::new(key), &hash(&token), &sealed).ok()
    }))
}

/// The session token from the request's cookies.
pub fn token(headers: &HeaderMap) -> Option<String> {
    cookie(headers, COOKIE_NAME)
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(n, _)| *n == name)
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
    clear(COOKIE_NAME)
}

/// `Set-Cookie` for the key to the sealed sign-in password.
pub fn set_login_key_cookie(key: &str) -> (axum::http::HeaderName, HeaderValue) {
    let value = format!("{LOGIN_KEY_COOKIE}={key}; Path=/; HttpOnly; Secure; SameSite=Strict");
    (
        SET_COOKIE,
        HeaderValue::from_str(&value).expect("key is base64url"),
    )
}

pub fn clear_login_key_cookie() -> (axum::http::HeaderName, HeaderValue) {
    clear(LOGIN_KEY_COOKIE)
}

fn clear(name: &str) -> (axum::http::HeaderName, HeaderValue) {
    let value = format!("{name}=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0");
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
