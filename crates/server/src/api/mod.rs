//! HTTP API under `/api`.

mod audit;
mod health;
mod origin;
pub mod problem;
pub mod session;

use axum::Router;
use axum::middleware;
use axum::routing::{get, post};

use crate::AppState;
use problem::{ErrorCode, Problem};

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/health", get(health::health))
        .route(
            "/session",
            get(session::current)
                .post(session::sign_in)
                .delete(session::sign_out),
        )
        .route("/session/break-glass", post(session::sign_in_break_glass))
        .route("/audit", get(audit::list))
        .route("/audit/verify", post(audit::verify))
        // Unknown API paths are a problem response, never the SPA's index.html.
        .fallback(|| async { Problem::new(ErrorCode::NotFound) })
        .layer(middleware::from_fn_with_state(state, origin::same_origin))
}
