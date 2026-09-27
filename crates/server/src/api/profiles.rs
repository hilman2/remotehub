//! Login profiles (#192): a login many devices share, a password or an SSH
//! key, like a domain administrator for a customer's servers. A profile lies
//! in a device folder and has that folder's grants; one at the top level is
//! for administrators only. Devices in its folder and below may sign in with
//! it; its password or key reaches no browser but through `reveal`.
//!
//! - `POST /api/profiles`: create, with `edit` on the folder
//! - `PUT /api/profiles/{id}`: change or move; absent secrets stay
//! - `DELETE /api/profiles/{id}`: only one no device uses
//!
//! The profiles come with `GET /api/tree`; `reveal.rs` shows the secret.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use remotehub_model::{Catalog, ObjectId, Role, Subject};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::catalog::{
    Created, SecretInput, body, context, database, entry, invalid, missing_secret, name,
    new_secrets, out_of_reach, plain, require, seal_version,
};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action};
use crate::session::Session;

#[derive(Deserialize)]
pub struct ProfileInput {
    /// The device folder it lies in; none: the top level.
    folder_id: Option<Uuid>,
    name: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    domain: String,
    /// `password` (the default) or `ssh_key`.
    #[serde(default)]
    secret_kind: Option<String>,
    #[serde(flatten)]
    secrets: SecretInput,
}

/// Where a profile may be created or moved to: a folder one may edit, or the
/// top level for administrators.
fn require_place(
    catalog: &Catalog,
    subject: &Subject,
    folder: Option<Uuid>,
) -> Result<(), Problem> {
    match folder {
        Some(folder) => require(catalog, subject, Role::Edit, ObjectId::Folder(folder)),
        None if catalog.may_create_top_level(subject) => Ok(()),
        None => Err(Problem::new(ErrorCode::Forbidden)),
    }
}

fn secret_kind(input: &ProfileInput) -> Result<&'static str, Problem> {
    match input.secret_kind.as_deref() {
        None | Some("password") => Ok("password"),
        Some("ssh_key") => Ok("ssh_key"),
        _ => Err(invalid("secret_kind")),
    }
}

pub async fn create(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<ProfileInput>, JsonRejection>,
) -> Result<(StatusCode, Json<Created>), Problem> {
    let input = body(input)?;
    let name = name(&input.name, "name")?;
    let username = plain(&input.username, "username")?;
    let domain = plain(&input.domain, "domain")?;
    let kind = secret_kind(&input)?;
    let secrets = new_secrets(kind, &input.secrets.given())?.ok_or_else(|| missing_secret(kind))?;
    let (subject, catalog) = context(&state, &session).await?;
    require_place(&catalog, &subject, input.folder_id)?;

    let (algorithm, fingerprint, has_certificate) = secrets.key_info();
    let mut tx = state.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO login_profiles (folder_id, name, username, domain, secret_kind,
             secret_version, key_algorithm, key_fingerprint, has_certificate)
         VALUES ($1, $2, $3, $4, $5, 1, $6, $7, $8) RETURNING id",
    )
    .bind(input.folder_id)
    .bind(&name)
    .bind(&username)
    .bind(&domain)
    .bind(kind)
    .bind(algorithm)
    .bind(fingerprint)
    .bind(has_certificate)
    .fetch_one(&mut *tx)
    .await
    .map_err(database)?;
    seal_version(&mut tx, &state, id, 1, &secrets).await?;
    let details = json!({
        "name": name, "folder_id": input.folder_id, "username": username, "domain": domain,
        "secret_kind": kind, "key_fingerprint": fingerprint,
    });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::ProfileCreated,
            ObjectId::Profile(id),
            details,
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(Created::new(id))))
}

/// A profile as it is stored, before a change.
#[derive(sqlx::FromRow)]
struct Before {
    secret_kind: String,
    secret_version: i32,
    key_algorithm: Option<String>,
    key_fingerprint: Option<String>,
    has_certificate: bool,
}

