//! HTTP API under `/api`.

mod health;
pub mod problem;

use axum::Router;
use axum::routing::get;

use crate::AppState;
use problem::{ErrorCode, Problem};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health::health))
        // Unknown API paths are a problem response, never the SPA's index.html.
        .fallback(|| async { Problem::new(ErrorCode::NotFound) })
}
