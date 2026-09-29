//! Server-side sessions (ADR 0005).
//!
//! A session is a random 256-bit token in the cookie `__Host-remotehub-session`
//! (HttpOnly, Secure, SameSite=Strict, host-only). The database keeps only its
//! SHA-256 hash, the user and the group SIDs from sign-in. A session ends
//! after the maximum lifetime or when the user signs out; after the idle time
//! without requests or input into a connection it locks, and a confirmation
//! with the second factor unlocks it (#241).
//!
//! The browser extension (#201) has sessions of its own in the same table,
//! with the client `extension`. Their token travels as a bearer token, never
//! as a cookie, and opens only the routes under `/api/extension/`
//! (`api/extension.rs`); a browser session opens none of those.

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

use crate::AppState;

pub const COOKIE_NAME: &str = "__Host-remotehub-session";
/// Holds the key to the sealed sign-in password (ADR 0005); never stored on the server.
pub const LOGIN_KEY_COOKIE: &str = "__Host-remotehub-login-key";

/// The signed-in user of a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct Session {
    /// Its key in the database, to read it again.
    #[serde(skip)]
    pub token_hash: Vec<u8>,
    /// Names the session where its token hash must not show, as on
    /// *My account* for the extension's sessions.
    #[serde(skip)]
    pub id: Uuid,
    /// `web` for a browser signed in to remotehub's pages, `extension` for
    /// the browser extension (#201).
    #[serde(skip)]
    pub client: String,
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
    /// Idle longer than the idle time (#241): only a confirmation or signing
    /// out takes the session then; everything else answers `session_locked`.
    #[serde(skip)]
    #[sqlx(default)]
    pub locked: bool,
    /// Confirmed with the second factor within [`CONFIRMED_FOR`] (#242, #243).
    #[serde(skip)]
    #[sqlx(default)]
    pub confirmed: bool,
}

/// How long a confirmation holds: showing and then copying one secret asks
/// once (#242).
pub const CONFIRMED_FOR: Duration = Duration::from_secs(60);

impl Session {
    /// `confirmation_required` unless the session was confirmed with the
    /// second factor within [`CONFIRMED_FOR`]: for showing a secret (#242)
    /// and for devices that ask for it (#243).
    pub fn require_confirmation(&self) -> Result<(), Problem> {
        if self.confirmed {
            Ok(())
        } else {
            Err(Problem::new(ErrorCode::ConfirmationRequired))
        }
    }

    /// Administrators manage remotehub itself: break-glass accounts, and
    /// whoever has the role, given by the setup wizard (#143) or on *Users*.
    pub fn is_admin(&self) -> bool {
        self.kind == "break_glass" || self.has_role(Role::Administrator)
    }

    pub fn has_role(&self, role: Role) -> bool {
        self.roles.iter().any(|r| r == role.as_str())
    }

    /// Auditors read the audit log; administrators may too.
    pub fn is_auditor(&self) -> bool {
        self.has_role(Role::Auditor) || self.is_admin()
    }

