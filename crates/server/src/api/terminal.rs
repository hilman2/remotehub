//! `GET /api/devices/{id}/terminal`: an SSH session in the browser over a
//! WebSocket (ADR 0003).
//!
//! The server is the SSH client; the browser only exchanges terminal bytes.
//!
//! Browser → server:
//! - first text frame: `{"type":"start","cols":…,"rows":…}`, plus
//!   `"username"` and `"password"` when the device asks for credentials
//! - binary frames: keystrokes
//! - text frames `{"type":"resize","cols":…,"rows":…}`
//!
//! Server → browser:
//! - `{"type":"connected","host_key_fingerprint":…,"pinned":bool}`
//! - binary frames: terminal output
//! - `{"type":"closed","exit_status":…}` or
//!   `{"type":"error","code":…,"params":{…}}`, then the socket closes.

use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::header::ORIGIN;
use axum::response::Response;
use remotehub_gateway::ssh::{self, Output, Size, SshAuth, SshError, SshKey, SshTarget};
use remotehub_model::{ObjectId, Role};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::audit::{self, Action, Actor, Entry};
use crate::session::Session;
use crate::{AppState, catalog, secrets};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const START_TIMEOUT: Duration = Duration::from_secs(120);
const PASSWORD_FIELD: &str = "password";
const PRIVATE_KEY_FIELD: &str = "private_key";
const PASSPHRASE_FIELD: &str = "passphrase";
const CERTIFICATE_FIELD: &str = "certificate";

#[derive(sqlx::FromRow)]
struct Target {
    id: Uuid,
    name: String,
    protocol: String,
    host: String,
    port: i32,
    auth_mode: String,
    credential_id: Option<Uuid>,
    host_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Start {
        cols: u32,
        rows: u32,
        username: Option<String>,
        password: Option<SecretString>,
    },
    Resize {
        cols: u32,
        rows: u32,
    },
}

/// Browsers send `Origin` with every WebSocket handshake; without this check
/// any website could open a terminal with the user's cookie (cross-site
/// WebSocket hijacking), because the handshake is a plain GET.
fn same_origin(headers: &HeaderMap, state: &AppState) -> Result<(), Problem> {
    match headers.get(ORIGIN).and_then(|o| o.to_str().ok()) {
        Some(origin) if origin.eq_ignore_ascii_case(&state.settings.public_origin) => Ok(()),
        _ => Err(Problem::new(ErrorCode::ForbiddenOrigin)),
    }
}

