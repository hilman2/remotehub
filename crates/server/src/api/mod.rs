//! HTTP API under `/api`.

mod health;
mod origin;
pub mod problem;
mod session;

use axum::Router;
use axum::middleware;
use axum::routing::get;

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
        // Unknown API paths are a problem response, never the SPA's index.html.
        .fallback(|| async { Problem::new(ErrorCode::NotFound) })
        .layer(middleware::from_fn_with_state(state, origin::same_origin))
}