    /// The roles as the UI sees them: break-glass accounts count as
    /// administrators.
    pub fn role_names(&self) -> Vec<&'static str> {
        Role::ALL
            .iter()
            .filter(|role| match role {
                Role::Administrator => self.is_admin(),
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
    pub fn subject(&self) -> Subject {
        Subject {
            sids: self.sids().into_iter().collect(),
            admin: self.is_admin(),
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

/// Starts a session of the browser extension for `user_id` (#201) and
/// returns its token. `name` is the browser it runs in, for *My account*.
pub async fn create_extension<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    groups: &[String],
    max: Duration,
    name: &str,
) -> Result<String, sqlx::Error> {
    let token = new_token();
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, groups, expires_at, client, name)
         VALUES ($1, $2, $3, now() + make_interval(secs => $4), 'extension', $5)",
    )
    .bind(hash(&token).as_slice())
    .bind(user_id)
    .bind(groups)
    .bind(max.as_secs_f64())
    .bind(name)
    .execute(db)
    .await?;
    Ok(token)
}

/// A random code of 256 bits, e.g. for the extension to trade for a token.
pub fn new_code() -> String {
    new_token()
}

/// The session for a token, if it is still valid, locked or not; marks an
/// unlocked one as used.
pub async fn lookup(
    db: &PgPool,
    token: &str,
    idle: Duration,
) -> Result<Option<Session>, sqlx::Error> {
    find(db, token, idle, true).await
}

/// The session for a token, like [`lookup`]; marks it as used only with
/// `keep_alive`.
async fn find(
    db: &PgPool,
    token: &str,
    idle: Duration,
    keep_alive: bool,
) -> Result<Option<Session>, sqlx::Error> {
    let token_hash = hash(token);
    let Some(session) = by_hash(db, &token_hash, idle).await? else {
        return Ok(None);
    };
    if keep_alive && !session.locked {
        touch(db, &token_hash, idle).await?;
    }
    Ok(Some(session))
}

/// Input into an open connection (#239): it keeps the session alive like a
/// request, and while the session is locked it does not reach the device
/// (#241). One query a minute while unlocked, at most one every two seconds
/// while locked.
pub struct Activity {
    db: PgPool,
    token_hash: Vec<u8>,
    idle: Duration,
    checked: Option<std::time::Instant>,
    open: bool,
}

impl Activity {
    pub fn of(state: &AppState, session: &Session) -> Self {
        Activity {
            db: state.db.clone(),
            token_hash: session.token_hash.clone(),
            idle: state.settings.session.idle,
            checked: None,
            open: !session.locked,
        }
    }

    /// Whether this input may go on to the device.
    pub async fn input(&mut self) -> bool {
        let every = Duration::from_secs(if self.open { 60 } else { 2 });
        if self.checked.is_some_and(|at| at.elapsed() < every) {
            return self.open;
        }
        self.checked = Some(std::time::Instant::now());
        // Unlocked and not over: marked as used, as `touch` does.
        let open = sqlx::query(
            "UPDATE sessions
             SET last_seen_at = CASE WHEN last_seen_at < now() - interval '1 minute'
                                     THEN now() ELSE last_seen_at END
             WHERE token_hash = $1 AND expires_at > now()
               AND last_seen_at > now() - make_interval(secs => $2)",
        )
        .bind(&self.token_hash)
        .bind(self.idle.as_secs_f64())
        .execute(&self.db)
        .await;
        match open {
            Ok(done) => self.open = done.rows_affected() == 1,
            Err(error) => tracing::warn!(%error, "a connection's session could not be checked"),
        }
        self.open
    }
}

/// The header of requests the page makes on its own, e.g. to read a state
/// every minute: they do not keep the session alive, or it would never lock
/// (#241).
pub const BACKGROUND: &str = "x-remotehub-background";

/// Keeps a session alive, at most one write per minute and session; a
/// locked one stays locked (#241). Requests do it, and so does input into
/// an open connection (#239).
pub async fn touch(db: &PgPool, token_hash: &[u8], idle: Duration) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE sessions SET last_seen_at = now()
         WHERE token_hash = $1 AND last_seen_at < now() - interval '1 minute'
           AND last_seen_at > now() - make_interval(secs => $2)",
    )
    .bind(token_hash)
    .bind(idle.as_secs_f64())
    .execute(db)
    .await?;
    Ok(())
}

/// The same session read again, e.g. after its groups changed (#108).
pub async fn reload(
    db: &PgPool,
    session: &Session,
    idle: Duration,
) -> Result<Option<Session>, sqlx::Error> {
    by_hash(db, &session.token_hash, idle).await
}

async fn by_hash(
    db: &PgPool,
    token_hash: &[u8],
    idle: Duration,
) -> Result<Option<Session>, sqlx::Error> {
    sqlx::query_as(
        "SELECT s.token_hash, s.id, s.client, s.user_id, s.groups,
                u.username, u.display_name, u.kind, u.sid, u.upn,
                u.identity_id, own.memberships,
                s.last_seen_at <= now() - make_interval(secs => $2) AS locked,
                coalesce(s.confirmed_at > now() - make_interval(secs => $3), false) AS confirmed,
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
           -- A browser's session locks after the idle time (#241); the
           -- extension's ends.
           AND (s.client = 'web' OR s.last_seen_at > now() - make_interval(secs => $2))",
    )
    .bind(token_hash)
    .bind(idle.as_secs_f64())
    .bind(CONFIRMED_FOR.as_secs_f64())
    .fetch_optional(db)
    .await
}

pub async fn delete<'e>(db: impl PgExecutor<'e>, token: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(hash(token).as_slice())
        .execute(db)
        .await?;
    Ok(())
}

/// Removes sessions that can no longer be used, and the extension's codes
/// that ran out unused. A locked browser session stays until its maximum
/// lifetime (#241).
pub async fn purge(db: &PgPool, idle: Duration) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM sessions
         WHERE expires_at <= now()
            OR (client <> 'web' AND last_seen_at <= now() - make_interval(secs => $1))",
    )
    .bind(idle.as_secs_f64())
    .execute(db)
    .await?;
    sqlx::query("DELETE FROM extension_codes WHERE expires_at <= now()")
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

/// Handlers that take a [`Session`] only run for users signed in to
/// remotehub's pages; everyone else gets the problem `unauthenticated`. The
/// extension's token opens nothing here, not even when it is sent as the
/// cookie.
impl FromRequestParts<AppState> for Session {
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Problem> {
        let AnySession(session) = AnySession::from_request_parts(parts, state).await?;
        if session.locked {
            return Err(Problem::new(ErrorCode::SessionLocked));
        }
        Ok(session)
    }
}

/// A browser's session, locked or not (#241): for what a locked one may do,
/// confirming and saying whose it is.
pub struct AnySession(pub Session);

impl FromRequestParts<AppState> for AnySession {
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Problem> {
        let token = token(&parts.headers).ok_or(Problem::new(ErrorCode::Unauthenticated))?;
        let keep_alive = !parts.headers.contains_key(BACKGROUND);
        find(&state.db, &token, state.settings.session.idle, keep_alive)
            .await?
            .filter(|session| session.client == "web")
            .map(AnySession)
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
