//! HTTP API under `/api`.

mod accounts;
mod audit;
mod catalog;
mod connect;
mod connectors;
mod display;
mod groups;
pub mod health;
mod journal;
mod origin;
mod personal;
pub mod problem;
mod requests;
mod search;
pub mod session;
mod terminal;
mod users;

use axum::Router;
use axum::middleware;
use axum::routing::{any, delete, get, patch, post, put};

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
        .route("/session/local", post(accounts::sign_in))
        .route("/session/methods", get(accounts::methods))
        .route("/auth/{*path}", any(accounts::forward))
        .route("/users", get(users::list))
        .route("/users/invite", post(users::invite))
        .route("/users/{id}", delete(users::delete))
        .route(
            "/users/{id}/block",
            post(users::block).delete(users::unblock),
        )
        .route("/users/{id}/sessions", delete(users::end))
        .route("/users/{id}/recovery", post(users::recovery))
        .route("/groups", get(groups::list).post(groups::create))
        .route("/groups/{id}", patch(groups::update).delete(groups::delete))
        .route(
            "/groups/{id}/members/{sid}",
            put(groups::add_member).delete(groups::remove_member),
        )
        .route("/tree", get(catalog::tree))
        .route("/folders", post(catalog::create_folder))
        .route(
            "/folders/{id}",
            patch(catalog::update_folder).delete(catalog::delete_folder),
        )
        .route("/folders/{id}/open", put(catalog::set_folder_open))
        .route("/devices", post(catalog::create_device))
        .route(
            "/devices/{id}",
            put(catalog::update_device).delete(catalog::delete_device),
        )
        .route("/ssh-ca.pub", get(connect::ssh_ca_public_key))
        .route("/devices/{id}/terminal", get(terminal::terminal))
        .route("/devices/{id}/display", get(display::display))
        .route("/devices/{id}/host-key", delete(catalog::reset_host_key))
        .route(
            "/devices/{id}/journal",
            get(journal::journal).post(journal::add_note),
        )
        .route("/purpose-principals", get(journal::purpose_principals))
        .route(
            "/purpose-principals/{sid}",
            put(journal::require_purpose).delete(journal::waive_purpose),
        )
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
        .route("/search/picks", get(search::picks).post(search::pick))
        .route("/personal/search", put(personal::save_search))
        .route("/audit", get(audit::list))
        .route("/audit/verify", post(audit::verify))
        // Unknown API paths are a problem response, never the SPA's index.html.
        .fallback(|| async { Problem::new(ErrorCode::NotFound) })
        .layer(middleware::from_fn_with_state(state, origin::same_origin))
}
