//! remotehub server: API, sign-in, permissions, vault, protocol engines and
//! the web UI in one binary. See docs/architecture.md.

pub mod api;
pub mod config;
pub mod db;

use std::path::Path;

use axum::Router;
use sqlx::PgPool;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

/// Version of this build; also the release version (workspace Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
}

/// The whole HTTP application: the API under `/api` and, if a built UI is
/// given, the SPA for every other path (unknown paths get index.html, the
/// SPA routes in the browser).
pub fn app(state: AppState, web_dir: Option<&Path>) -> Router {
    let mut app = Router::new().nest("/api", api::router());
    if let Some(dir) = web_dir {
        let spa = ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")));
        app = app.fallback_service(spa);
    }
    app.layer(TraceLayer::new_for_http()).with_state(state)
}
