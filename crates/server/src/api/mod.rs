//! HTTP API under `/api`.

mod accounts;
mod attachments;
mod audit;
mod catalog;
pub mod certificate;
mod connect;
mod connectors;
pub mod courier;
mod directory;
mod display;
mod fields;
mod groups;
pub mod health;
mod journal;
mod mail;
mod origin;
mod personal;
pub mod problem;
mod recovery;
mod reports;
mod requests;
mod reveal;
mod roles;
mod search;
mod second_factor;
pub mod session;
mod setup;
mod terminal;
mod users;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{any, delete, get, patch, post, put};

use crate::AppState;
use problem::{ErrorCode, Problem};

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/health", get(health::health))
        .route("/setup", get(setup::status))
        .route("/setup/code", post(setup::code))
        .route("/setup/administrator", post(setup::administrator))
        .route("/setup/step", put(setup::step))
        .route("/setup/break-glass", post(setup::create_break_glass))
        .route("/setup/complete", post(setup::complete))
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
        .route(
            "/account/second-factor",
            get(second_factor::status)
                .put(second_factor::enroll)
                .delete(second_factor::remove),
        )
        .route("/account/second-factor/offer", post(second_factor::offer))
        .route(
            "/account/security-keys/offer",
            post(second_factor::offer_key),
        )
        .route("/account/security-keys", post(second_factor::add_key))
        .route(
            "/account/security-keys/{id}",
            delete(second_factor::remove_key),
        )
        .route("/users/{id}/second-factor", delete(second_factor::reset))
        .route("/second-factor-principals", get(second_factor::rules))
        .route(
            "/second-factor-principals/{sid}",
            put(second_factor::require).delete(second_factor::waive),
        )
        .route(
            "/settings/directory",
            get(directory::get)
                .put(directory::save)
                .delete(directory::remove),
        )
        .route("/settings/directory/check", post(directory::check))
        .route(
            "/settings/mail",
            get(mail::get).put(mail::save).delete(mail::remove),
        )
        .route("/settings/mail/test", post(mail::test))
        .route(
            "/settings/certificate",
            get(certificate::status)
                .put(certificate::upload)
                .delete(certificate::reset)
                // Base64 of a PFX file or PEM text of a few kilobytes.
                .layer(DefaultBodyLimit::max(256 * 1024)),
        )
        .route("/roles", get(roles::list))
        .route(
            "/roles/{role}/members/{sid}",
            put(roles::assign).delete(roles::revoke),
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
        .route("/credentials/{id}/reveal", post(reveal::credential))
        .route("/credentials/{id}/versions", get(reveal::versions))
        .route(
            "/credentials/{id}/attachments",
            post(attachments::upload).layer(DefaultBodyLimit::max(attachments::MAX_SIZE + 1024)),
        )
        .route(
            "/credentials/{id}/attachments/{attachment}",
            get(attachments::download).delete(attachments::delete),
        )
        .route(
            "/personal/attachments/{id}",
            get(personal::attachment)
                .put(personal::save_attachment)
                .delete(personal::delete_attachment)
                // Base64 of a sealed file of up to 5 MiB.
                .layer(DefaultBodyLimit::max(8 * 1024 * 1024)),
        )
        .route("/devices/{id}/reveal", post(reveal::device))
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
        .route(
            "/recovery-keys",
            get(recovery::keys).post(recovery::create_key),
        )
        .route("/recovery-keys/{id}", delete(recovery::delete_key))
        .route(
            "/vault-recoveries",
            get(recovery::list).post(recovery::create),
        )
        .route("/vault-recoveries/{id}", delete(recovery::cancel))
        .route("/vault-recoveries/{id}/approve", post(recovery::approve))
        .route("/vault-recoveries/{id}/vault", get(recovery::vault))
        .route(
            "/vault-recoveries/{id}/attachments/{file}",
            get(recovery::attachment),
        )
        .route("/vault-recoveries/{id}/complete", post(recovery::complete))
        .route("/reports/people", get(reports::people))
        .route("/reports/folders", get(reports::folders))
        .route("/reports/users/{id}", get(reports::user))
        .route("/reports/folders/{id}", get(reports::folder))
        .route("/audit", get(audit::list))
        .route("/audit/verify", post(audit::verify))
        // Unknown API paths are a problem response, never the SPA's index.html.
        // Only routes that exist wait for setup: an unknown path stays a 404.
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::setup::gate,
        ))
        .fallback(|| async { Problem::new(ErrorCode::NotFound) })
        .layer(middleware::from_fn_with_state(state, origin::same_origin))
}
