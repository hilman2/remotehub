//! What every connection shares, whatever the protocol: the device and the
//! caller's right to connect to it, the WebSocket's origin, the credentials
//! resolved on the server, and the audit entries.

use axum::extract::ws::{Message, WebSocket};
use axum::http::HeaderMap;
use axum::http::header::ORIGIN;
use remotehub_gateway::ssh::SshKey;
use remotehub_model::{ObjectId, Role};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use uuid::Uuid;

use super::problem::{ErrorCode, Problem};
use crate::audit::{Action, Actor, Entry};
use crate::session::Session;
use crate::{AppState, catalog, secrets};

const PASSWORD_FIELD: &str = "password";
const PRIVATE_KEY_FIELD: &str = "private_key";
const PASSPHRASE_FIELD: &str = "passphrase";
const CERTIFICATE_FIELD: &str = "certificate";

#[derive(sqlx::FromRow)]
pub struct Target {
    pub id: Uuid,
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub port: i32,
    pub auth_mode: String,
    pub credential_id: Option<Uuid>,
    pub host_key: Option<String>,
    pub keyboard_layout: Option<String>,
    pub certificate_fingerprint: Option<String>,
}

/// Browsers send `Origin` with every WebSocket handshake; without this check
/// any website could open a connection with the user's cookie (cross-site
/// WebSocket hijacking), because the handshake is a plain GET.
pub fn same_origin(headers: &HeaderMap, state: &AppState) -> Result<(), Problem> {
    match headers.get(ORIGIN).and_then(|o| o.to_str().ok()) {
        Some(origin) if origin.eq_ignore_ascii_case(&state.settings.public_origin) => Ok(()),
        _ => Err(Problem::new(ErrorCode::ForbiddenOrigin)),
    }
}

/// The device, if the caller may connect to it and it speaks one of
/// `protocols`. Invisible devices are not found.
pub async fn target(
    state: &AppState,
    session: &Session,
    id: Uuid,
    protocols: &[&str],
) -> Result<Target, Problem> {
    let catalog = catalog::load(&state.db).await?;
    let subject = session.subject(&state.settings);
    match catalog.effective_role(&subject, ObjectId::Device(id)) {
        Some(role) if role >= Role::Connect => {}
        Some(_) => return Err(Problem::new(ErrorCode::Forbidden)),
        None => return Err(Problem::new(ErrorCode::NotFound)),
    }
    let target: Target = sqlx::query_as(
        "SELECT id, name, protocol, host, port, auth_mode, credential_id, host_key, keyboard_layout,
                certificate_fingerprint
         FROM devices WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    if !protocols.contains(&target.protocol.as_str()) {
        return Err(Problem::new(ErrorCode::InvalidRequest).param("field", "protocol"));
    }
    Ok(target)
}

pub async fn send_json(socket: &mut WebSocket, value: Value) {
    let _ = socket.send(Message::Text(value.to_string().into())).await;
}

pub async fn send_problem(socket: &mut WebSocket, problem: &Problem) {
    send_json(
        socket,
        json!({ "type": "error", "code": problem.code, "params": problem.params }),
    )
    .await;
}

pub fn entry<'a>(
    session: &'a Session,
    action: Action,
    device: Uuid,
    details: Value,
    address: &'a str,
) -> Entry<'a> {
    Entry {
        actor: Actor {
            id: Some(session.user_id),
            name: &session.username,
        },
        action,
        object: Some(("device", device)),
        details,
        address: Some(address),
    }
}

/// How to sign in to the target, resolved on the server.
pub enum Login {
    Password(SecretString),
    Key(Box<SshKey>),
}

/// Who to be on the target.
pub struct Credentials {
    pub username: String,
    /// Windows domain, empty if none.
    pub domain: String,
    pub login: Login,
}

/// The signed-in user's own directory account (ADR 0005): their user name
/// and the sign-in password kept for this session. The outer error ends the
/// request, the inner one is reported on the opened socket.
pub async fn own_account(
    state: &AppState,
    session: &Session,
    headers: &HeaderMap,
) -> Result<Result<Credentials, Problem>, Problem> {
    if !state.settings.own_account_connections || session.kind != "directory" {
        return Ok(Err(Problem::new(ErrorCode::OwnAccountUnavailable)));
    }
    let Some(password) = crate::session::sign_in_password(&state.db, headers).await? else {
        return Ok(Err(Problem::new(ErrorCode::OwnAccountUnavailable)));
    };
    let password =
        String::from_utf8(password.to_vec()).map_err(|_| Problem::new(ErrorCode::Internal))?;
    Ok(Ok(Credentials {
        username: session.username.clone(),
        domain: String::new(),
        login: Login::Password(SecretString::from(password)),
    }))
}

/// A sealed field of the credential's current version as text.
async fn stored_text(
    state: &AppState,
    credential: Uuid,
    version: i32,
    field: &str,
) -> Result<Option<SecretString>, Problem> {
    let secret = secrets::load(&state.db, &state.vault, credential, version, field)
        .await
        .map_err(|error| {
            tracing::error!(%error, "cannot open a stored secret");
            Problem::new(ErrorCode::Internal)
        })?;
    secret
        .map(|bytes| String::from_utf8(bytes.to_vec()).map(SecretString::from))
        .transpose()
        .map_err(|_| Problem::new(ErrorCode::Internal))
}

/// Credentials for the device's sign-in mode `stored` or `ask`.
pub async fn credentials(
    state: &AppState,
    target: &Target,
    username: Option<String>,
    password: Option<SecretString>,
) -> Result<Credentials, Problem> {
    match target.auth_mode.as_str() {
        "stored" => {
            let credential = target
                .credential_id
                .ok_or(Problem::new(ErrorCode::InvalidRequest).param("field", "credential_id"))?;
            let (username, domain, version, kind): (String, String, i32, String) = sqlx::query_as(
                "SELECT username, domain, version, kind FROM credentials WHERE id = $1",
            )
            .bind(credential)
            .fetch_one(&state.db)
            .await?;
            let login = if kind == "ssh_key" {
                let private_key = stored_text(state, credential, version, PRIVATE_KEY_FIELD)
                    .await?
                    .ok_or(Problem::new(ErrorCode::Internal))?;
                let passphrase = stored_text(state, credential, version, PASSPHRASE_FIELD).await?;
                let certificate =
                    stored_text(state, credential, version, CERTIFICATE_FIELD).await?;
                let key = SshKey::parse(
                    private_key.expose_secret(),
                    passphrase.as_ref().map(|p| p.expose_secret()),
                    certificate.as_ref().map(|c| c.expose_secret()),
                )
                .map_err(|error| {
                    tracing::error!(%error, "a stored SSH key does not open");
                    Problem::new(ErrorCode::Internal)
                })?;
                Login::Key(Box::new(key))
            } else {
                let password = stored_text(state, credential, version, PASSWORD_FIELD)
                    .await?
                    .ok_or(Problem::new(ErrorCode::Internal))?;
                Login::Password(password)
            };
            Ok(Credentials {
                username,
                domain,
                login,
            })
        }
        "ask" => {
            let username = username
                .filter(|u| !u.trim().is_empty())
                .ok_or(Problem::new(ErrorCode::InvalidRequest).param("field", "username"))?;
            let password = password
                .ok_or(Problem::new(ErrorCode::InvalidRequest).param("field", "password"))?;
            Ok(Credentials {
                username: username.trim().to_owned(),
                domain: String::new(),
                login: Login::Password(password),
            })
        }
        _ => Err(Problem::new(ErrorCode::InvalidRequest).param("field", "auth_mode")),
    }
}
