//! `GET /api/devices/{id}/display`: an RDP or VNC session in the browser over
//! a WebSocket (ADR 0003), drawn by the Guacamole client.
//!
//! The server opens the connection through guacd with the credentials
//! resolved here; the browser only exchanges Guacamole instructions for
//! input and drawing, and never sees the connection's parameters.
//!
//! Browser → server:
//! - first text frame: `{"type":"start","width":…,"height":…,"dpi":…,
//!   "timezone":…}`, plus `"username"` and `"password"` when the device asks
//!   for credentials
//! - then text frames with Guacamole instructions; only input, display size,
//!   clipboard and stream acknowledgements reach guacd
//!
//! Server → browser:
//! - `{"type":"connected"}`, then text frames with whole Guacamole
//!   instructions; guacd reports trouble with the target as `error`
//! - or `{"type":"error","code":…,"params":{…}}`, then the socket closes.

use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use remotehub_gateway::guacamole::{self, Connection, GuacError, Handshake, Parser};
use secrecy::{ExposeSecret, SecretString};
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
/// Browser frames are input and clipboard chunks; guacd splits clipboard
/// data into blobs of a few KiB.
const MAX_BROWSER_FRAME: usize = 256 * 1024;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Start {
        width: u32,
        height: u32,
        dpi: Option<u32>,
        timezone: Option<String>,
        username: Option<String>,
        password: Option<SecretString>,
    },
}

pub async fn display(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, Problem> {
    connect::same_origin(&headers, &state)?;
    let target = connect::target(&state, &session, id, &["rdp", "vnc"]).await?;
    // The own account's password needs the key cookie, which only this
    // request carries.
    let own = if target.auth_mode == "own" {
        Some(
            own_account(&state, &session, &headers)
                .await?
                .map(|mut own| {
                    // NTLM needs the domain: the principal name carries it.
                    if let Some(upn) = &session.upn {
                        own.username.clone_from(upn);
                    }
                    own
                }),
        )
    } else {
        None
    };
    Ok(upgrade
        .max_message_size(MAX_BROWSER_FRAME)
        .on_upgrade(move |socket| run(socket, state, session, target, own, address)))
}

/// guacd's parameters for the device. Holds the password.
fn parameters<'a>(
    target: &'a Target,
    port: &'a str,
    credentials: &'a Credentials,
    password: &'a str,
    default_layout: &'a str,
) -> Vec<(&'static str, &'a str)> {
    let mut parameters = vec![("hostname", target.host.as_str()), ("port", port)];
    if target.protocol == "rdp" {
        // `DOMAIN\user` as typed, unless the domain is given separately.
        let (domain, username) = match credentials.username.split_once('\\') {
            Some((domain, user)) if credentials.domain.is_empty() => (domain, user),
            _ => (credentials.domain.as_str(), credentials.username.as_str()),
        };
        parameters.extend([
            ("username", username),
            ("password", password),
            ("domain", domain),
            ("security", "any"),
            // TODO(#53): pin the certificate on first use instead.
            ("ignore-cert", "true"),
            ("client-name", "remotehub"),
            // The session's input language on Windows follows it too.
            (
                "server-layout",
                target.keyboard_layout.as_deref().unwrap_or(default_layout),
            ),
            ("resize-method", "display-update"),
            ("disable-audio", "true"),
            ("disable-download", "true"),
            ("disable-upload", "true"),
        ]);
    } else {
        parameters.extend([
            ("username", credentials.username.as_str()),
            ("password", password),
        ]);
    }
    parameters
}

/// An IANA time zone name as the browser reports it, e.g. `Europe/Berlin`.
fn timezone(value: Option<String>) -> Option<String> {
    value.filter(|zone| {
        !zone.is_empty()
            && zone.len() <= 64
            && zone
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
    })
}

