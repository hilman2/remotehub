//! `GET /api/devices/{id}/display`: an RDP, VNC or HTTPS session in the
//! browser over a WebSocket (ADR 0003), drawn by the Guacamole client.
//!
//! The server opens the connection through guacd with the credentials
//! resolved here; the browser only exchanges Guacamole instructions for
//! input and drawing, and never sees the connection's parameters. An HTTPS
//! device is a Chromium of the browser service (ADR 0007), which signs in
//! with the credentials and is shown through guacd as a VNC display.
//!
//! Browser → server:
//! - first text frame: `{"type":"start","width":…,"height":…,"dpi":…,
//!   "timezone":…}`, plus `"username"` and `"password"` when the device asks
//!   for credentials, and `"purpose"` when the user must state one (#90)
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
use remotehub_browser::client::{self as browser, BrowserError};
use remotehub_browser::protocol::Reply;
use remotehub_gateway::guacamole::{self, Connection, GuacError, Handshake, Parser};
use remotehub_gateway::rdp;
use remotehub_gateway::tls::{self, ProbeError};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::connect::{
    self, Credentials, Engine, Login, Target, entry, own_account, send_json, send_problem,
};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::audit::{self, Action};
use crate::session::Session;
use crate::{AppState, refresh};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Learning an RDP server's certificate: TCP, X.224 and the TLS handshake.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const START_TIMEOUT: Duration = Duration::from_secs(120);
/// The browser service starts a display and Chromium: a second or two.
const BROWSER_TIMEOUT: Duration = Duration::from_secs(20);
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
        purpose: Option<String>,
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
    let session = refresh::before_connecting(&state, session).await?;
    let target = connect::target(&state, &session, id, &["rdp", "vnc", "https"]).await?;
    // The own account's password needs the key cookie, which only this
    // request carries.
    let own = if target.auth_mode == "own" {
        Some(
            own_account(&state, &session, &headers)
                .await?
                .map(|mut own| {
                    // NTLM needs the domain: the principal name carries it.
                    // A web interface gets the name the user signs in with.
                    if let Some(upn) = session.upn.as_ref().filter(|_| target.protocol != "https") {
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

/// guacd's parameters for the device at `host:port` (its own address, or a
/// forward to it). Holds the password.
fn parameters<'a>(
    target: &'a Target,
    host: &'a str,
    port: &'a str,
    credentials: &'a Credentials,
    password: &'a str,
    default_layout: &'a str,
    certificate: &'a str,
) -> Vec<(&'static str, &'a str)> {
    let mut parameters = vec![("hostname", host), ("port", port)];
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
            // FreeRDP accepts this certificate and no other (`sha256:AA:BB:…`).
            ("cert-fingerprints", certificate),
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
        purpose,
    }) = start
    else {
        send_problem(&mut socket, &Problem::new(ErrorCode::InvalidRequest)).await;
        return;
    };
    let purpose = match connect::purpose(&state, &session, purpose).await {
        Ok(purpose) => purpose,
        Err(problem) => {
            send_problem(&mut socket, &problem).await;
            return;
        }
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

    // 3. RDP and HTTPS: the certificate the device presents must be the
    // pinned one, or becomes it (trust on first use, like SSH host keys).
    let port = u16::try_from(target.port).unwrap_or_default();
    let probe_route = match target.protocol.as_str() {
        "rdp" | "https" => {
            match connect::route(&state, &target, Engine::Server, &session.username).await {
                Ok(route) => Some(route),
                Err(problem) => {
                    fail(&mut socket, &state, &session, &target, &problem, &address).await;
                    return;
                }
            }
        }
        _ => None,
    };
    let probed = match (target.protocol.as_str(), &probe_route) {
        ("rdp", Some(route)) => Some(
            rdp::certificate_fingerprint(&route.host, route.port, PROBE_TIMEOUT)
                .await
                .map(|fingerprint| (fingerprint, None)),
        ),
        ("https", Some(route)) => Some(
            tls::https_certificate(&target.host, &route.host, route.port, PROBE_TIMEOUT)
                .await
                .map(|presented| (presented.fingerprint, Some(presented.spki))),
        ),
        _ => None,
    };
    drop(probe_route);
    let (certificate, spki) = match probed {
        None => (None, None),
        Some(Ok((presented, spki))) => match &target.certificate_fingerprint {
            Some(pinned) if *pinned != presented => {
                let problem = Problem::new(ErrorCode::CertificateChanged)
                    .param("expected", pinned.as_str())
                    .param("presented", presented.as_str());
                fail(&mut socket, &state, &session, &target, &problem, &address).await;
                return;
            }
            _ => (Some(presented), spki),
        },
        Some(Err(error)) => {
            tracing::warn!(device = %target.id, %error, "certificate probe failed");
            let problem = Problem::new(match error {
                ProbeError::NoTls => ErrorCode::TlsRequired,
                ProbeError::Unreachable(_) | ProbeError::Timeout => ErrorCode::TargetUnreachable,
                ProbeError::Protocol(_) | ProbeError::Tls(_) => ErrorCode::ConnectionFailed,
            });
            fail(&mut socket, &state, &session, &target, &problem, &address).await;
            return;
        }
    };
    let accepted = certificate
        .as_ref()
        .map(|fingerprint| format!("sha256:{fingerprint}"));
    let password = match &credentials.login {
        Login::Password(password) => password.expose_secret(),
        Login::Key(_) => "",
    };
    let zone = timezone(zone);

    // 4. Where the engine reaches the device: guacd for RDP and VNC, the
    // browser service for HTTPS.
    let engine = if spki.is_some() {
        &state.settings.browser
    } else {
        &state.settings.guacd
    };
    let route =
        match connect::route(&state, &target, Engine::Service(engine), &session.username).await {
            Ok(route) => route,
            Err(problem) => {
                fail(&mut socket, &state, &session, &target, &problem, &address).await;
                return;
            }
        };

    // 5. HTTPS: a browser on the device, signing in with the credentials.
    let mut browser = match &spki {
        Some(spki) => {
            let via = route.forwarded().then(|| route.authority());
            let request = browser::Request {
                host: &target.host,
                port,
                via: via.as_deref(),
                spki,
                width,
                height,
                timezone: zone.as_deref(),
                login: Some((&credentials.username, password)),
            };
            match browser::open(&state.settings.browser, &request, BROWSER_TIMEOUT).await {
                Ok(browser) => Some(browser),
                Err(error) => {
                    tracing::warn!(device = %target.id, %error, "browser service failed");
                    let reason = match error {
                        BrowserError::Failed(reason) => format!("browser_{reason}"),
                        _ => "browser_unreachable".to_owned(),
                    };
                    let _ = audit::record(
                        &state.db,
                        entry(
                            &session,
                            Action::ConnectionFailed,
                            target.id,
                            json!({ "protocol": target.protocol, "reason": reason, "host": target.host }),
                            &address,
                        ),
                    )
                    .await;
                    send_problem(&mut socket, &Problem::new(ErrorCode::ConnectionFailed)).await;
                    return;
                }
            }
        }
        None => None,
    };

    // 6. Connect through guacd: to the device, or to the browser's display.
    let port = route.port.to_string();
    let opened = {
        let vnc_port = browser.as_ref().map(|b| b.vnc_port.to_string());
        let parameters = match (&browser, &vnc_port) {
            (Some(browser), Some(vnc_port)) => vec![
                ("hostname", browser.vnc_host.as_str()),
                ("port", vnc_port.as_str()),
                ("password", browser.vnc_password.as_str()),
            ],
            _ => parameters(
                &target,
                &route.host,
                &port,
                &credentials,
                password,
                &state.settings.rdp_keyboard_layout,
                accepted.as_deref().unwrap_or_default(),
            ),
        };
        guacamole::open(
            &state.settings.guacd,
            &Handshake {
                protocol: if browser.is_some() {
                    "vnc"
                } else {
                    &target.protocol
                },
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
                "credential_id": target.credential_id, "purpose": purpose,
            }),
            &address,
        ),
    )
    .await;
    let journal = connect::journal_opened(&state, &session, &target, &purpose).await;
    let pinned_now = match &certificate {
        Some(fingerprint) if target.certificate_fingerprint.is_none() => {
            pin_certificate(&state, &session, &target, fingerprint, &address).await
        }
        _ => false,
    };
    tracing::info!(device = %target.id, name = %target.name, user = %session.username,
        protocol = %target.protocol, guacd = %connection.id, "display session opened");
    send_json(
        &mut socket,
        json!({ "type": "connected", "certificate_fingerprint": certificate, "pinned": pinned_now }),
    )
    .await;

    // 6. Relay until either side ends; the browser ends with the session.
    let started = Instant::now();
    let outcome = relay(&mut socket, &mut connection, browser.as_mut()).await;
    connection.close().await;
    drop(browser);
    let _ = socket.send(Message::Close(None)).await;
    connect::journal_closed(&state, journal).await;

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
                "signed_in": outcome.signed_in,
            }),
            &address,
        ),
    )
    .await;
}

