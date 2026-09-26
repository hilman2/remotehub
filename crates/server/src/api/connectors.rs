//! Site connectors (#19, ADR 0008).
//!
//! For connectors, signed in with their token (`Authorization: Bearer`) and
//! speaking remotehub's protocol version (`remotehub-connector-protocol`):
//! - `GET /api/connectors/control`: the control WebSocket; remotehub sends
//!   `open` requests, the connector reports streams it could not open
//! - `GET /api/connectors/streams/{id}`: the WebSocket for one stream that
//!   remotehub asked for
//! - `POST /api/connectors/state`: whether the customer lets remotehub in
//!   (#165); a closed connector has no control socket and reports only here
//!
//! For people:
//! - `GET /api/connectors`: names, whether they are online, whether the
//!   customer keeps access open, and how many connections they carry, for
//!   everyone signed in (the device form offers them)
//! - `POST /api/connectors`: administrators create one; the answer holds its
//!   token, which is shown this once
//! - `DELETE /api/connectors/{id}`: administrators delete one no device uses

use std::time::Duration;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use data_encoding::BASE64URL_NOPAD;
use remotehub_connector::protocol::{self, Report};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};
use uuid::Uuid;

use super::catalog::{body, name};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::connectors::token_hash;
use crate::session::Session;

/// Keeps proxies and NAT from closing an idle control socket.
const PING: Duration = Duration::from_secs(30);

#[derive(Serialize, sqlx::FromRow)]
pub struct ConnectorView {
    id: Uuid,
    name: String,
    #[sqlx(skip)]
    online: bool,
    /// Connections it carries now.
    #[sqlx(skip)]
    streams: usize,
    /// Connections it has carried since remotehub started.
    #[sqlx(skip)]
    streams_carried: u64,
    /// `open`, `partly` (some devices, #180) or `closed`, as the connector
    /// reports it; none without a recent report.
    #[sqlx(skip)]
    access: Option<&'static str>,
    /// RFC 3339; while open until a point in time.
    #[sqlx(skip)]
    open_until: Option<String>,
    /// RFC 3339 in UTC; none before the first connection.
    last_seen_at: Option<String>,
}

#[derive(Deserialize)]
pub struct ConnectorInput {
    name: String,
}

fn require_admin(session: &Session) -> Result<(), Problem> {
    if session.is_admin() {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::Forbidden))
    }
}

fn actor(session: &Session) -> Actor<'_> {
    Actor {
        id: Some(session.user_id),
        name: &session.username,
    }
}

pub async fn list(
    State(state): State<AppState>,
    _session: Session,
) -> Result<Json<Vec<ConnectorView>>, Problem> {
    let mut connectors: Vec<ConnectorView> = sqlx::query_as(
        r#"SELECT id, name,
                  to_char(last_seen_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') AS last_seen_at
           FROM connectors ORDER BY lower(name)"#,
    )
    .fetch_all(&state.db)
    .await?;
    for connector in &mut connectors {
        connector.online = state.connectors.is_online(connector.id);
        connector.streams = state.connectors.streams(connector.id);
        connector.streams_carried = state.connectors.streams_carried(connector.id);
        if let Some(access) = state.connectors.access(connector.id) {
            connector.access = Some(if access.open {
                "open"
            } else if access.partly {
                "partly"
            } else {
                "closed"
            });
            connector.open_until = access.until;
        }
    }
    Ok(Json(connectors))
}

pub async fn create(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<ConnectorInput>, JsonRejection>,
) -> Result<(StatusCode, Json<serde_json::Value>), Problem> {
    require_admin(&session)?;
    let name = name(&body(input)?.name, "name")?;
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).expect("the OS has randomness");
    let token = zeroize::Zeroizing::new(format!("rhc_{}", BASE64URL_NOPAD.encode(&secret)));
    let mut tx = state.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO connectors (name, token_hash) VALUES ($1, $2) RETURNING id",
    )
    .bind(&name)
    .bind(token_hash(&token))
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| match error.as_database_error() {
        Some(db) if db.is_unique_violation() => Problem::new(ErrorCode::NameTaken),
        _ => Problem::from(error),
    })?;
    audit::record(
        &mut *tx,
        Entry {
            actor: actor(&session),
            action: Action::ConnectorCreated,
            object: Some(("connector", id)),
            details: json!({ "name": name }),
            address: Some(&address),
        },
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "name": name, "token": token.as_str() })),
    ))
}

