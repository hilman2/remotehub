//! What every connection shares, whatever the protocol: the device and the
//! caller's right to connect to it, the WebSocket's origin, the credentials
//! resolved on the server, and the audit entries.

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket};
use axum::http::HeaderMap;
use axum::http::header::ORIGIN;
use remotehub_directory::AuthError;
use remotehub_directory::laps::LapsError;
use remotehub_gateway::ssh::SshKey;
use remotehub_model::{ObjectId, Role};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use uuid::Uuid;

use super::problem::{ErrorCode, Problem};
use crate::audit::{Action, Actor, Entry};
use crate::connectors::{Forward, towards};
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
    pub connector_id: Option<Uuid>,
}

/// Who opens the connection to the device.
pub enum Engine<'a> {
    /// remotehub itself: SSH and certificate probes.
    Server,
    /// A service next to remotehub, `host:port`: guacd or the browser
    /// service.
    Service(&'a str),
}

/// Where an engine reaches the device: the device's own host and port, or,
/// behind a site connector, a forward that lives as long as the route.
pub struct Route {
    pub host: String,
    pub port: u16,
    _forward: Option<Forward>,
}

impl Route {
    /// Whether the engine connects to a forward rather than the device.
    pub fn forwarded(&self) -> bool {
        self._forward.is_some()
    }

    /// `host:port`, IPv6 addresses in brackets.
    pub fn authority(&self) -> String {
        authority(&self.host, self.port)
    }
}

fn authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// The way `engine` reaches the device (ADR 0008).
pub async fn route(
    state: &AppState,
    target: &Target,
    engine: Engine<'_>,
) -> Result<Route, Problem> {
    let port = u16::try_from(target.port).unwrap_or_default();
    let Some(connector) = target.connector_id else {
        return Ok(Route {
            host: target.host.clone(),
            port,
            _forward: None,
        });
    };
    if !state.connectors.is_online(connector) {
        return Err(Problem::new(ErrorCode::ConnectorOffline));
    }
    let loopback = IpAddr::from(Ipv4Addr::LOCALHOST);
    let (bind, peers) = match engine {
        Engine::Server => (loopback, vec![loopback]),
        Engine::Service(service) => towards(service).await.map_err(|error| {
            tracing::warn!(service, %error, "no route to the service");
            Problem::new(ErrorCode::ConnectionFailed)
        })?,
    };
    let forward = Forward::open(
        state.connectors.clone(),
        connector,
        authority(&target.host, port),
        bind,
        peers,
    )
    .await
    .map_err(|error| {
        tracing::warn!(%error, "cannot open a forward");
        Problem::new(ErrorCode::ConnectionFailed)
    })?;
    Ok(Route {
        host: forward.address.ip().to_string(),
        port: forward.address.port(),
        _forward: Some(forward),
    })
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
                certificate_fingerprint, connector_id
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

/// The longest purpose of a connection, in characters.
const PURPOSE_MAX: usize = 500;

/// Whether the caller states a purpose before every connection (#90): their
/// own SID or one of their groups is among the `purpose_principals`.
pub async fn purpose_required(db: &sqlx::PgPool, session: &Session) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM purpose_principals WHERE principal_sid = ANY($1))",
    )
    .bind(session.sids())
    .fetch_one(db)
    .await
}

/// The purpose the caller gave, trimmed, or empty if they gave none and need
/// not. Checked before anything reaches the device.
pub async fn purpose(
    state: &AppState,
    session: &Session,
    given: Option<String>,
) -> Result<String, Problem> {
    let given = given.unwrap_or_default().trim().to_owned();
    if given.chars().count() > PURPOSE_MAX {
        return Err(Problem::new(ErrorCode::InvalidRequest).param("field", "purpose"));
    }
    if given.is_empty() && purpose_required(&state.db, session).await? {
        return Err(Problem::new(ErrorCode::PurposeRequired));
    }
    Ok(given)
}

/// Writes an opened connection into the device's journal and returns the
/// entry, for [`journal_closed`]. A journal that cannot be written does not
/// stop the session: the audit log has the connection as well.
pub async fn journal_opened(
    state: &AppState,
    session: &Session,
    target: &Target,
    purpose: &str,
) -> Option<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO device_journal (device_id, user_id, kind, protocol, text)
         VALUES ($1, $2, 'connection', $3, $4) RETURNING id",
    )
    .bind(target.id)
    .bind(session.user_id)
    .bind(&target.protocol)
    .bind(purpose)
    .fetch_one(&state.db)
    .await
    .inspect_err(|error| tracing::warn!(device = %target.id, %error, "cannot write the journal"))
    .ok()
}

