//! Asking the customer for access through a site connector (#181, ADR 0013).
//! A user who may connect asks for a device, or for a folder's devices behind
//! its connector, for a while and with a reason. The connector fetches the
//! pending requests with its state report ([`pending`]) and brings the
//! customer's answer the same way ([`answer`]); remotehub never writes to it.
//!
//! - `GET /api/connector-requests`: the caller's own requests; an
//!   administrator sees everyone's
//! - `POST /api/connector-requests`: ask
//! - `DELETE /api/connector-requests/{id}`: withdraw a pending request

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use remotehub_connector::protocol::{self, Answer, CustomerRequest, Pending, RequestTarget};
use remotehub_connector::requests::{self, plain, readable};
use remotehub_model::{ObjectId, Role};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::catalog::{ObjectQuery, body, context, entry, invalid, object};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::audit::{self, Action, Actor, Entry};
use crate::session::Session;
use crate::{AppState, catalog};

/// Requests as the UI reads them, then `$filter`, a literal so the query
/// stays static. A request waits a day for the customer; older, it reads
/// `expired`, and the connector no longer sees it ([`pending`]).
macro_rules! select {
    ($filter:literal) => {
        concat!(
            "SELECT r.id, r.connector_id, c.name AS connector_name, r.folder_id, r.device_id,
                    r.object_name, r.targets, r.requester_name, r.minutes, r.reason,
                    CASE WHEN r.status = 'pending' AND r.created_at < now() - interval '1 day'
                         THEN 'expired' ELSE r.status END AS status,
                    r.answered_by,
                    to_char(r.answered_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS answered_at,
                    to_char(r.until AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS until,
                    to_char(r.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS created_at
             FROM connector_requests r JOIN connectors c ON c.id = r.connector_id ",
            $filter
        )
    };
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    connector_id: Uuid,
    connector_name: String,
    folder_id: Option<Uuid>,
    device_id: Option<Uuid>,
    object_name: String,
    targets: sqlx::types::Json<Vec<RequestTarget>>,
    requester_name: String,
    minutes: i32,
    reason: String,
    status: String,
    answered_by: Option<String>,
    answered_at: Option<String>,
    until: Option<String>,
    created_at: String,
}

#[derive(Serialize)]
pub struct ConnectorRequest {
    id: Uuid,
    connector_id: Uuid,
    connector_name: String,
    /// The folder or device asked for; none once it is deleted.
    object: Option<ObjectId>,
    object_name: String,
    targets: Vec<RequestTarget>,
    requester_name: String,
    minutes: i32,
    reason: String,
    /// `pending`, `approved`, `refused`, `cancelled` or `expired`.
    status: String,
    /// The connector user who answered, as the connector reports them.
    answered_by: Option<String>,
    answered_at: Option<String>,
    /// When an approval ends.
    until: Option<String>,
    created_at: String,
}

impl From<Row> for ConnectorRequest {
    fn from(row: Row) -> Self {
        ConnectorRequest {
            object: catalog::object_id(row.folder_id, row.device_id, None),
            id: row.id,
            connector_id: row.connector_id,
            connector_name: row.connector_name,
            object_name: row.object_name,
            targets: row.targets.0,
            requester_name: row.requester_name,
            minutes: row.minutes,
            reason: row.reason,
            status: row.status,
            answered_by: row.answered_by,
            answered_at: row.answered_at,
            until: row.until,
            created_at: row.created_at,
        }
    }
}

pub async fn list(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<ConnectorRequest>>, Problem> {
    let rows: Vec<Row> = if session.is_admin() {
        sqlx::query_as(select!("ORDER BY r.created_at DESC LIMIT 100"))
            .fetch_all(&state.db)
            .await?
    } else {
        sqlx::query_as(select!(
            "WHERE r.requester_id = $1 ORDER BY r.created_at DESC LIMIT 50"
        ))
        .bind(session.user_id)
        .fetch_all(&state.db)
        .await?
    };
    Ok(Json(rows.into_iter().map(ConnectorRequest::from).collect()))
}

#[derive(Deserialize)]
pub struct NewRequest {
    object: ObjectQuery,
    minutes: i32,
    reason: String,
}

/// A device behind a connector, as a request names it.
#[derive(sqlx::FromRow)]
struct Behind {
    id: Uuid,
    name: String,
    host: String,
    port: i32,
    connector_id: Option<Uuid>,
}

/// What an approval opens: remotehub's name, and the address and port.
fn target(device: &Behind) -> Option<RequestTarget> {
    let port = u16::try_from(device.port).ok().filter(|port| *port != 0)?;
    requests::host(&device.host).then(|| RequestTarget {
        name: readable(&device.name, protocol::MAX_NAME),
        host: device.host.clone(),
        port,
    })
}

pub async fn create(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<NewRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<serde_json::Value>), Problem> {
    let input = body(input)?;
    let asked = object(&input.object.kind, input.object.id)?;
    let (subject, catalog) = context(&state, &session).await?;
    let may_connect = |object: ObjectId| {
        catalog
            .effective_role(&subject, object)
            .is_some_and(|role| role >= Role::Connect)
    };
    match catalog.effective_role(&subject, asked) {
        None => return Err(Problem::new(ErrorCode::NotFound)),
        Some(role) if role < Role::Connect => return Err(Problem::new(ErrorCode::Forbidden)),
        Some(_) => {}
    }
    if !u32::try_from(input.minutes).is_ok_and(|minutes| protocol::MINUTES.contains(&minutes)) {
        return Err(invalid("minutes"));
    }
    let reason = input.reason.trim();
    if !plain(reason, protocol::MAX_REASON) {
        return Err(invalid("reason"));
    }

    // The targets are fixed now: the customer approves what they see.
    let (connector, object_name, devices): (Option<Uuid>, String, Vec<Behind>) = match asked {
        ObjectId::Device(id) => {
            let device: Behind = sqlx::query_as(
                "SELECT id, name, host, port,
                        device_connector(connector_mode, connector_id, folder_id) AS connector_id
                 FROM devices WHERE id = $1",
            )
            .bind(id)
            .fetch_one(&state.db)
            .await?;
            (device.connector_id, device.name.clone(), vec![device])
        }
        ObjectId::Folder(id) => {
            let (name, connector): (String, Option<Uuid>) =
                sqlx::query_as("SELECT name, folder_connector(id) FROM folders WHERE id = $1")
                    .bind(id)
                    .fetch_one(&state.db)
                    .await?;
            let devices: Vec<Behind> = sqlx::query_as(
                "WITH RECURSIVE below AS (
                     SELECT id FROM folders WHERE id = $1
                     UNION ALL
                     SELECT f.id FROM folders f JOIN below b ON f.parent_id = b.id
                 )
                 SELECT id, name, host, port,
                        device_connector(connector_mode, connector_id, folder_id) AS connector_id
                 FROM devices WHERE folder_id IN (SELECT id FROM below) ORDER BY lower(name)",
            )
            .bind(id)
            .fetch_all(&state.db)
            .await?;
            // Only the devices behind the folder's connector that the user
            // may connect to.
            let devices = devices
                .into_iter()
                .filter(|d| d.connector_id.is_some() && d.connector_id == connector)
                .filter(|d| may_connect(ObjectId::Device(d.id)))
                .collect();
            (connector, name, devices)
        }
        ObjectId::Credential(_) => return Err(invalid("kind")),
    };
    let connector = connector.ok_or_else(|| invalid("object"))?;
    let targets: Vec<RequestTarget> = devices.iter().filter_map(target).collect();
    if targets.is_empty() || targets.len() > protocol::MAX_TARGETS {
        return Err(invalid("object"));
    }
    let (folder, device) = match asked {
        ObjectId::Folder(id) => (Some(id), None),
        ObjectId::Device(id) => (None, Some(id)),
        ObjectId::Credential(_) => (None, None),
    };

    let mut tx = state.db.begin().await?;
    let waiting: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM connector_requests
                        WHERE requester_id = $1 AND status = 'pending'
                          AND created_at >= now() - interval '1 day'
                          AND folder_id IS NOT DISTINCT FROM $2
                          AND device_id IS NOT DISTINCT FROM $3)",
    )
    .bind(session.user_id)
    .bind(folder)
    .bind(device)
    .fetch_one(&mut *tx)
    .await?;
    if waiting {
        return Err(Problem::new(ErrorCode::RequestPending));
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO connector_requests (connector_id, folder_id, device_id, object_name, targets,
                                         requester_id, requester_name, minutes, reason)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id",
    )
    .bind(connector)
    .bind(folder)
    .bind(device)
    .bind(&object_name)
    .bind(sqlx::types::Json(&targets))
    .bind(session.user_id)
    .bind(&session.username)
    .bind(input.minutes)
    .bind(reason)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::ConnectorAccessRequested,
            asked,
            json!({ "request_id": id, "connector_id": connector, "minutes": input.minutes,
                    "reason": reason, "targets": targets }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub async fn withdraw(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    let mut tx = state.db.begin().await?;
    // Only the requester withdraws, and others do not learn the request exists.
    let found: Option<(String, Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
        "SELECT status, folder_id, device_id FROM connector_requests
         WHERE id = $1 AND requester_id = $2 FOR UPDATE",
    )
    .bind(id)
    .bind(session.user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, folder, device) = found.ok_or(Problem::new(ErrorCode::NotFound))?;
    if status != "pending" {
        return Err(Problem::new(ErrorCode::RequestDecided));
    }
    sqlx::query("UPDATE connector_requests SET status = 'cancelled' WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let details = json!({ "request_id": id });
    let record = match catalog::object_id(folder, device, None) {
        Some(object) => entry(
            &session,
            Action::ConnectorAccessWithdrawn,
            object,
            details,
            &address,
        ),
        None => Entry {
            actor: Actor {
                id: Some(session.user_id),
                name: &session.username,
            },
            action: Action::ConnectorAccessWithdrawn,
            object: None,
            details,
            address: Some(&address),
        },
    };
    audit::record(&mut *tx, record).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The requests waiting for the customer of `connector`, oldest first, as
/// many as fit into what the connector takes.
pub async fn pending(db: &PgPool, connector: Uuid) -> Result<Pending, sqlx::Error> {
    type Waiting = (
        Uuid,
        String,
        String,
        i32,
        sqlx::types::Json<Vec<RequestTarget>>,
    );
    let rows: Vec<Waiting> = sqlx::query_as(
        "SELECT id, requester_name, reason, minutes, targets FROM connector_requests
             WHERE connector_id = $1 AND status = 'pending'
               AND created_at >= now() - interval '1 day'
             ORDER BY created_at LIMIT $2",
    )
    .bind(connector)
    .bind(i64::try_from(protocol::MAX_REQUESTS).unwrap_or(i64::MAX))
    .fetch_all(db)
    .await?;
    let mut pending = Pending {
        requests: rows
            .into_iter()
            .map(
                |(id, requester, reason, minutes, targets)| CustomerRequest {
                    id,
                    requester: readable(&requester, protocol::MAX_NAME),
                    reason,
                    minutes: u32::try_from(minutes).unwrap_or_default(),
                    targets: targets.0,
                },
            )
            .collect(),
    };
    // The newest wait for the next report when the answer would be too long.
    while pending.requests.len() > 1
        && serde_json::to_vec(&pending).map_or(0, |bytes| bytes.len()) > protocol::MAX_PENDING_BYTES
    {
        pending.requests.pop();
    }
    Ok(pending)
}

/// Records the customer's answers of `connector`, named `name`, to requests
/// still pending; answers to others are left alone.
pub async fn answer(
    db: &PgPool,
    connector: Uuid,
    name: &str,
    answers: &[Answer],
) -> Result<(), sqlx::Error> {
    for answer in answers {
        let status = if answer.approved {
            "approved"
        } else {
            "refused"
        };
        let mut tx = db.begin().await?;
        let answered: Option<(Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
            "UPDATE connector_requests
             SET status = $3, answered_by = $4, answered_at = now(), until = $5::timestamptz
             WHERE id = $1 AND connector_id = $2 AND status = 'pending'
             RETURNING folder_id, device_id",
        )
        .bind(answer.id)
        .bind(connector)
        .bind(status)
        .bind(&answer.by)
        .bind(answer.until.as_deref())
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((folder, device)) = answered {
            let object = catalog::object_id(folder, device, None)
                .map(|object| (catalog::kind(object), catalog::id(object)))
                .unwrap_or(("connector", connector));
            audit::record(
                &mut *tx,
                Entry {
                    actor: Actor { id: None, name },
                    action: if answer.approved {
                        Action::ConnectorAccessApproved
                    } else {
                        Action::ConnectorAccessRefused
                    },
                    object: Some(object),
                    details: json!({ "request_id": answer.id, "connector_id": connector,
                                     "by": answer.by, "until": answer.until }),
                    address: None,
                },
            )
            .await?;
        }
        tx.commit().await?;
    }
    Ok(())
}
