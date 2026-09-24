//! Requests that change state must come from remotehub's own pages.
//!
//! Browsers send `Origin` with every such request; a foreign origin means a
//! page elsewhere tries to act with the user's cookie (CSRF). Together with
//! the SameSite=Strict cookie this closes that door. Clients without a
//! browser send no `Origin` and are not affected.

use axum::extract::{Request, State};
use axum::http::header::ORIGIN;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::problem::{ErrorCode, Problem};
use crate::AppState;

pub async fn same_origin(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if !request.method().is_safe()
        && let Some(origin) = request.headers().get(ORIGIN)
        && !origin
            .to_str()
            .is_ok_and(|o| o.eq_ignore_ascii_case(&state.settings.public_origin))
    {
        tracing::warn!(origin = ?origin, path = %request.uri().path(), "request from a foreign origin");
        return Problem::new(ErrorCode::ForbiddenOrigin).into_response();
    }
    next.run(request).await
}