pub async fn terminal(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, Problem> {
    same_origin(&headers, &state)?;
    let catalog = catalog::load(&state.db).await?;
    let subject = session.subject(&state.settings);
    match catalog.effective_role(&subject, ObjectId::Device(id)) {
        Some(role) if role >= Role::Connect => {}
        Some(_) => return Err(Problem::new(ErrorCode::Forbidden)),
        None => return Err(Problem::new(ErrorCode::NotFound)),
    }
    let target: Target = sqlx::query_as(
        "SELECT id, name, protocol, host, port, auth_mode, credential_id, host_key FROM devices WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    if target.protocol != "ssh" {
        return Err(Problem::new(ErrorCode::InvalidRequest).param("field", "protocol"));
    }
    // The own account's password needs the key cookie, which only this
    // request carries.
    let own = if target.auth_mode == "own" {
        Some(own_account(&state, &session, &headers).await?)
    } else {
        None
    };
    Ok(upgrade.on_upgrade(move |socket| run(socket, state, session, target, own, address)))
}

async fn send_json(socket: &mut WebSocket, value: Value) {
    let _ = socket.send(Message::Text(value.to_string().into())).await;
}

async fn send_problem(socket: &mut WebSocket, problem: &Problem) {
    send_json(
        socket,
        json!({ "type": "error", "code": problem.code, "params": problem.params }),
    )
    .await;
}

fn entry<'a>(
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

/// The signed-in user's own directory account (ADR 0005): their user name
/// and the sign-in password kept for this session.
async fn own_account(
    state: &AppState,
    session: &Session,
    headers: &HeaderMap,
) -> Result<Result<(String, Login), Problem>, Problem> {
    if !state.settings.own_account_connections || session.kind != "directory" {
        return Ok(Err(Problem::new(ErrorCode::OwnAccountUnavailable)));
    }
    let Some(password) = crate::session::sign_in_password(&state.db, headers).await? else {
        return Ok(Err(Problem::new(ErrorCode::OwnAccountUnavailable)));
    };
    let password =
        String::from_utf8(password.to_vec()).map_err(|_| Problem::new(ErrorCode::Internal))?;
    Ok(Ok((
        session.username.clone(),
        Login::Password(SecretString::from(password)),
    )))
}

async fn run(
    mut socket: WebSocket,
    state: AppState,
    session: Session,
    target: Target,
    own: Option<Result<(String, Login), Problem>>,
    address: String,
) {
    // 1. The browser says how big its terminal is (and, if asked, who to be).
    let start = match tokio::time::timeout(START_TIMEOUT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<ClientMessage>(&text).ok(),
        _ => None,
    };
    let Some(ClientMessage::Start {
        cols,
        rows,
        username,
        password,
    }) = start
    else {
        send_problem(&mut socket, &Problem::new(ErrorCode::InvalidRequest)).await;
        return;
    };
    let size = Size {
        cols: cols.clamp(10, 1000),
        rows: rows.clamp(2, 500),
    };

    // 2. Credentials: from the vault, or as entered for this connection only.
    let resolved = match own {
        Some(own) => own,
        None => credentials(&state, &target, username, password).await,
    };
    let (username, login) = match resolved {
        Ok(credentials) => credentials,
        Err(problem) => {
            send_problem(&mut socket, &problem).await;
            return;
        }
    };

    // 3. Connect on the server.
    let port = u16::try_from(target.port).unwrap_or(22);
    let opened = ssh::open(
        SshTarget {
            host: &target.host,
            port,
            username: &username,
            auth: match &login {
                Login::Password(password) => SshAuth::Password(password),
                Login::Key(key) => SshAuth::Key(key),
            },
            pinned_host_key: target.host_key.as_deref(),
        },
        size,
        CONNECT_TIMEOUT,
    )
    .await;
    drop(login);
    let mut shell = match opened {
        Ok(shell) => shell,
        Err(error) => {
            let problem = problem_for(&error);
            tracing::warn!(device = %target.id, %error, "SSH connection failed");
            let _ = audit::record(
                &state.db,
                entry(
                    &session,
                    Action::ConnectionFailed,
                    target.id,
                    json!({ "protocol": "ssh", "reason": problem.code, "host": target.host }),
                    &address,
                ),
            )
            .await;
            send_problem(&mut socket, &problem).await;
            return;
        }
    };

    // 4. Pin the host key at the first connection.
    let pinned_now = target.host_key.is_none()
        && pin_host_key(&state, &session, &target, &shell.host_key, &address).await;
    let _ = audit::record(
        &state.db,
        entry(
            &session,
            Action::ConnectionOpened,
            target.id,
            json!({
                "protocol": "ssh", "host": target.host, "port": port, "username": username,
                "auth_mode": target.auth_mode, "credential_id": target.credential_id,
            }),
            &address,
        ),
    )
    .await;
    tracing::info!(device = %target.id, name = %target.name, user = %session.username, "SSH session opened");
    send_json(
        &mut socket,
        json!({
            "type": "connected",
            "host_key_fingerprint": shell.host_key_fingerprint,
            "pinned": pinned_now,
        }),
    )
    .await;

    // 5. Relay until either side ends.
    let started = Instant::now();
    let (mut sent, mut received) = (0usize, 0usize);
    let mut exit_status = None;
    loop {
        tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Binary(data))) => {
                    sent += data.len();
                    if shell.send(&data).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Text(text))) => {
                    if let Ok(ClientMessage::Resize { cols, rows }) = serde_json::from_str(&text) {
                        let _ = shell.resize(Size { cols: cols.clamp(10, 1000), rows: rows.clamp(2, 500) }).await;
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            output = shell.next() => match output {
                Some(Output::Data(data)) => {
                    received += data.len();
                    if socket.send(Message::Binary(data.into())).await.is_err() {
                        break;
                    }
                }
                Some(Output::Exit(status)) => {
                    exit_status = status;
                    send_json(&mut socket, json!({ "type": "closed", "exit_status": status })).await;
                    break;
                }
                None => {
                    send_json(&mut socket, json!({ "type": "closed", "exit_status": null })).await;
                    break;
                }
            },
        }
    }
    shell.close().await;
    let _ = socket.send(Message::Close(None)).await;

    let _ = audit::record(
        &state.db,
        entry(
            &session,
            Action::ConnectionClosed,
            target.id,
            json!({
                "protocol": "ssh", "seconds": started.elapsed().as_secs(),
                "bytes_sent": sent, "bytes_received": received, "exit_status": exit_status,
            }),
            &address,
        ),
    )
    .await;
}