async fn run(
    mut socket: WebSocket,
    state: AppState,
    session: Session,
    target: Target,
    own: Option<Result<Credentials, Problem>>,
    address: String,
) {
    // 1. The browser says how big its display is (and, if asked, who to be).
    let start = match tokio::time::timeout(START_TIMEOUT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<ClientMessage>(&text).ok(),
        _ => None,
    };
    let Some(ClientMessage::Start {
        width,
        height,
        dpi,
        timezone: zone,
        username,
        password,
    }) = start
    else {
        send_problem(&mut socket, &Problem::new(ErrorCode::InvalidRequest)).await;
        return;
    };

    // 2. Credentials: from the vault, as entered, or the own account. VNC
    // servers usually only know a password; the user name may stay empty.
    let resolved = match own {
        Some(own) => own,
        None if target.protocol == "vnc" && target.auth_mode == "ask" => password
            .ok_or(Problem::new(ErrorCode::InvalidRequest).param("field", "password"))
            .map(|password| Credentials {
                username: username.unwrap_or_default().trim().to_owned(),
                domain: String::new(),
                login: Login::Password(password),
            }),
        None => connect::credentials(&state, &target, username, password).await,
    };
    let credentials = match resolved {
        Ok(Credentials {
            login: Login::Key(_),
            ..
        }) => Err(Problem::new(ErrorCode::InvalidRequest).param("field", "credential_id")),
        other => other,
    };
    let credentials = match credentials {
        Ok(credentials) => credentials,
        Err(problem) => {
            send_problem(&mut socket, &problem).await;
            return;
        }
    };

    // 3. Connect through guacd.
    let port = target.port.to_string();
    let zone = timezone(zone);
    let opened = {
        let password = match &credentials.login {
            Login::Password(password) => password.expose_secret(),
            Login::Key(_) => "",
        };
        let parameters = parameters(
            &target,
            &port,
            &credentials,
            password,
            &state.settings.rdp_keyboard_layout,
        );
        guacamole::open(
            &state.settings.guacd,
            &Handshake {
                protocol: &target.protocol,
                parameters: &parameters,
                width: width.clamp(320, 8192),
                height: height.clamp(200, 8192),
                dpi: dpi.unwrap_or(96).clamp(48, 384),
                timezone: zone.as_deref(),
            },
            CONNECT_TIMEOUT,
        )
        .await
    };
    let username = credentials.username.clone();
    drop(credentials);
    let mut connection = match opened {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!(device = %target.id, %error, "guacd connection failed");
            let problem = Problem::new(ErrorCode::ConnectionFailed);
            let _ = audit::record(
                &state.db,
                entry(
                    &session,
                    Action::ConnectionFailed,
                    target.id,
                    json!({ "protocol": target.protocol, "reason": guacd_reason(&error), "host": target.host }),
                    &address,
                ),
            )
            .await;
            send_problem(&mut socket, &problem).await;
            return;
        }
    };

    let _ = audit::record(
        &state.db,
        entry(
            &session,
            Action::ConnectionOpened,
            target.id,
            json!({
                "protocol": target.protocol, "host": target.host, "port": target.port,
                "username": username, "auth_mode": target.auth_mode,
                "credential_id": target.credential_id,
            }),
            &address,
        ),
    )
    .await;
    tracing::info!(device = %target.id, name = %target.name, user = %session.username,
        protocol = %target.protocol, guacd = %connection.id, "display session opened");
    send_json(&mut socket, json!({ "type": "connected" })).await;

    // 4. Relay until either side ends.
    let started = Instant::now();
    let outcome = relay(&mut socket, &mut connection).await;
    connection.close().await;
    let _ = socket.send(Message::Close(None)).await;

    let _ = audit::record(
        &state.db,
        entry(
            &session,
            Action::ConnectionClosed,
            target.id,
            json!({
                "protocol": target.protocol, "seconds": started.elapsed().as_secs(),
                "bytes_sent": outcome.sent, "bytes_received": outcome.received,
                "dropped_instructions": outcome.dropped, "error": outcome.error,
            }),
            &address,
        ),
    )
    .await;
}

#[derive(Default)]
struct Outcome {
    sent: usize,
    received: usize,
    /// Instructions from the browser that guacd must not get.
    dropped: usize,
    /// guacd's last `error`: message and status.
    error: Option<(String, String)>,
}

