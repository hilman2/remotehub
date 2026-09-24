//! Just-in-time access (#20): a user asks for `connect` or `reveal` on an
//! object for a while, with a reason. Someone who manages the object, and
//! is not the requester, approves or denies. An approval becomes a grant
//! that runs out by itself (`grants.expires_at`); `authorize()` stops
//! counting it then. Every step is audited.
//!
//! - `GET /api/access-requests`: the caller's own requests and the pending
//!   ones they may decide
//! - `POST /api/access-requests`: ask
//! - `POST /api/access-requests/{id}/approve`, `…/deny`: decide
//! - `DELETE /api/access-requests/{id}`: take back a pending request

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use remotehub_model::{ObjectId, Role};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::catalog::{ObjectQuery, body, context, entry, invalid, object};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::audit::{self, Action};
use crate::session::Session;
use crate::{AppState, catalog};

/// How long access may be asked for, in minutes: a quarter of an hour to a
/// day. Longer needs are for permanent grants.
const MINUTES: std::ops::RangeInclusive<i32> = 15..=1440;

/// Requests with their object's name, then `$filter` (a literal, so the
/// query stays a static string). Timestamps as the UI reads them: RFC 3339
/// in UTC.
macro_rules! select {
    ($filter:literal) => {
        concat!(
            "SELECT r.id, r.folder_id, r.device_id, r.credential_id,
                    COALESCE(f.name, d.name, c.name, '') AS object_name,
                    r.requester_id, r.requester_name, r.role, r.minutes, r.reason, r.status,
                    r.decider_name,
                    to_char(r.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS created_at,
                    to_char(r.decided_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS decided_at,
                    to_char(r.expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS expires_at
             FROM access_requests r
             LEFT JOIN folders f ON f.id = r.folder_id
             LEFT JOIN devices d ON d.id = r.device_id
             LEFT JOIN credentials c ON c.id = r.credential_id ",
            $filter
        )
    };
}

#[derive(sqlx::FromRow)]
struct RequestRow {
    id: Uuid,
    folder_id: Option<Uuid>,
    device_id: Option<Uuid>,
    credential_id: Option<Uuid>,
    object_name: String,
    requester_id: Uuid,
    requester_name: String,
    role: String,
    minutes: i32,
    reason: String,
    status: String,
    decider_name: Option<String>,
    created_at: String,
    decided_at: Option<String>,
    expires_at: Option<String>,
}

#[derive(Serialize)]
pub struct AccessRequest {
    id: Uuid,
    object: ObjectId,
    object_name: String,
    requester_name: String,
    role: String,
    minutes: i32,
    reason: String,
    /// `pending`, `approved`, `denied` or `cancelled`.
    status: String,
    decider_name: Option<String>,
    created_at: String,
    decided_at: Option<String>,
    /// When an approved request's grant runs out.
    expires_at: Option<String>,
}

impl RequestRow {
    fn object(&self) -> Option<ObjectId> {
        catalog::object_id(self.folder_id, self.device_id, self.credential_id)
    }

    fn view(self) -> Option<AccessRequest> {
        Some(AccessRequest {
            object: self.object()?,
            id: self.id,
            object_name: self.object_name,
            requester_name: self.requester_name,
            role: self.role,
            minutes: self.minutes,
            reason: self.reason,
            status: self.status,
            decider_name: self.decider_name,
            created_at: self.created_at,
            decided_at: self.decided_at,
            expires_at: self.expires_at,
        })
    }
}

#[derive(Serialize)]
pub struct Requests {
    /// The caller's own requests, newest first.
    mine: Vec<AccessRequest>,
    /// Pending requests of others that the caller may decide, oldest first.
    to_decide: Vec<AccessRequest>,
}

pub async fn list(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Requests>, Problem> {
    let (subject, catalog) = context(&state, &session).await?;
    let mine: Vec<RequestRow> = sqlx::query_as(select!(
        "WHERE r.requester_id = $1 ORDER BY r.created_at DESC LIMIT 50"
    ))
    .bind(session.user_id)
    .fetch_all(&state.db)
    .await?;
    let pending: Vec<RequestRow> =
        sqlx::query_as(select!("WHERE r.status = 'pending' ORDER BY r.created_at"))
            .fetch_all(&state.db)
            .await?;
    let to_decide = pending
        .into_iter()
        .filter(|row| {
            row.requester_id != session.user_id
                && row
                    .object()
                    .is_some_and(|object| catalog.may_approve(&subject, object))
        })
        .filter_map(RequestRow::view)
        .collect();
    Ok(Json(Requests {
        mine: mine.into_iter().filter_map(RequestRow::view).collect(),
        to_decide,
    }))
}

#[derive(Deserialize)]
pub struct NewRequest {
    object: ObjectQuery,
    role: String,
    minutes: i32,
    reason: String,
}

pub async fn create(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<NewRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<serde_json::Value>), Problem> {
    let input = body(input)?;
    let target = object(&input.object.kind, input.object.id)?;
    // The grant goes to the requester's own SID; break-glass accounts have
    // none, and administer everything anyway.
    let sid = session
        .sid
        .clone()
        .ok_or(Problem::new(ErrorCode::Forbidden))?;
    let (subject, catalog) = context(&state, &session).await?;
    if catalog.effective_role(&subject, target).is_none() {
        return Err(Problem::new(ErrorCode::NotFound));
    }
    let role = Role::parse(&input.role).ok_or_else(|| invalid("role"))?;
    if !catalog.may_request(&subject, role, target) {
        return Err(invalid("role"));
    }
    if !MINUTES.contains(&input.minutes) {
        return Err(invalid("minutes"));
    }
    let reason = input.reason.trim();
    if reason.is_empty() || reason.chars().count() > 500 {
        return Err(invalid("reason"));
    }
    let (folder, device, credential) = columns(target);

    let mut tx = state.db.begin().await?;
    let waiting: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM access_requests
                        WHERE requester_id = $1 AND status = 'pending' AND role = $2
                          AND folder_id IS NOT DISTINCT FROM $3
                          AND device_id IS NOT DISTINCT FROM $4
                          AND credential_id IS NOT DISTINCT FROM $5)",
    )
    .bind(session.user_id)
    .bind(role.as_str())
    .bind(folder)
    .bind(device)
    .bind(credential)
    .fetch_one(&mut *tx)
    .await?;
    if waiting {
        return Err(Problem::new(ErrorCode::RequestPending));
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO access_requests (folder_id, device_id, credential_id, requester_id,
                                      requester_sid, requester_name, role, minutes, reason)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id",
    )
    .bind(folder)
    .bind(device)
    .bind(credential)
    .bind(session.user_id)
    .bind(&sid)
    .bind(&session.username)
    .bind(role.as_str())
    .bind(input.minutes)
    .bind(reason)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::AccessRequested,
            target,
            json!({ "request_id": id, "role": role.as_str(), "minutes": input.minutes, "reason": reason }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

fn columns(object: ObjectId) -> (Option<Uuid>, Option<Uuid>, Option<Uuid>) {
    match object {
        ObjectId::Folder(id) => (Some(id), None, None),
        ObjectId::Device(id) => (None, Some(id), None),
        ObjectId::Credential(id) => (None, None, Some(id)),
    }
}

#[derive(sqlx::FromRow)]
struct Pending {
    folder_id: Option<Uuid>,
    device_id: Option<Uuid>,
    credential_id: Option<Uuid>,
    requester_id: Uuid,
    requester_sid: String,
    requester_name: String,
    role: String,
    minutes: i32,
    status: String,
}

enum Decision {
    Approve,
    Deny,
}

pub async fn approve(
    state: State<AppState>,
    session: Session,
    address: ClientAddress,
    id: Path<Uuid>,
) -> Result<StatusCode, Problem> {
    decide(state, session, address, id, Decision::Approve).await
}

pub async fn deny(
    state: State<AppState>,
    session: Session,
    address: ClientAddress,
    id: Path<Uuid>,
) -> Result<StatusCode, Problem> {
    decide(state, session, address, id, Decision::Deny).await
}

async fn decide(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    decision: Decision,
) -> Result<StatusCode, Problem> {
    let (subject, catalog) = context(&state, &session).await?;
    let mut tx = state.db.begin().await?;
    let request: Pending = sqlx::query_as(
        "SELECT folder_id, device_id, credential_id, requester_id, requester_sid,
                requester_name, role, minutes, status
         FROM access_requests WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Problem::new(ErrorCode::NotFound))?;
    let target = catalog::object_id(request.folder_id, request.device_id, request.credential_id)
        .ok_or(Problem::new(ErrorCode::NotFound))?;
    if !catalog.may_approve(&subject, target) {
        return Err(Problem::new(
            match catalog.effective_role(&subject, target) {
                Some(_) => ErrorCode::Forbidden,
                None => ErrorCode::NotFound,
            },
        ));
    }
    // Four eyes: not even an administrator decides their own request.
    if request.requester_id == session.user_id {
        return Err(Problem::new(ErrorCode::OwnRequest));
    }
    if request.status != "pending" {
        return Err(Problem::new(ErrorCode::RequestDecided));
    }

    let (status, action) = match decision {
        Decision::Approve => ("approved", Action::AccessApproved),
        Decision::Deny => ("denied", Action::AccessDenied),
    };
    let expires_at: Option<String> = sqlx::query_scalar(
        "UPDATE access_requests
         SET status = $2, decided_by = $3, decider_name = $4, decided_at = now(),
             expires_at = CASE WHEN $2 = 'approved' THEN now() + minutes * interval '1 minute' END
         WHERE id = $1
         RETURNING to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')",
    )
    .bind(id)
    .bind(status)
    .bind(session.user_id)
    .bind(&session.username)
    .fetch_one(&mut *tx)
    .await?;
    let mut details = json!({
        "request_id": id, "requester": request.requester_name,
        "role": request.role, "minutes": request.minutes,
    });
    if let Decision::Approve = decision {
        let (folder, device, credential) = columns(target);
        let grant: Uuid = sqlx::query_scalar(
            "INSERT INTO grants (folder_id, device_id, credential_id, principal_kind, principal_sid,
                                 principal_name, role, created_by, expires_at, request_id)
             SELECT $1, $2, $3, 'user', $4, $5, $6, $7, expires_at, id
             FROM access_requests WHERE id = $8
             RETURNING id",
        )
        .bind(folder)
        .bind(device)
        .bind(credential)
        .bind(&request.requester_sid)
        .bind(&request.requester_name)
        .bind(&request.role)
        .bind(session.user_id)
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        details["grant_id"] = json!(grant);
        details["expires_at"] = json!(expires_at);
    }
    audit::record(&mut *tx, entry(&session, action, target, details, &address)).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The requester takes back a request that is still pending.
pub async fn cancel(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    let mut tx = state.db.begin().await?;
    let request = sqlx::query_as::<_, Pending>(
        "SELECT folder_id, device_id, credential_id, requester_id, requester_sid,
                requester_name, role, minutes, status
         FROM access_requests WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .filter(|request| request.requester_id == session.user_id)
    .ok_or(Problem::new(ErrorCode::NotFound))?;
    if request.status != "pending" {
        return Err(Problem::new(ErrorCode::RequestDecided));
    }
    let target = catalog::object_id(request.folder_id, request.device_id, request.credential_id)
        .ok_or(Problem::new(ErrorCode::NotFound))?;
    sqlx::query(
        "UPDATE access_requests SET status = 'cancelled', decided_at = now() WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::AccessCancelled,
            target,
            json!({ "request_id": id, "role": request.role }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
