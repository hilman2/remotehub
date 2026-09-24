//! `GET /api/health`: liveness, version and database reachability. Used by
//! the container health check, the update script and the UI footer.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;

use crate::{AppState, VERSION, db};

#[derive(Debug, Serialize)]
pub struct Health {
    pub status: Status,
    pub version: &'static str,
    pub database: Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Unavailable,
}

pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<Health>) {
    let database = if db::is_reachable(&state.db).await {
        Status::Ok
    } else {
        Status::Unavailable
    };
    let code = match database {
        Status::Ok => StatusCode::OK,
        Status::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
    };
    (
        code,
        Json(Health {
            status: database,
            version: VERSION,
            database,
        }),
    )
}
