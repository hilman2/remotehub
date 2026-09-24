//! Shared helpers for the server's HTTP tests.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use remotehub_directory::{AuthError, Identity, IdentityProvider, Sid};
use remotehub_server::config::SessionConfig;
use remotehub_server::{AppState, Settings};
use remotehub_vault::{DynVault, FileKeyring, Vault, generate_key_line};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use uuid::Uuid;

pub const ORIGIN: &str = "https://remotehub.test";

pub fn settings() -> Settings {
    Settings {
        public_origin: ORIGIN.to_owned(),
        session: SessionConfig {
            idle: Duration::from_secs(30 * 60),
            max: Duration::from_secs(12 * 3600),
        },
        admin_groups: vec![ADMINS_SID.to_owned()],
    }
}

/// A pool whose database does not exist, for behaviour without a database.
pub fn unreachable_pool() -> PgPool {
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(500))
        .connect_lazy("postgres://nobody:nothing@127.0.0.1:1/none")
        .unwrap()
}

/// A vault with a fresh random master key.
pub fn vault() -> DynVault {
    Vault::new(Box::new(FileKeyring::parse(&generate_key_line(1)).unwrap()))
}

pub fn state(db: PgPool) -> AppState {
    AppState::new(db, Some(Arc::new(FakeDirectory)), settings(), vault())
}

/// alice / right signs in (member of the admin group), bob / right signs in
/// without groups; carol is disabled;
/// "offline" makes the directory unreachable; everything else is wrong.
pub struct FakeDirectory;

pub const ALICE_SID: &str = "S-1-5-21-1-2-3-1105";
pub const ADMINS_SID: &str = "S-1-5-21-1-2-3-1201";
pub const BOB_SID: &str = "S-1-5-21-1-2-3-1106";

impl IdentityProvider for FakeDirectory {
    async fn authenticate(
        &self,
        username: &str,
        password: &SecretString,
    ) -> Result<Identity, AuthError> {
        match (username, password.expose_secret()) {
            ("alice", "right") => Ok(Identity {
                sid: ALICE_SID.parse().unwrap(),
                guid: Uuid::from_u128(0xa11ce),
                username: "alice".into(),
                upn: Some("alice@remotehub.test".into()),
                display_name: "Alice Admin".into(),
                email: None,
                groups: vec![ADMINS_SID.parse::<Sid>().unwrap()],
            }),
            ("bob", "right") => Ok(Identity {
                sid: BOB_SID.parse().unwrap(),
                guid: Uuid::from_u128(0xb0b),
                username: "bob".into(),
                upn: None,
                display_name: "Bob Helpdesk".into(),
                email: None,
                groups: vec![],
            }),
            ("carol", _) => Err(AuthError::AccountDisabled),
            ("offline", _) => Err(AuthError::Unavailable("connection refused".into())),
            _ => Err(AuthError::InvalidCredentials),
        }
    }
}

pub struct Response {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    /// The problem code of an error response.
    pub fn code(&self) -> String {
        self.json()["code"].as_str().unwrap_or_default().to_owned()
    }

    /// The session token from `Set-Cookie`, if one was set.
    pub fn session_token(&self) -> Option<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .filter_map(|v| {
                v.split(';')
                    .next()?
                    .strip_prefix("__Host-remotehub-session=")
            })
            .map(str::to_owned)
            .find(|t| !t.is_empty())
    }
}

pub async fn send(app: &Router, request: Request<Body>) -> Response {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    Response {
        status,
        headers,
        body,
    }
}

pub fn get(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut request = Request::get(uri);
    if let Some(token) = token {
        request = request.header(header::COOKIE, format!("__Host-remotehub-session={token}"));
    }
    request.body(Body::empty()).unwrap()
}

pub fn json(method: &str, uri: &str, body: Value, origin: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(origin) = origin {
        request = request.header(header::ORIGIN, origin);
    }
    request.body(Body::from(body.to_string())).unwrap()
}

pub fn sign_in_request(username: &str, password: &str) -> Request<Body> {
    json(
        "POST",
        "/api/session",
        serde_json::json!({ "username": username, "password": password }),
        Some(ORIGIN),
    )
}
