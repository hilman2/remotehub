//! Local accounts through Ory Kratos (#103), against a stand-in for Kratos:
//! its answers depend on the cookie the browser sends, and it notes which
//! sessions remotehub ends.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use remotehub_server::config::KratosConfig;
use remotehub_server::kratos::Kratos;
use remotehub_server::{AppState, app};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{ORIGIN, get as get_request, send, settings, vault};

const ADA: &str = "5b0c3cb1-7a0e-4a5e-9d0a-0000000000a1";
const ADMIN: &str = "5b0c3cb1-7a0e-4a5e-9d0a-0000000000a2";
const KRATOS_SESSION: &str = "0f0e0d0c-0b0a-4908-8706-050403020100";

/// Sessions the stand-in was asked to end.
type Revoked = Arc<Mutex<Vec<String>>>;

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

async fn stand_in() -> (String, Revoked) {
    let revoked = Revoked::default();
    let router = Router::new()
        .route("/sessions/whoami", get(whoami))
        .route("/self-service/login/browser", get(login_flow))
        // Every password is wrong.
        .route(
            "/self-service/login",
            post(|| async { (StatusCode::BAD_REQUEST, axum::Json(json!({ "ui": {} }))) }),
        )
        .route("/health/alive", get(|| async { "ok" }))
        .route(
            "/admin/sessions/{id}",
            delete(
                |State(revoked): State<Revoked>, Path(id): Path<String>| async move {
                    revoked.lock().unwrap().push(id);
                    StatusCode::NO_CONTENT
                },
            ),
        )
        .with_state(revoked.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}"), revoked)
}

async fn setup(pool: PgPool) -> (Router, Revoked) {
    let (url, revoked) = stand_in().await;
    let mut settings = settings();
    settings.kratos = Some(Kratos::new(&KratosConfig {
        public_url: url.clone(),
        admin_url: url,
    }));
    settings.admin_accounts = vec!["admin@example.com".to_owned()];
    let state = AppState::new(pool, None, settings, vault());
    (app(state, None), revoked)
}

/// `POST /api/session/local` from a browser whose Kratos cookie is `case`.
fn sign_in(case: &str) -> Request<Body> {
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
        json!({ "username": "ada@example.com", "display_name": "Ada", "kind": "local", "admin": false })
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
    for hidden in ["/api/auth/health/alive", "/api/auth/admin/sessions/x"] {
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
        json!({ "directory": false, "local": false })
    );
    assert_eq!(
        send(&app, get_request("/api/session/methods", None))
            .await
            .json(),
        json!({ "directory": false, "local": true })
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
    let (app, revoked) = setup(pool).await;
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
    assert_eq!(revoked.lock().unwrap().as_slice(), [KRATOS_SESSION]);
    assert_eq!(
        send(&app, get_request("/api/session", Some(&token)))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}