pub async fn delete(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    require_admin(&session)?;
    let mut tx = state.db.begin().await?;
    let deleted: Option<String> =
        sqlx::query_scalar("DELETE FROM connectors WHERE id = $1 RETURNING name")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|error| match error.as_database_error() {
                // Devices still go through it (ON DELETE RESTRICT).
                Some(db) if db.code().as_deref() == Some("23001") => {
                    Problem::new(ErrorCode::ConnectorInUse)
                }
                _ => Problem::from(error),
            })?;
    let Some(name) = deleted else {
        return Err(Problem::new(ErrorCode::NotFound));
    };
    audit::record(
        &mut *tx,
        Entry {
            actor: actor(&session),
            action: Action::ConnectorDeleted,
            object: Some(("connector", id)),
            details: json!({ "name": name }),
            address: Some(&address),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The connector whose token the request carries.
async fn signed_in(state: &AppState, headers: &HeaderMap) -> Result<Uuid, Problem> {
    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(Problem::new(ErrorCode::Unauthenticated))?;
    sqlx::query_scalar("SELECT id FROM connectors WHERE token_hash = $1")
        .bind(token_hash(token))
        .fetch_optional(&state.db)
        .await?
        .ok_or(Problem::new(ErrorCode::Unauthenticated))
}

/// The refusal of a connector that speaks another protocol version, if it
/// does. The refusal names remotehub's version in the same header, so the
/// connector can tell its administrator which release to run.
fn other_protocol(headers: &HeaderMap) -> Option<Response> {
    let theirs = headers
        .get(protocol::HEADER)
        .and_then(|value| value.to_str().ok());
    if theirs == Some(protocol::VERSION.to_string().as_str()) {
        return None;
    }
    let mut response = Problem::new(ErrorCode::ConnectorProtocol)
        .param("version", protocol::VERSION)
        .into_response();
    response
        .headers_mut()
        .insert(protocol::HEADER, protocol::VERSION.into());
    Some(response)
}

pub async fn control(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, Problem> {
    let connector = signed_in(&state, &headers).await?;
    if let Some(refusal) = other_protocol(&headers) {
        return Ok(refusal);
    }
    Ok(upgrade.on_upgrade(move |socket| run_control(socket, state, connector)))
}

async fn run_control(mut socket: WebSocket, state: AppState, connector: Uuid) {
    let (mut requests, registration) = state.connectors.register(connector);
    seen(&state, connector).await;
    tracing::info!(%connector, "connector online");
    let mut ping = tokio::time::interval(PING);
    loop {
        tokio::select! {
            request = requests.recv() => {
                let Some(request) = request else { break };
                let text = serde_json::to_string(&request).expect("requests serialize");
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            message = socket.recv() => match message {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<Report>(&text) {
                    Ok(Report::Failed { id, reason, not_open }) => {
                        state.connectors.fail(connector, id, reason, not_open);
                    }
                    Ok(Report::Checked { id, open }) => state.connectors.checked(connector, id, open),
                    Err(error) => tracing::warn!(%connector, %error, "unknown message from a connector"),
                },
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            _ = ping.tick() => {
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }
    drop(registration);
    seen(&state, connector).await;
    tracing::info!(%connector, "connector offline");
}

async fn seen(state: &AppState, connector: Uuid) {
    let _ = sqlx::query("UPDATE connectors SET last_seen_at = now() WHERE id = $1")
        .bind(connector)
        .execute(&state.db)
        .await;
}

/// The connector reports whether its customer lets remotehub in. A change
/// against its previous report goes to the audit log; after a restart of
/// remotehub, the first report has nothing to compare with.
pub async fn report_state(
    State(state): State<AppState>,
    headers: HeaderMap,
    input: Result<Json<protocol::State>, JsonRejection>,
) -> Result<Response, Problem> {
    let connector = signed_in(&state, &headers).await?;
    if let Some(refusal) = other_protocol(&headers) {
        return Ok(refusal);
    }
    let mut reported = body(input)?;
    // Only a point in time reaches the audit log and the UI, written the
    // same way whatever the connector sent.
    reported.until = match reported.until.as_deref() {
        None => None,
        Some(text) => Some(
            OffsetDateTime::parse(text, &Rfc3339)
                .ok()
                .and_then(|at| at.to_offset(UtcOffset::UTC).format(&Rfc3339).ok())
                .ok_or_else(|| Problem::new(ErrorCode::InvalidRequest).param("field", "until"))?,
        ),
    };
    let before = state.connectors.report(connector, reported.clone());
    if before.is_some_and(|before| before != reported) {
        let name: String = sqlx::query_scalar("SELECT name FROM connectors WHERE id = $1")
            .bind(connector)
            .fetch_one(&state.db)
            .await?;
        audit::record(
            &state.db,
            Entry {
                actor: Actor {
                    id: None,
                    name: &name,
                },
                action: if reported.open {
                    Action::ConnectorOpened
                } else {
                    Action::ConnectorClosed
                },
                object: Some(("connector", connector)),
                details: json!({ "name": name, "until": reported.until }),
                address: None,
            },
        )
        .await?;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn stream(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, Problem> {
    let connector = signed_in(&state, &headers).await?;
    if let Some(refusal) = other_protocol(&headers) {
        return Ok(refusal);
    }
    let reply = state
        .connectors
        .claim(connector, id)
        .ok_or(Problem::new(ErrorCode::NotFound))?;
    Ok(upgrade.on_upgrade(move |socket| async move {
        let _ = reply.send(Ok(socket));
    }))
}
