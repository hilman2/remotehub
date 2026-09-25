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
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::session::Session;

#[derive(Deserialize)]
pub struct Reveal {
    /// `show` or `copy`, for the audit log.
    purpose: String,
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
    purpose: &str,
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
            details: json!({ "purpose": purpose }),
            address: Some(address),
        },
    )
    .await?;
    Ok(())
}

fn purpose(input: &Reveal) -> Result<&str, Problem> {
    match input.purpose.as_str() {
        "show" | "copy" => Ok(&input.purpose),
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
    let (username, domain, version, kind): (String, String, i32, String) =
        sqlx::query_as("SELECT username, domain, version, kind FROM credentials WHERE id = $1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    let revealed = open(&state, id, version, &kind, username, domain).await?;
    record(&state, &session, ("credential", id), purpose, &address).await?;
    Ok(answer(revealed))
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
    let (mode, username, domain, version, kind): (String, String, String, i32, String) =
        sqlx::query_as(
            "SELECT auth_mode, username, domain, secret_version, secret_kind
             FROM devices WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    // Other sign-in modes use nothing of the device's own; version 0 is none.
    if mode != "device" || version == 0 {
        return Err(Problem::new(ErrorCode::NotFound));
    }
    let revealed = open(&state, id, version, &kind, username, domain).await?;
    record(&state, &session, ("device", id), purpose, &address).await?;
    Ok(answer(revealed))
}
