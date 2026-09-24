//! remotehub server: API, sign-in, permissions, vault, protocol engines and
//! the web UI in one binary. See docs/architecture.md.

pub mod api;
pub mod audit;
pub mod auth;
pub mod break_glass;
pub mod catalog;
pub mod config;
pub mod db;
pub mod proxy;
pub mod secrets;
pub mod session;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use sqlx::PgPool;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use auth::{Authenticator, SignInLimiter};
use config::SessionConfig;
use remotehub_vault::DynVault;

/// Version of this build; also the release version (workspace Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    /// The configured directory; `None` if only break-glass accounts exist.
    pub directory: Option<Arc<dyn Authenticator>>,
    pub settings: Arc<Settings>,
    pub limiter: Arc<SignInLimiter>,
    /// Seals and opens stored secrets (ADR 0004).
    pub vault: Arc<DynVault>,
}

/// Settings the request handlers need.
#[derive(Debug, Clone)]
pub struct Settings {
    /// e.g. `https://remotehub.example.com`
    pub public_origin: String,
    pub session: SessionConfig,
    /// SIDs of the groups whose members are administrators.
    pub admin_groups: Vec<String>,
    /// Whether devices may be opened with the own directory account (ADR 0005).
    pub own_account_connections: bool,
    /// guacd for RDP and VNC (`host:port`).
    pub guacd: String,
    /// Keyboard layout of RDP sessions for devices without one of their own.
    pub rdp_keyboard_layout: String,
    /// Reverse proxies whose `X-Forwarded-For` names the client.
    pub trusted_proxies: Vec<proxy::Network>,
}

impl AppState {
    pub fn new(
        db: PgPool,
        directory: Option<Arc<dyn Authenticator>>,
        settings: Settings,
        vault: DynVault,
    ) -> Self {
        AppState {
            db,
            directory,
            settings: Arc::new(settings),
            limiter: Arc::new(SignInLimiter::new(Duration::from_secs(5 * 60))),
            vault: Arc::new(vault),
        }
    }
}

/// The whole HTTP application: the API under `/api` and, if a built UI is
/// given, the SPA for every other path (unknown paths get index.html, the
/// SPA routes in the browser).
pub fn app(state: AppState, web_dir: Option<&Path>) -> Router {
    let mut app = Router::new().nest("/api", api::router(state.clone()));
    if let Some(dir) = web_dir {
        let spa = ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")));
        app = app.fallback_service(spa);
    }
    app.layer(TraceLayer::new_for_http()).with_state(state)
}
