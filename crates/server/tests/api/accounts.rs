//! Local accounts through Ory Kratos (#103), against a stand-in for Kratos:
//! its answers depend on the cookie the browser sends, and it notes every
//! call to its admin API.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{any, get, post};
use remotehub_server::config::KratosConfig;
use remotehub_server::kratos::Kratos;
use remotehub_server::{AppState, app};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{FakeDirectory, ORIGIN, get as get_request, send, settings, vault};

pub const ADA: &str = "5b0c3cb1-7a0e-4a5e-9d0a-0000000000a1";
const ADMIN: &str = "5b0c3cb1-7a0e-4a5e-9d0a-0000000000a2";
/// The identity the stand-in creates for an invitation.
pub const INVITED: &str = "5b0c3cb1-7a0e-4a5e-9d0a-0000000000a3";
const KRATOS_SESSION: &str = "0f0e0d0c-0b0a-4908-8706-050403020100";
/// Kratos already has an account with this address.
pub const TAKEN: &str = "taken@example.com";

/// The calls to the stand-in's admin API, as `METHOD /path`; a change of
/// state adds the new state.
pub type Calls = Arc<Mutex<Vec<String>>>;

fn session(identity: &str, email: &str, aal: &str, state: &str) -> Value {
    json!({
        "id": KRATOS_SESSION,
        "active": true,
        "authenticator_assurance_level": aal,
        "identity": { "id": identity, "state": state, "traits": { "email": email, "name": "Ada" } },
    })
}

async fn whoami(headers: HeaderMap) -> impl IntoResponse {
    let cookie = headers
        .get(header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .unwrap_or_default();
    let case = cookie
        .split(';')
        .filter_map(|pair| pair.trim().strip_prefix("kratos="))
        .next()
        .unwrap_or("none");
    let (status, body) = match case {
        "pending" => (
            StatusCode::FORBIDDEN,
            json!({ "error": { "id": "session_aal2_required" } }),
        ),
        "aal1" => (
            StatusCode::OK,
            session(ADA, "ada@example.com", "aal1", "active"),
        ),
        "aal2" => (
            StatusCode::OK,
            session(ADA, "Ada@Example.com", "aal2", "active"),
        ),
        "admin" => (
            StatusCode::OK,
            session(ADMIN, "admin@example.com", "aal2", "active"),
        ),
        "inactive" => (
            StatusCode::OK,
            session(ADA, "ada@example.com", "aal2", "inactive"),
        ),
        _ => (
            StatusCode::UNAUTHORIZED,
            json!({ "error": { "id": "session_inactive" } }),
        ),
    };
    (status, axum::Json(body)).into_response()
}

/// Echoes what reached it, and sets a cookie like Kratos' CSRF cookie.
async fn login_flow(headers: HeaderMap, RawQuery(query): RawQuery) -> impl IntoResponse {
    let seen = |name: header::HeaderName| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    (
        [(header::SET_COOKIE, "csrf_token_abc=token; Path=/; HttpOnly")],
        axum::Json(json!({
            "query": query,
            "cookie": seen(header::COOKIE),
            "authorization": seen(header::AUTHORIZATION),
        })),
    )
}

/// The admin API: notes the call and answers as Kratos does. An account
/// never has a WebAuthn credential.
async fn admin(State(calls): State<Calls>, request: Request<Body>) -> impl IntoResponse {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let body = axum::body::to_bytes(request.into_body(), 1 << 16)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let call = match body[0]["path"].as_str() {
        Some("/state") => format!("{method} {path} {}", body[0]["value"].as_str().unwrap()),
        _ => format!("{method} {path}"),
    };
    calls.lock().unwrap().push(call);
    let error = || axum::Json(json!({ "error": { "code": 0 } }));
    match (method, path.as_str()) {
        (Method::POST, "/admin/identities") if body["traits"]["email"] == TAKEN => {
            (StatusCode::CONFLICT, error()).into_response()
        }
        (Method::POST, "/admin/identities") => {
            (StatusCode::CREATED, axum::Json(json!({ "id": INVITED }))).into_response()
        }
        (Method::POST, "/admin/recovery/code") => (
            StatusCode::CREATED,
            axum::Json(json!({
                "recovery_link": "https://remotehub.test/sign-in/recovery?flow=f",
                "recovery_code": "123456",
                "expires_at": "2026-09-27T00:00:00Z",
            })),
        )
            .into_response(),
        (Method::PATCH, _) => axum::Json(json!({})).into_response(),
        (Method::DELETE, path) if path.ends_with("/credentials/webauthn") => {
            (StatusCode::NOT_FOUND, error()).into_response()
        }
        (Method::DELETE, _) => StatusCode::NO_CONTENT.into_response(),
        _ => (StatusCode::NOT_FOUND, error()).into_response(),
    }
}

async fn stand_in() -> (String, Calls) {
    let calls = Calls::default();
    let router = Router::new()
        .route("/sessions/whoami", get(whoami))
        .route("/self-service/login/browser", get(login_flow))
        // Every password is wrong.
        .route(
            "/self-service/login",
            post(|| async { (StatusCode::BAD_REQUEST, axum::Json(json!({ "ui": {} }))) }),
        )
        .route(
            "/self-service/login/api",
            get(|| async {
                axum::Json(json!({ "ui": { "nodes": [
                    { "group": "oidc", "attributes": { "name": "provider", "value": "entra" },
                      "meta": { "label": { "context": { "provider": "Microsoft" } } } },
                ] } }))
            }),
        )
        .route("/health/alive", get(|| async { "ok" }))
        .route("/admin/{*path}", any(admin))
        // Registration answers as if open, under any spelling of its path:
        // remotehub must not let a browser get there.
        .fallback(|uri: axum::http::Uri| async move {
            if uri.path().contains("registration") {
                (StatusCode::OK, axum::Json(json!({ "ui": {} })))
            } else {
                (StatusCode::NOT_FOUND, axum::Json(json!({})))
            }
        })
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}"), calls)
}

