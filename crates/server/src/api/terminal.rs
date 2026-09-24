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
use axum::response::Response;
use remotehub_gateway::ssh::{self, Output, Size, SshAuth, SshError, SshTarget};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::connect::{
    self, Credentials, Login, Target, entry, own_account, send_json, send_problem,
};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action};
use crate::session::Session;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const START_TIMEOUT: Duration = Duration::from_secs(120);

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

pub async fn terminal(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, Problem> {
    connect::same_origin(&headers, &state)?;
    let target = connect::target(&state, &session, id, &["ssh"]).await?;
    // The own account's password needs the key cookie, which only this
    // request carries.
    let own = if target.auth_mode == "own" {
        Some(own_account(&state, &session, &headers).await?)
    } else {
        None
    };
    Ok(upgrade.on_upgrade(move |socket| run(socket, state, session, target, own, address)))
}

async fn run(
    mut socket: WebSocket,
    state: AppState,
    session: Session,
    target: Target,
    own: Option<Result<Credentials, Problem>>,
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
        None => connect::credentials(&state, &target, username, password).await,
    };
    let Credentials {
        username, login, ..
    } = match resolved {
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
