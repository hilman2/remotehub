//! HTTP API under `/api`.

mod audit;
mod catalog;
mod connect;
mod connectors;
mod display;
pub mod health;
mod origin;
mod personal;
pub mod problem;
mod requests;
pub mod session;
mod terminal;

use axum::Router;
use axum::middleware;
use axum::routing::{delete, get, patch, post, put};

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
        .route("/tree", get(catalog::tree))
        .route("/folders", post(catalog::create_folder))
        .route(
            "/folders/{id}",
            patch(catalog::update_folder).delete(catalog::delete_folder),
        )
        .route("/devices", post(catalog::create_device))
        .route(
            "/devices/{id}",
            put(catalog::update_device).delete(catalog::delete_device),
        )
        .route("/ssh-ca.pub", get(connect::ssh_ca_public_key))
        .route("/devices/{id}/terminal", get(terminal::terminal))
        .route("/devices/{id}/display", get(display::display))
        .route("/devices/{id}/host-key", delete(catalog::reset_host_key))
        .route("/credentials", post(catalog::create_credential))
        .route(
            "/credentials/{id}",
            put(catalog::update_credential).delete(catalog::delete_credential),
        )
        .route(
            "/grants",
            get(catalog::list_grants).post(catalog::add_grant),
        )
        .route("/grants/{id}", delete(catalog::remove_grant))
        .route("/directory/principals", get(catalog::search_principals))
        .route(
            "/access-requests",
            get(requests::list).post(requests::create),
        )
        .route("/access-requests/{id}", delete(requests::cancel))
        .route("/access-requests/{id}/approve", post(requests::approve))
        .route("/access-requests/{id}/deny", post(requests::deny))
        .route(
            "/personal/vault",
            get(personal::vault).delete(personal::reset),
        )
        .route("/personal/unlocks", post(personal::add_unlock))
        .route("/personal/unlocks/{id}", delete(personal::remove_unlock))
        .route(
            "/personal/entries/{id}",
            put(personal::save_entry).delete(personal::delete_entry),
        )
        .route(
            "/connectors",
            get(connectors::list).post(connectors::create),
        )
        .route("/connectors/{id}", delete(connectors::delete))
        .route("/connectors/control", get(connectors::control))
        .route("/connectors/streams/{id}", get(connectors::stream))
        .route("/audit", get(audit::list))
        .route("/audit/verify", post(audit::verify))
        // Unknown API paths are a problem response, never the SPA's index.html.
        .fallback(|| async { Problem::new(ErrorCode::NotFound) })
        .layer(middleware::from_fn_with_state(state, origin::same_origin))
}