/// Notes the end of the session in its journal entry.
pub async fn journal_closed(state: &AppState, entry: Option<Uuid>) {
    let Some(entry) = entry else { return };
    let _ = sqlx::query("UPDATE device_journal SET ended_at = now() WHERE id = $1")
        .bind(entry)
        .execute(&state.db)
        .await;
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

/// How long a certificate from the SSH CA is valid. It only has to last
/// until the target has accepted it; the session runs on after that.
const CERTIFICATE_VALIDITY: Duration = Duration::from_secs(5 * 60);

/// For the sign-in mode `certificate`: a fresh key with a certificate from
/// remotehub's SSH CA, for the signed-in user as principal and login name.
pub fn certificate(
    state: &AppState,
    session: &Session,
    target: &Target,
) -> Result<Credentials, Problem> {
    let ca = state
        .settings
        .ssh_ca
        .as_ref()
        .ok_or(Problem::new(ErrorCode::SshCaUnavailable))?;
    // The target logs the key ID with every sign-in.
    let key_id = format!("remotehub {} device {}", session.username, target.id);
    let key = ca
        .issue(&session.username, &key_id, CERTIFICATE_VALIDITY)
        .map_err(|error| {
            tracing::error!(%error, "cannot issue an SSH certificate");
            Problem::new(ErrorCode::Internal)
        })?;
    Ok(Credentials {
        username: session.username.clone(),
        domain: String::new(),
        login: Login::Key(Box::new(key)),
    })
}

/// `GET /api/ssh-ca.pub`: the SSH CA's public key for `TrustedUserCAKeys`
/// on the targets. It is public, so no session is needed to fetch it.
pub async fn ssh_ca_public_key(State(state): State<AppState>) -> Result<String, Problem> {
    state
        .settings
        .ssh_ca
        .as_ref()
        .map(|ca| format!("{}\n", ca.public_key()))
        .ok_or(Problem::new(ErrorCode::NotFound))
}

/// A sealed field of the owner's current version as text: a credential's,
/// or a device's own credentials'.
async fn stored_text(
    state: &AppState,
    owner: Uuid,
    version: i32,
    field: &str,
) -> Result<Option<SecretString>, Problem> {
    let secret = secrets::load(&state.db, &state.vault, owner, version, field)
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

/// How to sign in with the sealed secrets of `owner`'s `version`: a
/// password, or (`kind` `ssh_key`) a key with its passphrase and certificate.
async fn sealed_login(
    state: &AppState,
    owner: Uuid,
    version: i32,
    kind: &str,
) -> Result<Login, Problem> {
    if kind != "ssh_key" {
        let password = stored_text(state, owner, version, PASSWORD_FIELD)
            .await?
            .ok_or(Problem::new(ErrorCode::Internal))?;
        return Ok(Login::Password(password));
    }
    let private_key = stored_text(state, owner, version, PRIVATE_KEY_FIELD)
        .await?
        .ok_or(Problem::new(ErrorCode::Internal))?;
    let passphrase = stored_text(state, owner, version, PASSPHRASE_FIELD).await?;
    let certificate = stored_text(state, owner, version, CERTIFICATE_FIELD).await?;
    let key = SshKey::parse(
        private_key.expose_secret(),
        passphrase.as_ref().map(|p| p.expose_secret()),
        certificate.as_ref().map(|c| c.expose_secret()),
    )
    .map_err(|error| {
        tracing::error!(%error, "a stored SSH key does not open");
        Problem::new(ErrorCode::Internal)
    })?;
    Ok(Login::Key(Box::new(key)))
}

/// Credentials for the device's sign-in mode `stored`, `device`, `laps` or `ask`.
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
            Ok(Credentials {
                username,
                domain,
                login: sealed_login(state, credential, version, &kind).await?,
            })
        }
        "laps" => {
            let directory = state
                .directory
                .as_ref()
                .ok_or(Problem::new(ErrorCode::DirectoryUnavailable))?;
            let laps = directory
                .laps_password(&target.host)
                .await
                .map_err(|error| {
                    tracing::warn!(device = %target.id, %error, "no LAPS password");
                    match error {
                        LapsError::Directory(AuthError::Unavailable(_)) => {
                            Problem::new(ErrorCode::DirectoryUnavailable)
                        }
                        _ => Problem::new(ErrorCode::LapsUnavailable),
                    }
                })?;
            // The computer's name as the domain makes it a local account.
            Ok(Credentials {
                username: laps.account,
                domain: laps.computer,
                login: Login::Password(laps.password),
            })
        }
        "device" => {
            let (username, domain, version, kind): (String, String, i32, String) = sqlx::query_as(
                "SELECT username, domain, secret_version, secret_kind FROM devices WHERE id = $1",
            )
            .bind(target.id)
            .fetch_one(&state.db)
            .await?;
            Ok(Credentials {
                username,
                domain,
                login: sealed_login(state, target.id, version, &kind).await?,
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
