//! HTTP API under `/api`.

mod health;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health::health))
        // Unknown API paths are a 404, never the SPA's index.html.
        .fallback(|| async { StatusCode::NOT_FOUND })
}
