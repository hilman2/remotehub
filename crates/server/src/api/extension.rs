//! The browser extension (#201, ADR 0017): it fills the vault's logins into
//! web pages, on request only, and only into pages they belong to
//! (`crate::site`).
//!
//! Connecting runs through the browser signed in to remotehub, as OAuth 2.0
//! with PKCE (RFC 7636). The extension opens the page `/extension/connect`
//! with `chrome.identity.launchWebAuthFlow`; after the user confirms, the page
//! asks for a code and hands it to the extension through the redirect to
//! `https://<extension-id>.chromiumapp.org/`, which only that extension sees.
//!
//! - `POST /api/extension-codes` (browser session): a code, once and for a
//!   minute, for an extension remotehub knows.
//! - `POST /api/extension/token`: the code and the verifier behind its
//!   challenge, traded for a session of the extension's own.
//! - `GET /api/account/extensions`, `DELETE /api/account/extensions/{id}`
//!   (browser session): the user's extensions, and ending one.
//!
//! With that session's token as `Authorization: Bearer`:
//!
//! - `GET /api/extension/entries`: the shared logins the user may reveal and
//!   that name a URL, without their secrets.
//! - `POST /api/extension/entries/{id}/fill`: user name and password for the
//!   page at the origin given, if it belongs to the login.
//! - `POST /api/extension/entries/{id}/copy`: the password, for the clipboard.
//! - `POST /api/extension/entries/{id}/code`: the one-time code.
//! - `GET /api/extension/personal`: the personal vault, sealed as ever.
//! - `DELETE /api/extension/session`: the extension signs out.
//!
//! Handing out a password or a code takes `reveal` and is audited, as in the
//! vault, with the purpose `fill` or `copy`. Every request names the
//! extension's version in `X-Remotehub-Extension`; older ones than
//! [`OLDEST_EXTENSION`] get `extension_outdated`.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use remotehub_model::{ObjectId, Role};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::catalog::{PASSWORD_FIELD, body, context, invalid, require};
use super::connect::stored_text;
use super::personal::{self, Vault};
use super::problem::{ErrorCode, Problem};
use super::reveal::{Code, current_code};
use super::session::ClientAddress;
use crate::audit::{self, Action, Actor, Entry};
use crate::session::{self, Session};
use crate::{AppState, site};

/// The oldest extension this server still answers. Raise it when the routes
/// under `/api/extension/` change in a way an older extension cannot follow;
/// the extension then asks for its update instead of failing on its own.
pub const OLDEST_EXTENSION: [u32; 3] = [0, 2, 2];

/// The header in which the extension names its version.
const VERSION_HEADER: &str = "x-remotehub-extension";

/// How long a code waits for the extension to trade it.
const CODE_SECONDS: f64 = 60.0;

/// Whether the request comes from an extension this server still answers.
fn check_version(headers: &HeaderMap) -> Result<(), Problem> {
    let version: Option<Vec<u32>> = headers
        .get(VERSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|text| text.split('.').map(|part| part.parse().ok()).collect());
    match version {
        Some(version) if version.as_slice() >= OLDEST_EXTENSION.as_slice() => Ok(()),
        _ => Err(Problem::new(ErrorCode::ExtensionOutdated)
            .param("oldest", OLDEST_EXTENSION.map(|n| n.to_string()).join("."))),
    }
}

/// A session of the browser extension: handlers that take one only run for
/// a valid bearer token of the client `extension`. A browser session's
/// token opens nothing here, and cookies are not read at all.
pub struct ExtensionSession(pub Session);

impl FromRequestParts<AppState> for ExtensionSession {
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Problem> {
        check_version(&parts.headers)?;
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .filter(|token| !token.is_empty())
            .ok_or(Problem::new(ErrorCode::Unauthenticated))?;
        session::lookup(&state.db, token, state.settings.session.idle)
            .await?
            .filter(|session| session.client == "extension")
            .map(ExtensionSession)
            .ok_or(Problem::new(ErrorCode::Unauthenticated))
    }
}

fn record<'a>(
    session: &'a Session,
    action: Action,
    object: (&'a str, Uuid),
    details: serde_json::Value,
    address: &'a str,
) -> Entry<'a> {
    Entry {
        actor: Actor {
            id: Some(session.user_id),
            name: &session.username,
        },
        action,
        object: Some(object),
        details,
        address: Some(address),
    }
}

/// Secrets leave with `no-store`, so no cache keeps them.
fn secret<T: Serialize>(value: T) -> impl IntoResponse {
    ([(header::CACHE_CONTROL, "no-store")], Json(value))
}