pub async fn update(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    input: Result<Json<ProfileInput>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let input = body(input)?;
    let name = name(&input.name, "name")?;
    let username = plain(&input.username, "username")?;
    let domain = plain(&input.domain, "domain")?;
    let kind = secret_kind(&input)?;
    let secrets = new_secrets(kind, &input.secrets.given())?;
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Edit, ObjectId::Profile(id))?;
    if catalog.parent(ObjectId::Profile(id)) != input.folder_id {
        require_place(&catalog, &subject, input.folder_id)?;
    }
    let before: Before = sqlx::query_as(
        "SELECT secret_kind, secret_version, key_algorithm, key_fingerprint, has_certificate
         FROM login_profiles WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    // Another kind needs its own secret: a password is no key.
    if secrets.is_none() && kind != before.secret_kind {
        return Err(missing_secret(kind));
    }
    let version = before.secret_version + i32::from(secrets.is_some());
    let (algorithm, fingerprint, has_certificate) = match &secrets {
        Some(secrets) => {
            let (algorithm, fingerprint, certificate) = secrets.key_info();
            (
                algorithm.map(str::to_owned),
                fingerprint.map(str::to_owned),
                certificate,
            )
        }
        None => (
            before.key_algorithm,
            before.key_fingerprint,
            before.has_certificate,
        ),
    };

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE login_profiles SET folder_id = $2, name = $3, username = $4, domain = $5,
             secret_kind = $6, secret_version = $7, key_algorithm = $8, key_fingerprint = $9,
             has_certificate = $10, updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(input.folder_id)
    .bind(&name)
    .bind(&username)
    .bind(&domain)
    .bind(kind)
    .bind(version)
    .bind(&algorithm)
    .bind(&fingerprint)
    .bind(has_certificate)
    .execute(&mut *tx)
    .await
    .map_err(database)?;
    if let Some(secrets) = &secrets {
        seal_version(&mut tx, &state, id, version, secrets).await?;
        // A profile keeps only its current secret: the devices use nothing
        // else, and nobody asks for an old one.
        sqlx::query("DELETE FROM secret_fields WHERE owner_id = $1 AND version <> $2")
            .bind(id)
            .bind(version)
            .execute(&mut *tx)
            .await?;
    }
    // Keys are SSH's: RDP, VNC and web interfaces take passwords.
    if kind == "ssh_key" {
        let misfits: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM devices WHERE profile_id = $1 AND protocol <> 'ssh'
             ORDER BY lower(name)",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        if !misfits.is_empty() {
            return Err(invalid("secret_kind").param("devices", misfits.join(", ")));
        }
    }
    let stranded = out_of_reach(&mut tx).await?;
    if !stranded.is_empty() {
        return Err(
            Problem::new(ErrorCode::ProfileOutOfReach).param("devices", stranded.join(", "))
        );
    }
    let details = json!({
        "name": name, "folder_id": input.folder_id, "username": username, "domain": domain,
        "secret_kind": kind, "key_fingerprint": fingerprint,
        "secret_changed": secrets.is_some(),
    });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::ProfileUpdated,
            ObjectId::Profile(id),
            details,
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Only a profile no device signs in with goes; the others first get
/// another sign-in, so none falls back to something unexpected.
pub async fn delete(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Edit, ObjectId::Profile(id))?;
    let mut tx = state.db.begin().await?;
    let users: Vec<String> =
        sqlx::query_scalar("SELECT name FROM devices WHERE profile_id = $1 ORDER BY lower(name)")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
    if !users.is_empty() {
        return Err(Problem::new(ErrorCode::ProfileInUse).param("devices", users.join(", ")));
    }
    sqlx::query("DELETE FROM secret_fields WHERE owner_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM login_profiles WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::ProfileDeleted,
            ObjectId::Profile(id),
            json!({}),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