/// remotehub with the stand-in for Kratos and the fake directory.
pub async fn setup(pool: PgPool) -> (Router, Calls) {
    let (url, calls) = stand_in().await;
    let mut settings = settings();
    settings.kratos = Some(Kratos::new(&KratosConfig {
        public_url: url.clone(),
        admin_url: url,
    }));
    settings.admin_accounts = vec!["admin@example.com".to_owned()];
    let state = AppState::new(pool, Some(Arc::new(FakeDirectory)), settings, vault());
    (app(state, None), calls)
}

/// `POST /api/session/local` from a browser whose Kratos cookie is `case`:
/// `aal2` signs Ada in, `admin` the local administrator.
pub fn sign_in(case: &str) -> Request<Body> {
    Request::post("/api/session/local")
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, format!("kratos={case}"))
        .body(Body::empty())
        .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_kratos_session_counts_only_with_its_second_factor(pool: PgPool) {
    let (app, _) = setup(pool.clone()).await;
    for (case, status, code) in [
        ("none", StatusCode::UNAUTHORIZED, "unauthenticated"),
        ("pending", StatusCode::FORBIDDEN, "second_factor_required"),
        (
            "aal1",
            StatusCode::FORBIDDEN,
            "second_factor_setup_required",
        ),
        ("inactive", StatusCode::FORBIDDEN, "account_disabled"),
    ] {
        let response = send(&app, sign_in(case)).await;
        assert_eq!(
            (response.status, response.code().as_str()),
            (status, code),
            "{case}"
        );
        assert!(response.session_token().is_none(), "{case}");
    }

    let response = send(&app, sign_in("aal2")).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.json(),
        json!({
            "username": "ada@example.com", "display_name": "Ada", "kind": "local", "admin": false,
            "roles": [],
        })
    );
    let token = response.session_token().unwrap();
    assert_eq!(
        send(&app, get_request("/api/session", Some(&token)))
            .await
            .json()["kind"],
        "local"
    );
    // The same identity signs in as the same user.
    send(&app, sign_in("aal2")).await;
    let admin = send(&app, sign_in("admin")).await;
    assert_eq!(admin.json()["admin"], true);
    let users: Vec<(String, String)> = sqlx::query_as(
        "SELECT identity_id::text, username FROM users WHERE kind = 'local' ORDER BY username",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        users,
        [
            (ADA.to_owned(), "ada@example.com".to_owned()),
            (ADMIN.to_owned(), "admin@example.com".to_owned())
        ]
    );

    let log = send(
        &app,
        get_request("/api/audit", admin.session_token().as_deref()),
    )
    .await
    .json();
    let sign_ins = log
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == "session.sign_in" && e["details"]["kind"] == "local")
        .count();
    assert_eq!(sign_ins, 3, "{log}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_proxy_passes_only_the_self_service_flows(pool: PgPool) {
    let (app, _) = setup(pool.clone()).await;
    let request = Request::get("/api/auth/self-service/login/browser?aal=aal2")
        .header(header::COOKIE, "kratos=aal2")
        .header(header::AUTHORIZATION, "Bearer secret")
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())
        .unwrap();
    let response = send(&app, request).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.json(),
        json!({ "query": "aal=aal2", "cookie": "kratos=aal2", "authorization": null })
    );
    assert!(
        response.headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .starts_with("csrf_token_abc=")
    );
    for hidden in [
        "/api/auth/health/alive",
        "/api/auth/admin/sessions/x",
        // Registration takes a password; accounts come from invitations.
        "/api/auth/self-service/registration/browser",
        "/api/auth/self-service//registration/browser",
    ] {
        assert_eq!(
            send(&app, get_request(hidden, None)).await.status,
            StatusCode::NOT_FOUND,
            "{hidden}"
        );
    }

    // Without Kratos, there are no local accounts.
    let plain = remotehub_server::app(AppState::new(pool, None, settings(), vault()), None);
    let request = get_request("/api/auth/self-service/login/browser", None);
    assert_eq!(send(&plain, request).await.status, StatusCode::NOT_FOUND);
    assert_eq!(
        send(&plain, get_request("/api/session/methods", None))
            .await
            .json(),
        json!({ "directory": false, "local": false, "providers": [] })
    );
    assert_eq!(
        send(&app, get_request("/api/session/methods", None))
            .await
            .json(),
        json!({
            "directory": true, "local": true,
            "providers": [{ "id": "entra", "label": "Microsoft" }],
        })
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn wrong_passwords_through_kratos_are_slowed_down(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let attempt = || {
        Request::post("/api/auth/self-service/login?flow=f")
            .header(header::ORIGIN, ORIGIN)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap()
    };
    for _ in 0..30 {
        assert_eq!(send(&app, attempt()).await.status, StatusCode::BAD_REQUEST);
    }
    let blocked = send(&app, attempt()).await;
    assert_eq!(
        (blocked.status, blocked.code().as_str()),
        (StatusCode::TOO_MANY_REQUESTS, "too_many_attempts")
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn signing_out_ends_the_kratos_session_too(pool: PgPool) {
    let (app, calls) = setup(pool).await;
    let token = send(&app, sign_in("aal2")).await.session_token().unwrap();
    let request = Request::delete("/api/session")
        .header(header::ORIGIN, ORIGIN)
        .header(
            header::COOKIE,
            format!("__Host-remotehub-session={token}; kratos=aal2"),
        )
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, request).await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [format!("DELETE /admin/sessions/{KRATOS_SESSION}")]
    );
    assert_eq!(
        send(&app, get_request("/api/session", Some(&token)))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}