// ── Connecting ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct NewCode {
    extension_id: String,
    /// BASE64URL(SHA-256(verifier)), the method `S256` of RFC 7636.
    challenge: String,
    /// The browser, as the connect page describes it, e.g. `Edge on Windows`.
    name: String,
}

#[derive(Serialize)]
pub struct IssuedCode {
    code: String,
}

/// A browser name for *My account*: trimmed, at most 100 characters, no
/// control characters.
fn browser_name(value: &str) -> Result<String, Problem> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 100 || value.chars().any(char::is_control) {
        return Err(invalid("name"));
    }
    Ok(value.to_owned())
}

/// `POST /api/extension-codes`: a code for the extension the user just
/// confirmed on the connect page. Only an extension remotehub serves or
/// names in `REMOTEHUB_EXTENSION_IDS` gets one, since the page sends the code
/// to `https://<extension-id>.chromiumapp.org/`.
pub async fn create_code(
    State(state): State<AppState>,
    session: Session,
    input: Result<Json<NewCode>, JsonRejection>,
) -> Result<impl IntoResponse, Problem> {
    let input = body(input)?;
    if !state.settings.allows_extension(&input.extension_id) {
        return Err(Problem::new(ErrorCode::ExtensionUnknown));
    }
    // SHA-256 in base64url without padding is always 43 characters.
    if input.challenge.len() != 43
        || !input
            .challenge
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(invalid("challenge"));
    }
    let name = browser_name(&input.name)?;
    let code = session::new_code();
    sqlx::query(
        "INSERT INTO extension_codes
             (code_hash, user_id, groups, extension_id, challenge, name, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now() + make_interval(secs => $7))",
    )
    .bind(session::hash(&code).as_slice())
    .bind(session.user_id)
    .bind(&session.groups)
    .bind(&input.extension_id)
    .bind(&input.challenge)
    .bind(&name)
    .bind(CODE_SECONDS)
    .execute(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(IssuedCode { code })))
}

#[derive(Deserialize)]
pub struct Exchange {
    code: String,
    verifier: String,
}

#[derive(Serialize)]
pub struct Connected {
    token: String,
    username: String,
    display_name: String,
    /// The session ends after this long without a request; the extension
    /// locks the personal vault after the same time.
    idle_seconds: u64,
}