async fn relay(socket: &mut WebSocket, connection: &mut Connection) -> Outcome {
    let mut outcome = Outcome::default();
    let mut from_browser = Parser::default();
    loop {
        tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Text(text))) => {
                    from_browser.push(text.as_bytes());
                    let mut forward = String::new();
                    loop {
                        match from_browser.next_instruction() {
                            Ok(Some(instruction)) if guacamole::allowed_from_browser(&instruction.opcode) => {
                                forward.push_str(&instruction.encode());
                            }
                            // Internal instructions (empty opcode) and anything
                            // not allowed stay here.
                            Ok(Some(_)) => outcome.dropped += 1,
                            Ok(None) => break,
                            Err(_) => return outcome,
                        }
                    }
                    outcome.sent += forward.len();
                    if !forward.is_empty() && connection.send(&forward).await.is_err() {
                        return outcome;
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return outcome,
                Some(Ok(_)) => {}
            },
            received = connection.receive() => match received {
                Ok(Some(text)) => {
                    outcome.received += text.len();
                    if text.contains("5.error,") {
                        outcome.error = last_error(&text).or(outcome.error.take());
                    }
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        return outcome;
                    }
                }
                Ok(None) | Err(_) => return outcome,
            },
        }
    }
}

/// The last `error` instruction in a text of whole instructions.
fn last_error(text: &str) -> Option<(String, String)> {
    let mut parser = Parser::default();
    parser.push(text.as_bytes());
    let mut last = None;
    while let Ok(Some(instruction)) = parser.next_instruction() {
        if instruction.opcode == "error" {
            let mut args = instruction.args.into_iter();
            last = Some((
                args.next().unwrap_or_default(),
                args.next().unwrap_or_default(),
            ));
        }
    }
    last
}

fn guacd_reason(error: &GuacError) -> String {
    match error {
        GuacError::Unreachable(_) => "guacd_unreachable".to_owned(),
        GuacError::Timeout => "guacd_timeout".to_owned(),
        GuacError::Refused { status, .. } => format!("guacd_refused_{status}"),
        GuacError::Malformed(_) | GuacError::Io(_) => "guacd_protocol".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_zones_are_names_not_text() {
        assert_eq!(
            timezone(Some("Europe/Berlin".into())).as_deref(),
            Some("Europe/Berlin")
        );
        assert_eq!(
            timezone(Some("Etc/GMT+1".into())).as_deref(),
            Some("Etc/GMT+1")
        );
        for bad in ["", "Europe/Berlin;4.nop", "a b", &"x".repeat(65)] {
            assert_eq!(timezone(Some(bad.to_owned())), None, "{bad}");
        }
    }

    #[test]
    fn rdp_gets_the_layout_and_the_domain_it_needs() {
        let mut target = Target {
            id: Uuid::nil(),
            name: "x".into(),
            protocol: "rdp".into(),
            host: "desktop".into(),
            port: 3389,
            auth_mode: "ask".into(),
            credential_id: None,
            host_key: None,
            keyboard_layout: None,
        };
        let credentials = Credentials {
            username: r"EXAMPLE\alice".into(),
            domain: String::new(),
            login: Login::Password(SecretString::from("secret".to_owned())),
        };
        let value = |target: &Target, name: &str| {
            parameters(target, "3389", &credentials, "secret", "en-us-qwerty")
                .into_iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_owned())
        };
        assert_eq!(
            value(&target, "server-layout").as_deref(),
            Some("en-us-qwerty")
        );
        assert_eq!(value(&target, "domain").as_deref(), Some("EXAMPLE"));
        assert_eq!(value(&target, "username").as_deref(), Some("alice"));
        target.keyboard_layout = Some("de-de-qwertz".into());
        assert_eq!(
            value(&target, "server-layout").as_deref(),
            Some("de-de-qwertz")
        );
        target.protocol = "vnc".into();
        assert_eq!(value(&target, "server-layout"), None);
        assert_eq!(
            value(&target, "username").as_deref(),
            Some(r"EXAMPLE\alice")
        );
    }

    #[test]
    fn the_last_error_names_the_cause() {
        let text = "4.sync,1.1;5.error,17.DNS lookup failed,3.519;5.error,8.Aborted.,3.776;";
        assert_eq!(last_error(text), Some(("Aborted.".into(), "776".into())));
        assert_eq!(last_error("4.sync,1.1;"), None);
    }
}