/// How to sign in to the target, resolved on the server.
enum Login {
    Password(SecretString),
    Key(Box<SshKey>),
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

/// User name and login for the device's sign-in mode.
async fn credentials(
    state: &AppState,
    target: &Target,
    username: Option<String>,
    password: Option<SecretString>,
) -> Result<(String, Login), Problem> {
    match target.auth_mode.as_str() {
        "stored" => {
            let credential = target
                .credential_id
                .ok_or(Problem::new(ErrorCode::InvalidRequest).param("field", "credential_id"))?;
            let (username, version, kind): (String, i32, String) =
                sqlx::query_as("SELECT username, version, kind FROM credentials WHERE id = $1")
                    .bind(credential)
                    .fetch_one(&state.db)
                    .await?;
            if kind == "ssh_key" {
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
                return Ok((username, Login::Key(Box::new(key))));
            }
            let password = stored_text(state, credential, version, PASSWORD_FIELD)
                .await?
                .ok_or(Problem::new(ErrorCode::Internal))?;
            Ok((username, Login::Password(password)))
        }
        "ask" => {
            let username = username
                .filter(|u| !u.trim().is_empty())
                .ok_or(Problem::new(ErrorCode::InvalidRequest).param("field", "username"))?;
            let password = password
                .ok_or(Problem::new(ErrorCode::InvalidRequest).param("field", "password"))?;
            Ok((username.trim().to_owned(), Login::Password(password)))
        }
        _ => Err(Problem::new(ErrorCode::InvalidRequest).param("field", "auth_mode")),
    }
}

async fn pin_host_key(
    state: &AppState,
    session: &Session,
    target: &Target,
    key: &str,
    address: &str,
) -> bool {
    let pinned = sqlx::query(
        "UPDATE devices SET host_key = $2, host_key_pinned_at = now() WHERE id = $1 AND host_key IS NULL",
    )
    .bind(target.id)
    .bind(key)
    .execute(&state.db)
    .await
    .is_ok_and(|r| r.rows_affected() == 1);
    if pinned {
        let _ = audit::record(
            &state.db,
            entry(
                session,
                Action::HostKeyPinned,
                target.id,
                json!({ "fingerprint": ssh::fingerprint(key) }),
                address,
            ),
        )
        .await;
    }
    pinned
}

fn problem_for(error: &SshError) -> Problem {
    match error {
        SshError::HostKeyChanged {
            expected,
            presented,
        } => Problem::new(ErrorCode::HostKeyChanged)
            .param("expected", expected.as_str())
            .param("presented", presented.as_str()),
        SshError::AuthenticationFailed => Problem::new(ErrorCode::TargetAuthFailed),
        SshError::Unreachable(_) | SshError::Timeout => Problem::new(ErrorCode::TargetUnreachable),
        SshError::Protocol(_) => Problem::new(ErrorCode::ConnectionFailed),
    }
}
