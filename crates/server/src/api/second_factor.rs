//! The second factor of directory users (#107, `crate::second_factor`).
//!
//! - `GET /api/account/second-factor`: whether the caller has one and must.
//! - `POST /api/account/second-factor/offer`: a new secret to set up.
//! - `PUT`/`DELETE /api/account/second-factor`: set it up with a code of
//!   the app, or remove it with one; removing is refused while a rule asks
//!   for it and no security key stays.
//! - `POST /api/account/security-keys/offer`, `POST
//!   /api/account/security-keys`, `DELETE /api/account/security-keys/{id}`:
//!   security keys and passkeys (#129, `crate::webauthn`), with the same
//!   rule for the last one.
//! - `GET /api/second-factor-principals`, `PUT`/`DELETE
//!   /api/second-factor-principals/{sid}`: who must have one (administrators).
//! - `DELETE /api/users/{id}/second-factor`: an administrator removes a
//!   user's factor, e.g. for a lost phone; with a rule, the user sets up a
//!   new one at the next sign-in.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::catalog::{body, invalid, name};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::principal::PrincipalId;
use crate::second_factor;
use crate::session::Session;
use crate::totp;
use crate::webauthn::{self, Registration, RelyingParty};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

#[derive(Serialize)]
pub struct Status {
    /// Only directory users set one up here.
    available: bool,
    /// An authenticator app or a security key.
    enrolled: bool,
    required: bool,
    /// An authenticator app.
    app: bool,
    /// Security keys and passkeys (#129).
    keys: Vec<SecurityKey>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct SecurityKey {
    id: Uuid,
    name: String,
    /// RFC 3339, UTC.
    created_at: String,
    last_used_at: Option<String>,
}

#[derive(Serialize)]
pub struct KeyOffer {
    challenge_id: Uuid,
    /// `navigator.credentials.create()`'s options, binary fields in base64url.
    options: Value,
}

#[derive(Deserialize)]
pub struct NewKey {
    challenge_id: Uuid,
    name: String,
    credential: Registration,
}

#[derive(Serialize)]
pub struct Offer {
    secret: String,
    uri: String,
}

#[derive(Deserialize)]
pub struct Enrollment {
    secret: String,
    code: String,
}

#[derive(Deserialize)]
pub struct Confirmation {
    code: String,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Rule {
    principal_sid: String,
    principal_kind: String,
    principal_name: String,
}

#[derive(Deserialize)]
pub struct NewRule {
    principal_kind: String,
    principal_name: String,
}

fn directory_user(session: &Session) -> Result<(), Problem> {
    if session.kind == "directory" {
        Ok(())
    } else {
        Err(invalid("kind"))
    }
}

fn require_admin(session: &Session) -> Result<(), Problem> {
    if session.is_admin() {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::Forbidden))
    }
}

fn entry<'a>(
    session: &'a Session,
    action: Action,
    object: Option<(&'a str, Uuid)>,
    details: Value,
    address: &'a str,
) -> Entry<'a> {
    Entry {
        actor: Actor {
            id: Some(session.user_id),
            name: &session.username,
        },
        action,
        object,
        details,
        address: Some(address),
    }
}

