//! remotehub server: API, sign-in, permissions, vault, protocol engines and
//! the web UI in one binary. See docs/architecture.md.

pub mod api;
pub mod audit;
pub mod auth;
pub mod break_glass;
pub mod catalog;
pub mod config;
pub mod connector_agent;
pub mod connectors;
pub mod db;
pub mod directory;
pub mod escrow;
pub mod kratos;
pub mod mail;
pub mod principal;
pub mod proxy;
pub mod refresh;
pub mod second_factor;
pub mod secrets;
pub mod session;
pub mod setup;
pub mod totp;
pub mod webauthn;

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
    /// The directory, set on the settings page (#144); none without one.
    pub directory: Arc<directory::Current>,
    pub settings: Arc<Settings>,
    pub limiter: Arc<SignInLimiter>,
    /// Seals and opens stored secrets (ADR 0004).
    pub vault: Arc<DynVault>,
    /// Site connectors that are online, and streams through them (ADR 0008).
    pub connectors: Arc<connectors::Connectors>,
    /// Whether the setup wizard is done (#143).
    pub setup: Arc<setup::Completion>,
}

/// Settings the request handlers need.
#[derive(Debug, Clone)]
pub struct Settings {
    /// e.g. `https://remotehub.example.com`
    pub public_origin: String,
    pub session: SessionConfig,
    /// Whether devices may be opened with the own directory account (ADR 0005).
    pub own_account_connections: bool,
    /// guacd for RDP and VNC (`host:port`).
    pub guacd: String,
    /// The browser service for HTTPS devices (`host:port`).
    pub browser: String,
    /// Keyboard layout of RDP sessions for devices without one of their own.
    pub rdp_keyboard_layout: String,
    /// Reverse proxies whose `X-Forwarded-For` names the client.
    pub trusted_proxies: Vec<proxy::Network>,
    /// The SSH CA for devices that sign in with a certificate; none without
    /// `REMOTEHUB_SSH_CA_KEY_FILE`.
    pub ssh_ca: Option<Arc<remotehub_gateway::ssh_ca::SshCa>>,
    /// Ory Kratos for local accounts (#103); none: they are off.
    pub kratos: Option<kratos::Kratos>,
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
            directory: Arc::new(directory::Current::new(directory)),
            settings: Arc::new(settings),
            limiter: Arc::new(SignInLimiter::new(Duration::from_secs(5 * 60))),
            vault: Arc::new(vault),
            connectors: Arc::default(),
            setup: Arc::default(),
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