/// Reports a connection that did not start, to the audit log and the browser.
async fn fail(
    socket: &mut WebSocket,
    state: &AppState,
    session: &Session,
    target: &Target,
    problem: &Problem,
    address: &str,
) {
    let _ = audit::record(
        &state.db,
        entry(
            session,
            Action::ConnectionFailed,
            target.id,
            json!({ "protocol": target.protocol, "reason": problem.code, "host": target.host }),
            address,
        ),
    )
    .await;
    send_problem(socket, problem).await;
}

/// Pins the certificate of the device's first connection; false if another
/// connection was first.
async fn pin_certificate(
    state: &AppState,
    session: &Session,
    target: &Target,
    fingerprint: &str,
    address: &str,
) -> bool {
    let pinned = sqlx::query(
        "UPDATE devices SET certificate_fingerprint = $2, certificate_pinned_at = now()
         WHERE id = $1 AND certificate_fingerprint IS NULL",
    )
    .bind(target.id)
    .bind(fingerprint)
    .execute(&state.db)
    .await
    .is_ok_and(|r| r.rows_affected() == 1);
    if pinned {
        let _ = audit::record(
            &state.db,
            entry(
                session,
                Action::CertificatePinned,
                target.id,
                json!({ "fingerprint": fingerprint }),
                address,
            ),
        )
        .await;
    }
    pinned
}