/// A verifier as RFC 7636 allows it: 43 to 128 unreserved characters.
fn is_verifier(value: &str) -> bool {
    (43..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// `POST /api/extension/token`: trades a code and its verifier for a
/// session of the extension. The code is used up by the first attempt,
/// right or wrong.
pub async fn token(
    State(state): State<AppState>,
    ClientAddress(address): ClientAddress,
    headers: HeaderMap,
    input: Result<Json<Exchange>, JsonRejection>,
) -> Result<impl IntoResponse, Problem> {
    check_version(&headers)?;
    let input = body(input)?;
    if !is_verifier(&input.verifier) {
        return Err(invalid("verifier"));
    }
    let mut tx = state.db.begin().await?;
    type Row = (Uuid, Vec<String>, String, String, String, bool);
    let row: Option<Row> = sqlx::query_as(
        "DELETE FROM extension_codes WHERE code_hash = $1
         RETURNING user_id, groups, extension_id, challenge, name, expires_at > now()",
    )
    .bind(session::hash(&input.code).as_slice())
    .fetch_optional(&mut *tx)
    .await?;
    let Some((user_id, groups, extension_id, challenge, name, fresh)) = row else {
        return Err(invalid("code"));
    };
    // The deletion stands even when the exchange fails: a code is tried once.
    let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(input.verifier.as_bytes()));
    let answer = if !fresh {
        Err(invalid("code"))
    } else if expected != challenge {
        Err(invalid("verifier"))
    } else if session::blocked(&mut *tx, user_id).await? {
        Err(Problem::new(ErrorCode::Unauthenticated))
    } else {
        Ok(())
    };
    if let Err(problem) = answer {
        tx.commit().await?;
        return Err(problem);
    }
    let token = session::create_extension(
        &mut *tx,
        user_id,
        &groups,
        state.settings.session.max,
        &name,
    )
    .await?;
    let (username, display_name): (String, String) =
        sqlx::query_as("SELECT username, display_name FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
    audit::record(
        &mut *tx,
        Entry {
            actor: Actor {
                id: Some(user_id),
                name: &username,
            },
            action: Action::ExtensionConnected,
            object: Some(("user", user_id)),
            details: json!({ "name": name, "extension_id": extension_id }),
            address: Some(&address),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(secret(Connected {
        token,
        username,
        display_name,
        idle_seconds: state.settings.session.idle.as_secs(),
    }))
}

/// `DELETE /api/extension/session`: the extension signs out.
pub async fn sign_out(
    State(state): State<AppState>,
    ExtensionSession(session): ExtensionSession,
    ClientAddress(address): ClientAddress,
) -> Result<StatusCode, Problem> {
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(&session.token_hash)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        record(
            &session,
            Action::ExtensionDisconnected,
            ("user", session.user_id),
            json!({ "by": "extension" }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Connection {
    id: Uuid,
    name: String,
    /// RFC 3339, UTC.
    created_at: String,
    /// RFC 3339, UTC; to the minute (`session::lookup`).
    last_seen_at: String,
}

/// `GET /api/account/extensions`: the user's connected extensions.
pub async fn list(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<Connection>>, Problem> {
    let connections = sqlx::query_as(
        r#"SELECT id, name,
                  to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') AS created_at,
                  to_char(last_seen_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') AS last_seen_at
           FROM sessions
           WHERE user_id = $1 AND client = 'extension' AND expires_at > now()
             AND last_seen_at > now() - make_interval(secs => $2)
           ORDER BY created_at"#,
    )
    .bind(session.user_id)
    .bind(state.settings.session.idle.as_secs_f64())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(connections))
}

/// `DELETE /api/account/extensions/{id}`: ends one of the user's
/// extensions; the next request of that extension is `unauthenticated`.
pub async fn end(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    let mut tx = state.db.begin().await?;
    let name: Option<String> = sqlx::query_scalar(
        "DELETE FROM sessions WHERE id = $1 AND user_id = $2 AND client = 'extension'
         RETURNING name",
    )
    .bind(id)
    .bind(session.user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(name) = name else {
        return Err(Problem::new(ErrorCode::NotFound));
    };
    audit::record(
        &mut *tx,
        record(
            &session,
            Action::ExtensionDisconnected,
            ("user", session.user_id),
            json!({ "by": "account", "name": name }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Logins ──────────────────────────────────────────────────────────────────

#[derive(Serialize, sqlx::FromRow)]
pub struct Login {
    id: Uuid,
    name: String,
    username: String,
    url: String,
    /// One of KeePass' standard icons.
    icon: i16,
    has_totp: bool,
}

/// `GET /api/extension/entries`: the shared logins the extension may offer.
/// It matches pages against them itself, so remotehub does not learn which
/// pages are open.
pub async fn entries(
    State(state): State<AppState>,
    ExtensionSession(session): ExtensionSession,
) -> Result<impl IntoResponse, Problem> {
    let (subject, catalog) = context(&state, &session).await?;
    let logins: Vec<Login> = sqlx::query_as(
        "SELECT id, name, username, url, icon, has_totp FROM credentials
         WHERE deleted_at IS NULL AND url <> '' ORDER BY lower(name)",
    )
    .fetch_all(&state.db)
    .await?;
    let logins: Vec<Login> = logins
        .into_iter()
        .filter(|login| {
            catalog
                .effective_role(&subject, ObjectId::Credential(login.id))
                .is_some_and(|role| role >= Role::Reveal)
        })
        .collect();
    Ok(secret(logins))
}

/// A login the user may reveal, outside the recycle bin: its user name,
/// version and URL.
async fn revealable(
    state: &AppState,
    session: &Session,
    id: Uuid,
) -> Result<(String, i32, String), Problem> {
    let (subject, catalog) = context(state, session).await?;
    require(&catalog, &subject, Role::Reveal, ObjectId::Credential(id))?;
    sqlx::query_as(
        "SELECT username, version, url FROM credentials WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(Problem::new(ErrorCode::NotFound))
}

/// The page a login is to be filled into, as `location.origin` names it.
#[derive(Deserialize)]
pub struct Page {
    origin: String,
}

fn origin(page: &Page) -> Result<&str, Problem> {
    (page.origin.len() <= 2048)
        .then_some(page.origin.as_str())
        .ok_or_else(|| invalid("origin"))
}

/// `wrong_site` unless the page at `origin` belongs to the login with `url`.
/// The extension never asks for such a page itself, so the log keeps each
/// such request: a fault in the extension, or someone using its token.
fn belongs(session: &Session, id: Uuid, url: &str, origin: &str) -> Result<(), Problem> {
    if site::matches(url, origin) {
        return Ok(());
    }
    tracing::warn!(user = %session.username, credential = %id, %origin, "the extension asked to fill a login into a page it does not belong to");
    Err(Problem::new(ErrorCode::WrongSite))
}

#[derive(Serialize)]
pub struct Filled {
    username: String,
    password: String,
}

/// `POST /api/extension/entries/{id}/fill`: user name and password for the
/// page at `origin`. The extension only asks for pages the login belongs
/// to; remotehub checks that again, so a slip in the extension cannot send
/// a password elsewhere.
pub async fn fill(
    State(state): State<AppState>,
    ExtensionSession(session): ExtensionSession,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    input: Result<Json<Page>, JsonRejection>,
) -> Result<impl IntoResponse, Problem> {
    let input = body(input)?;
    let origin = origin(&input)?;
    let (username, version, url) = revealable(&state, &session, id).await?;
    belongs(&session, id, &url, origin)?;
    let password = stored_text(&state, id, version, PASSWORD_FIELD).await?;
    audit::record(
        &state.db,
        record(
            &session,
            Action::CredentialRevealed,
            ("credential", id),
            json!({ "purpose": "fill", "version": version, "origin": origin, "client": "extension" }),
            &address,
        ),
    )
    .await?;
    Ok(secret(Filled {
        username,
        password: password
            .map(|p| p.expose_secret().to_owned())
            .unwrap_or_default(),
    }))
}

#[derive(Serialize)]
pub struct Copied {
    password: String,
}

/// `POST /api/extension/entries/{id}/copy`: the password, for the
/// clipboard; any login the user may reveal, whatever page is open.
pub async fn copy(
    State(state): State<AppState>,
    ExtensionSession(session): ExtensionSession,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, Problem> {
    let (_, version, _) = revealable(&state, &session, id).await?;
    let password = stored_text(&state, id, version, PASSWORD_FIELD).await?;
    audit::record(
        &state.db,
        record(
            &session,
            Action::CredentialRevealed,
            ("credential", id),
            json!({ "purpose": "copy", "version": version, "client": "extension" }),
            &address,
        ),
    )
    .await?;
    Ok(secret(Copied {
        password: password
            .map(|p| p.expose_secret().to_owned())
            .unwrap_or_default(),
    }))
}

#[derive(Deserialize)]
pub struct CodeRequest {
    /// `fill` into the page at `origin`, or `copy`.
    purpose: String,
    #[serde(default)]
    origin: Option<String>,
}

/// `POST /api/extension/entries/{id}/code`: the current one-time code, to
/// fill into the page at `origin` (which must belong to the login) or to
/// copy.
pub async fn code(
    State(state): State<AppState>,
    ExtensionSession(session): ExtensionSession,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    input: Result<Json<CodeRequest>, JsonRejection>,
) -> Result<impl IntoResponse, Problem> {
    let input = body(input)?;
    let (_, _, url) = revealable(&state, &session, id).await?;
    let details = match (input.purpose.as_str(), input.origin) {
        ("fill", Some(origin)) => {
            let page = Page { origin };
            let origin = self::origin(&page)?;
            belongs(&session, id, &url, origin)?;
            json!({ "purpose": "fill", "origin": origin, "client": "extension" })
        }
        ("fill", None) => return Err(invalid("origin")),
        ("copy", _) => json!({ "purpose": "copy", "client": "extension" }),
        _ => return Err(invalid("purpose")),
    };
    let code: Code = current_code(&state, id).await?;
    audit::record(
        &state.db,
        record(
            &session,
            Action::CredentialCodeShown,
            ("credential", id),
            details,
            &address,
        ),
    )
    .await?;
    Ok(secret(code))
}

/// `GET /api/extension/personal`: the personal vault, which the extension
/// unlocks itself as the browser does; the server can still not read it.
pub async fn personal(
    State(state): State<AppState>,
    ExtensionSession(session): ExtensionSession,
) -> Result<Json<Vault>, Problem> {
    Ok(Json(personal::load(&state, session.user_id).await?))
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn older_extensions_are_asked_to_update() {
        let with = |version: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(VERSION_HEADER, HeaderValue::from_str(version).unwrap());
            check_version(&headers)
        };
        assert!(with(&OLDEST_EXTENSION.map(|n| n.to_string()).join(".")).is_ok());
        assert!(with("99.0.0").is_ok());
        assert!(with("0.2.10").is_ok());
        for old in ["0.2.1", "0.1.99", "0", "", "0.2.x", "latest"] {
            assert_eq!(
                with(old).unwrap_err().code,
                ErrorCode::ExtensionOutdated,
                "{old}"
            );
        }
        assert_eq!(
            check_version(&HeaderMap::new()).unwrap_err().code,
            ErrorCode::ExtensionOutdated
        );
    }

    #[test]
    fn verifiers_are_those_of_rfc_7636() {
        assert!(is_verifier(&"a".repeat(43)));
        assert!(is_verifier(&"A-._~9".repeat(20)));
        assert!(!is_verifier(&"a".repeat(42)));
        assert!(!is_verifier(&"a".repeat(129)));
        assert!(!is_verifier(&format!("{}+", "a".repeat(43))));
    }
}
