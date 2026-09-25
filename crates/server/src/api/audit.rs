//! `/api/audit`: the audit log for administrators.

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::json;

use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry, Record, Verification};
use crate::session::Session;

#[derive(Deserialize)]
pub struct Page {
    before: Option<i64>,
    limit: Option<i64>,
}

/// Auditors (#106) and administrators read the log and check its chain.
fn require_auditor(session: &Session, state: &AppState) -> Result<(), Problem> {
    if session.is_auditor(&state.settings) {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::Forbidden))
    }
}

pub async fn list(
    State(state): State<AppState>,
    session: Session,
    page: Result<Query<Page>, QueryRejection>,
) -> Result<Json<Vec<Record>>, Problem> {
    require_auditor(&session, &state)?;
    let Query(page) = page.map_err(|_| Problem::new(ErrorCode::InvalidRequest))?;
    let records = audit::list(&state.db, page.before, page.limit.unwrap_or(100)).await?;
    Ok(Json(records))
}

/// Recomputes the hash chain; the check itself is audited.
pub async fn verify(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
) -> Result<Json<Verification>, Problem> {
    require_auditor(&session, &state)?;
    let verification = audit::verify(&state.db).await?;
    audit::record(
        &state.db,
        Entry {
            actor: Actor {
                id: Some(session.user_id),
                name: &session.username,
            },
            action: Action::AuditVerified,
            object: None,
            details: json!({
                "entries": verification.entries,
                "first_broken": verification.first_broken,
            }),
            address: Some(&address),
        },
    )
    .await?;
    if let Some(seq) = verification.first_broken {
        tracing::error!(seq, "the audit log's hash chain is broken");
    }
    Ok(Json(verification))
}