#[derive(Default)]
struct Outcome {
    sent: usize,
    received: usize,
    /// Instructions from the browser that guacd must not get.
    dropped: usize,
    /// guacd's last `error`: message and status.
    error: Option<(String, String)>,
    /// HTTPS: whether the browser service filled in the sign-in form; none
    /// until it says, and for other protocols.
    signed_in: Option<bool>,
}

async fn relay(
    socket: &mut WebSocket,
    connection: &mut Connection,
    mut browser: Option<&mut browser::Session>,
) -> Outcome {
    let mut outcome = Outcome::default();
    let mut from_browser = Parser::default();
    loop {
        tokio::select! {
            event = browser_event(browser.as_deref_mut()) => match event {
                Some(Reply::Filled) => outcome.signed_in = Some(true),
                Some(Reply::NotFilled { reason }) => {
                    tracing::info!(reason, "the browser did not sign in");
                    outcome.signed_in = Some(false);
                }
                Some(_) => {}
                // Chromium or its display ended.
                None => return outcome,
            },
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

/// The browser service's next message; never resolves without a browser.
async fn browser_event(browser: Option<&mut browser::Session>) -> Option<Reply> {
    match browser {
        Some(browser) => browser.next().await,
        None => std::future::pending().await,
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
            certificate_fingerprint: None,
            connector_id: None,
        };
        let credentials = Credentials {
            username: r"EXAMPLE\alice".into(),
            domain: String::new(),
            login: Login::Password(SecretString::from("secret".to_owned())),
        };
        let value = |target: &Target, name: &str| {
            parameters(
                target,
                "desktop",
                "3389",
                &credentials,
                "secret",
                "en-us-qwerty",
                "sha256:AB:CD",
            )
            .into_iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.to_owned())
        };
        assert_eq!(
            value(&target, "server-layout").as_deref(),
            Some("en-us-qwerty")
        );
        assert_eq!(
            value(&target, "cert-fingerprints").as_deref(),
            Some("sha256:AB:CD")
        );
        assert_eq!(value(&target, "ignore-cert"), None);
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