pub async fn status(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Status>, Problem> {
    let keys = sqlx::query_as(
        r#"SELECT id, name,
                  to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') AS created_at,
                  to_char(last_used_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') AS last_used_at
           FROM security_keys WHERE user_id = $1 ORDER BY created_at"#,
    )
    .bind(session.user_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(Status {
        available: session.kind == "directory",
        enrolled: second_factor::enrolled(&state.db, session.user_id).await?,
        required: second_factor::required(&state.db, &session.sids()).await?,
        app: second_factor::has_app(&state.db, session.user_id).await?,
        keys,
    }))
}

/// A challenge to register a security key with, and the browser's options.
pub async fn offer_key(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<KeyOffer>, Problem> {
    directory_user(&session)?;
    let rp = RelyingParty::of(&state.settings.public_origin);
    let existing = second_factor::key_ids(&state.db, session.user_id).await?;
    let (challenge_id, challenge) =
        second_factor::challenge(&state.db, session.user_id, "register").await?;
    let b64 = |bytes: &[u8]| URL_SAFE_NO_PAD.encode(bytes);
    let options = json!({ "publicKey": {
        "rp": { "name": "remotehub", "id": rp.id },
        "user": {
            "id": b64(session.user_id.as_bytes()),
            "name": session.username,
            "displayName": session.display_name,
        },
        "challenge": b64(&challenge),
        // ES256 only: crate::webauthn verifies nothing else.
        "pubKeyCredParams": [{ "type": "public-key", "alg": webauthn::ES256 }],
        "timeout": 120_000,
        "attestation": "none",
        "excludeCredentials": existing.iter()
            .map(|id| json!({ "type": "public-key", "id": id }))
            .collect::<Vec<_>>(),
        "authenticatorSelection": { "userVerification": "discouraged" },
    }});
    Ok(Json(KeyOffer {
        challenge_id,
        options,
    }))
}

pub async fn add_key(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<NewKey>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    directory_user(&session)?;
    let input = body(input)?;
    let key_name = input.name.trim();
    if key_name.is_empty()
        || key_name.chars().count() > 100
        || key_name.chars().any(char::is_control)
    {
        return Err(invalid("name"));
    }
    let rp = RelyingParty::of(&state.settings.public_origin);
    let mut tx = state.db.begin().await?;
    let user = session.user_id;
    let challenge = second_factor::take_challenge(&mut *tx, user, "register", input.challenge_id)
        .await?
        .ok_or(Problem::new(ErrorCode::SecondFactorInvalid))?;
    let key = webauthn::register(&rp, &challenge, &input.credential).map_err(|error| {
        tracing::warn!(%error, %user, "a security key's registration was refused");
        Problem::new(ErrorCode::SecondFactorInvalid)
    })?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO security_keys (user_id, credential_id, public_key, sign_count, name)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(user)
    .bind(&key.credential_id)
    .bind(&key.public_key)
    .bind(i64::from(key.counter))
    .bind(key_name)
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| match error.as_database_error() {
        // The same key twice: the browser should have refused it.
        Some(db) if db.is_unique_violation() => invalid("credential"),
        _ => error.into(),
    })?;
    let details = json!({ "kind": "security_key", "key_id": id, "name": key_name });
    let recorded = entry(
        &session,
        Action::SecondFactorEnrolled,
        Some(("user", user)),
        details,
        &address,
    );
    audit::record(&mut *tx, recorded).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes a security key, but not the last factor while a rule asks for
/// one.
pub async fn remove_key(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    directory_user(&session)?;
    let mut tx = state.db.begin().await?;
    let user = session.user_id;
    let name: String = sqlx::query_scalar(
        "DELETE FROM security_keys WHERE id = $1 AND user_id = $2 RETURNING name",
    )
    .bind(id)
    .bind(user)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Problem::new(ErrorCode::NotFound))?;
    if !second_factor::enrolled(&mut *tx, user).await?
        && second_factor::required(&mut *tx, &session.sids()).await?
    {
        return Err(Problem::new(ErrorCode::Forbidden));
    }
    let details = json!({ "kind": "security_key", "key_id": id, "name": name });
    let recorded = entry(
        &session,
        Action::SecondFactorRemoved,
        Some(("user", user)),
        details,
        &address,
    );
    audit::record(&mut *tx, recorded).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn offer(session: Session) -> Result<Json<Offer>, Problem> {
    directory_user(&session)?;
    let (secret, uri) = second_factor::offer(&session.username);
    Ok(Json(Offer {
        secret: secret.as_str().to_owned(),
        uri: uri.as_str().to_owned(),
    }))
}

pub async fn enroll(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<Enrollment>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    directory_user(&session)?;
    let input = body(input)?;
    let mut tx = state.db.begin().await?;
    let user = session.user_id;
    let now = totp::unix_now();
    if !second_factor::enroll(&mut tx, &state.vault, user, &input.secret, &input.code, now).await? {
        return Err(Problem::new(ErrorCode::SecondFactorInvalid));
    }
    let object = Some(("user", user));
    let recorded = entry(
        &session,
        Action::SecondFactorEnrolled,
        object,
        json!({}),
        &address,
    );
    audit::record(&mut *tx, recorded).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<Confirmation>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    directory_user(&session)?;
    let input = body(input)?;
    // With a rule, the app goes only while a security key stays.
    if second_factor::required(&state.db, &session.sids()).await?
        && second_factor::key_ids(&state.db, session.user_id)
            .await?
            .is_empty()
    {
        return Err(Problem::new(ErrorCode::Forbidden));
    }
    let mut tx = state.db.begin().await?;
    let user = session.user_id;
    let now = totp::unix_now();
    if !second_factor::verify(&mut tx, &state.vault, user, &input.code, now).await? {
        return Err(Problem::new(ErrorCode::SecondFactorInvalid));
    }
    second_factor::remove(&mut *tx, user).await?;
    let object = Some(("user", user));
    let recorded = entry(
        &session,
        Action::SecondFactorRemoved,
        object,
        json!({}),
        &address,
    );
    audit::record(&mut *tx, recorded).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn reset(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    require_admin(&session)?;
    let mut tx = state.db.begin().await?;
    let username: Option<String> =
        sqlx::query_scalar("SELECT username FROM users WHERE id = $1 AND kind = 'directory'")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let username = username.ok_or(Problem::new(ErrorCode::NotFound))?;
    if !second_factor::remove_all(&mut tx, id).await? {
        return Err(Problem::new(ErrorCode::NotFound));
    }
    let details = json!({ "username": username });
    let recorded = entry(
        &session,
        Action::SecondFactorReset,
        Some(("user", id)),
        details,
        &address,
    );
    audit::record(&mut *tx, recorded).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn rules(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<Rule>>, Problem> {
    require_admin(&session)?;
    let rules = sqlx::query_as(
        "SELECT principal_sid, principal_kind, principal_name FROM second_factor_principals
         ORDER BY lower(principal_name)",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rules))
}

pub async fn require(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(sid): Path<String>,
    input: Result<Json<NewRule>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    require_admin(&session)?;
    let input = body(input)?;
    let sid: PrincipalId = sid.parse().map_err(|_| invalid("principal_sid"))?;
    if !sid.check(&state.db, &input.principal_kind).await? {
        return Err(invalid("principal_sid"));
    }
    let principal_name = name(&input.principal_name, "principal_name")?;
    let mut tx = state.db.begin().await?;
    let added = sqlx::query(
        "INSERT INTO second_factor_principals (principal_sid, principal_kind, principal_name, created_by)
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(sid.to_string())
    .bind(&input.principal_kind)
    .bind(&principal_name)
    .bind(session.user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if added == 1 {
        let details = json!({
            "principal_kind": input.principal_kind, "principal_sid": sid.to_string(),
            "principal_name": principal_name,
        });
        let recorded = entry(
            &session,
            Action::SecondFactorRequired,
            None,
            details,
            &address,
        );
        audit::record(&mut *tx, recorded).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn waive(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(sid): Path<String>,
) -> Result<StatusCode, Problem> {
    require_admin(&session)?;
    let mut tx = state.db.begin().await?;
    let removed: Option<Rule> = sqlx::query_as(
        "DELETE FROM second_factor_principals WHERE principal_sid = $1
         RETURNING principal_sid, principal_kind, principal_name",
    )
    .bind(&sid)
    .fetch_optional(&mut *tx)
    .await?;
    let removed = removed.ok_or(Problem::new(ErrorCode::NotFound))?;
    let details = json!({
        "principal_kind": removed.principal_kind, "principal_sid": removed.principal_sid,
        "principal_name": removed.principal_name,
    });
    let recorded = entry(
        &session,
        Action::SecondFactorWaived,
        None,
        details,
        &address,
    );
    audit::record(&mut *tx, recorded).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
