//! Showing and copying stored credentials (#97): only with `reveal`, and
//! every time audited before the value leaves the server.
//!
//! - `POST /api/credentials/{id}/reveal`: a shared credential.
//! - `POST /api/devices/{id}/reveal`: a device's own credentials (sign-in
//!   mode `device`).
//!
//! POST, so no cache and no link keeps the answer; it says `no-store` too.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::IntoResponse;
use remotehub_model::{ObjectId, Role};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::catalog::{body, context, invalid, require};
use super::connect::stored_text;
use super::fields::{Field, secret_name};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::session::Session;

#[derive(Deserialize)]
pub struct Reveal {
    /// `show`, `copy` or `export`, for the audit log.
    purpose: String,
    /// An older version of a credential (#100); the current one without.
    #[serde(default)]
    version: Option<i32>,
}

/// A version of a credential's secrets, for its history.
#[derive(Serialize, sqlx::FromRow)]
pub struct Version {
    version: i32,
    /// RFC 3339, UTC.
    created_at: String,
}

/// The stored values; the fields of the other kind are absent.
#[derive(Serialize)]
pub struct Revealed {
    username: String,
    domain: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    private_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    passphrase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    certificate: Option<String>,
    /// The protected custom fields of a credential (#98), in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fields: Vec<RevealedField>,
}

#[derive(Serialize)]
pub struct RevealedField {
    name: String,
    value: String,
}

/// What `owner`'s sealed `version` holds.
async fn open(
    state: &AppState,
    owner: Uuid,
    version: i32,
    kind: &str,
    username: String,
    domain: String,
) -> Result<Revealed, Problem> {
    let text = |field: &'static str| async move {
        Ok::<_, Problem>(
            stored_text(state, owner, version, field)
                .await?
                .map(|value: SecretString| value.expose_secret().to_owned()),
        )
    };
    let mut revealed = Revealed {
        username,
        domain,
        password: None,
        private_key: None,
        passphrase: None,
        certificate: None,
        fields: Vec::new(),
    };
    if kind == "ssh_key" {
        revealed.private_key = text("private_key").await?;
        revealed.passphrase = text("passphrase").await?;
        revealed.certificate = text("certificate").await?;
    } else {
        revealed.password = text("password").await?;
    }
    Ok(revealed)
}

async fn record(
    state: &AppState,
    session: &Session,
    object: (&str, Uuid),
    details: serde_json::Value,
    address: &str,
) -> Result<(), Problem> {
    audit::record(
        &state.db,
        Entry {
            actor: Actor {
                id: Some(session.user_id),
                name: &session.username,
            },
            action: Action::CredentialRevealed,
            object: Some(object),
            details,
            address: Some(address),
        },
    )
    .await?;
    Ok(())
}

fn purpose(input: &Reveal) -> Result<&str, Problem> {
    match input.purpose.as_str() {
        // `export`: into a KeePass file, in the browser (#99).
        "show" | "copy" | "export" => Ok(&input.purpose),
        _ => Err(invalid("purpose")),
    }
}

fn answer(revealed: Revealed) -> impl IntoResponse {
    ([(header::CACHE_CONTROL, "no-store")], Json(revealed))
}

pub async fn credential(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    input: Result<Json<Reveal>, JsonRejection>,
) -> Result<impl IntoResponse, Problem> {
    let input = body(input)?;
    let purpose = purpose(&input)?;
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Reveal, ObjectId::Credential(id))?;
    type Row = (String, String, i32, String, sqlx::types::Json<Vec<Field>>);
    let (username, domain, current, kind, fields): Row = sqlx::query_as(
        "SELECT username, domain, version, kind, fields FROM credentials WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    let version = input.version.unwrap_or(current);
    if !(1..=current).contains(&version) {
        return Err(invalid("version"));
    }
    // The current version's protected fields in their order; an older one's
    // as they were sealed then.
    let names: Vec<String> = if version == current {
        fields
            .0
            .into_iter()
            .filter(|f| f.protected)
            .map(|f| f.name)
            .collect()
    } else {
        sqlx::query_scalar(
            "SELECT substr(field, 7) FROM secret_fields
             WHERE owner_id = $1 AND version = $2 AND field LIKE 'field:%' ORDER BY field",
        )
        .bind(id)
        .bind(version)
        .fetch_all(&state.db)
        .await?
    };
    let mut revealed = open(&state, id, version, &kind, username, domain).await?;
    for name in names {
        let value = stored_text(&state, id, version, &secret_name(&name)).await?;
        revealed.fields.push(RevealedField {
            name,
            value: value
                .map(|v| v.expose_secret().to_owned())
                .unwrap_or_default(),
        });
    }
    let details = json!({ "purpose": purpose, "version": version });
    record(&state, &session, ("credential", id), details, &address).await?;
    Ok(answer(revealed))
}

/// The versions of a credential's secrets, newest first; with `reveal`,
/// since they lead to older passwords.
pub async fn versions(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<Version>>, Problem> {
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Reveal, ObjectId::Credential(id))?;
    let versions = sqlx::query_as(
        r#"SELECT version,
                  to_char(min(created_at) AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"')
                      AS created_at
           FROM secret_fields WHERE owner_id = $1
           GROUP BY version ORDER BY version DESC"#,
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(versions))
}

pub async fn device(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    input: Result<Json<Reveal>, JsonRejection>,
) -> Result<impl IntoResponse, Problem> {
    let input = body(input)?;
    let purpose = purpose(&input)?;
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Reveal, ObjectId::Device(id))?;
    let (username, domain, version, kind): (String, String, i32, String) = sqlx::query_as(
        "SELECT username, domain, secret_version, secret_kind FROM devices WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    // Version 0: the device keeps nothing of its own, as in every sign-in
    // mode but `device`.
    if version == 0 {
        return Err(Problem::new(ErrorCode::NotFound));
    }
    let revealed = open(&state, id, version, &kind, username, domain).await?;
    let details = json!({ "purpose": purpose });
    record(&state, &session, ("device", id), details, &address).await?;
    Ok(answer(revealed))
}
