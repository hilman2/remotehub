//! Shared helpers for the server's tests.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use remotehub_directory::laps::{LapsError, LapsPassword};
use remotehub_directory::{
    AuthError, Group, Identity, IdentityProvider, Principal, PrincipalKind, Sid,
};
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
        own_account_connections: true,
        guacd: std::env::var("REMOTEHUB_TEST_GUACD").unwrap_or_else(|_| "guacd:4822".to_owned()),
        browser: std::env::var("REMOTEHUB_TEST_BROWSER")
            .unwrap_or_else(|_| "browser:4823".to_owned()),
        rdp_keyboard_layout: "en-us-qwerty".to_owned(),
        trusted_proxies: Vec::new(),
        ssh_ca: None,
        kratos: None,
        caddy: None,
        downloads: None,
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
/// without groups, olaf / right is in the operators group; carol is disabled;
/// "offline" makes the directory unreachable; everything else is wrong.
pub struct FakeDirectory;

pub const ALICE_SID: &str = "S-1-5-21-1-2-3-1105";
pub const ADMINS_SID: &str = "S-1-5-21-1-2-3-1201";
pub const BOB_SID: &str = "S-1-5-21-1-2-3-1106";
pub const OPS_SID: &str = "S-1-5-21-1-2-3-1202";
pub const OLAF_SID: &str = "S-1-5-21-1-2-3-1107";

impl IdentityProvider for FakeDirectory {
    async fn search(&self, query: &str, _limit: i32) -> Result<Vec<Principal>, AuthError> {
        let all = [
            (PrincipalKind::Group, ADMINS_SID, "RH Admins"),
            (PrincipalKind::Group, OPS_SID, "RH Operators"),
            (PrincipalKind::User, ALICE_SID, "Alice Admin"),
            (PrincipalKind::User, BOB_SID, "Bob Helpdesk"),
        ];
        Ok(all
            .into_iter()
            .filter(|(_, _, name)| name.to_lowercase().contains(&query.to_lowercase()))
            .map(|(kind, sid, name)| Principal {
                kind,
                sid: sid.parse().unwrap(),
                name: name.to_owned(),
                detail: None,
            })
            .collect())
    }

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
            ("olaf", "right") => Ok(Identity {
                sid: OLAF_SID.parse().unwrap(),
                guid: Uuid::from_u128(0x01af),
                username: "olaf".into(),
                upn: None,
                display_name: "Olaf Operator".into(),
                email: None,
                groups: vec![OPS_SID.parse::<Sid>().unwrap()],
            }),
            // Same name and password as the lab's SSH target, for "own account" tests.
            ("tester", "Tester-Passw0rd!") => Ok(Identity {
                sid: "S-1-5-21-1-2-3-1108".parse().unwrap(),
                guid: Uuid::from_u128(0x7e57),
                username: "tester".into(),
                upn: None,
                display_name: "Tester".into(),
                email: None,
                groups: vec![],
            }),
            ("carol", _) => Err(AuthError::AccountDisabled),
            ("offline", _) => Err(AuthError::Unavailable("connection refused".into())),
            _ => Err(AuthError::InvalidCredentials),
        }
    }

    /// The directory's two groups; other SIDs it does not know.
    async fn group_names(&self, sids: &[Sid]) -> Result<Vec<Group>, AuthError> {
        Ok(sids
            .iter()
            .filter_map(|sid| {
                let name = match sid.as_str() {
                    ADMINS_SID => "RH Admins",
                    OPS_SID => "RH Operators",
                    _ => return None,
                };
                Some(Group {
                    sid: sid.clone(),
                    name: name.into(),
                })
            })
            .collect())
    }

    /// Everyone keeps the groups they sign in with.
    async fn refresh(&self, sid: &Sid) -> Result<Vec<Sid>, AuthError> {
        let groups: &[&str] = match sid.as_str() {
            ALICE_SID => &[ADMINS_SID],
            OLAF_SID => &[OPS_SID],
            _ => &[],
        };
        Ok(groups.iter().map(|g| g.parse().unwrap()).collect())
    }

    /// The lab's SSH and desktop targets have LAPS passwords, as in the lab's
    /// directory (deploy/testlab/dc/users.sh); "nolaps" has none; "offline"
    /// makes the directory unreachable.
    async fn laps_password(&self, host: &str) -> Result<LapsPassword, LapsError> {
        match host {
            "ssh-target" | "desktop-target" => Ok(LapsPassword {
                account: "tester".into(),
                password: SecretString::from("Tester-Passw0rd!"),
                computer: host.to_uppercase(),
            }),
            "nolaps" => Err(LapsError::NoPassword(host.into())),
            "offline" => Err(AuthError::Unavailable("connection refused".into()).into()),
            _ => Err(LapsError::ComputerNotFound(host.into())),
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

/// A request as a signed-in browser sends it: session cookie, own origin,
/// JSON body if given.
pub fn authed(method: &str, uri: &str, body: Option<Value>, token: &str) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, format!("__Host-remotehub-session={token}"))
        .header(header::ORIGIN, ORIGIN);
    if body.is_some() {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    request
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap()
}
