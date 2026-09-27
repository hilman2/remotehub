//! Requests that change state must come from remotehub's own pages.
//!
//! Browsers send `Origin` with every such request; a foreign origin means a
//! page elsewhere tries to act with the user's cookie (CSRF). Together with
//! the SameSite=Strict cookie this closes that door. Clients without a
//! browser send no `Origin` and are not affected.
//!
//! The browser extension (#201) sends its own origin, `chrome-extension://…`.
//! Its routes under `/api/extension/` pass: they read no cookie, only the
//! extension's bearer token or the code it trades for one, so a page
//! elsewhere has nothing to borrow there.

use axum::extract::{Request, State};
use axum::http::header::ORIGIN;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::problem::{ErrorCode, Problem};
use crate::AppState;

pub async fn same_origin(State(state): State<AppState>, request: Request, next: Next) -> Response {
    // Nested under `/api`, the path here lacks that prefix.
    if !request.method().is_safe()
        && !request.uri().path().starts_with("/extension/")
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
