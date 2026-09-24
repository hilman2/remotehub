//! HTTP behaviour of the whole application. Tests marked `#[sqlx::test]` get
//! a fresh database with all migrations (needs `DATABASE_URL`, provided by the
//! development compose and the local CI).

use std::net::{Ipv4Addr, SocketAddr};

use crate::common::{get, send, state, unreachable_pool};
use crate::terminal::serve;
use axum::http::StatusCode;
use remotehub_server::api::health;
use remotehub_server::{VERSION, app, db};
use secrecy::SecretString;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
async fn health_reports_version_and_database(pool: PgPool) {
    let response = send(&app(state(pool), None), get("/api/health", None)).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.json(),
        json!({ "status": "ok", "version": VERSION, "database": "ok" })
    );
}

#[tokio::test]
async fn health_is_unavailable_without_database() {
    let response = send(
        &app(state(unreachable_pool()), None),
        get("/api/health", None),
    )
    .await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.json()["database"], "unavailable");
    assert_eq!(response.json()["version"], VERSION);
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_health_probe_passes_only_a_healthy_server(pool: PgPool) {
    let healthy = serve(state(pool)).await;
    health::probe(healthy).await.unwrap();
    // The default REMOTEHUB_LISTEN, 0.0.0.0:8080, is what the probe gets.
    let everywhere = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), healthy.port());
    health::probe(everywhere).await.unwrap();

    let without_database = serve(state(unreachable_pool())).await;
    let error = health::probe(without_database).await.unwrap_err();
    assert!(error.contains("503"), "{error}");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let nobody = listener.local_addr().unwrap();
    drop(listener);
    assert!(health::probe(nobody).await.is_err());
}

/// The tests' database URL without its password, and the password.
fn url_and_password() -> (String, String) {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL for the tests");
    let (scheme, rest) = url.split_once("://").unwrap();
    let (credentials, host) = rest.split_once('@').unwrap();
    let (user, password) = credentials.split_once(':').unwrap();
    (format!("{scheme}://{user}@{host}"), password.to_owned())
}

#[tokio::test]
async fn the_database_password_can_be_separate_from_the_url() {
    let (url, password) = url_and_password();
    let password = SecretString::from(password);
    let pool = db::connect(&url, Some(&password)).await.unwrap();
    assert!(db::is_reachable(&pool).await);

    // The separate password wins over one in the URL.
    let (scheme, rest) = url.split_once('@').unwrap();
    let wrong_in_url = format!("{scheme}:wrong@{rest}");
    let pool = db::connect(&wrong_in_url, Some(&password)).await.unwrap();
    assert!(db::is_reachable(&pool).await);
    let wrong = SecretString::from("wrong".to_owned());
    assert!(db::connect(&url, Some(&wrong)).await.is_err());
}

#[sqlx::test(migrations = "../../migrations")]
async fn migrations_create_exactly_one_instance(pool: PgPool) {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM instance")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    // The singleton constraint keeps it at one row.
    assert!(
        sqlx::query("INSERT INTO instance DEFAULT VALUES")
            .execute(&pool)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn serves_the_spa_for_routes_but_not_for_unknown_api_paths() {
    let web = tempfile::tempdir().unwrap();
    std::fs::write(
        web.path().join("index.html"),
        "<!doctype html><title>spa</title>",
    )
    .unwrap();
    std::fs::write(web.path().join("app.js"), "console.log(1)").unwrap();
    let app = app(state(unreachable_pool()), Some(web.path()));

    let response = send(&app, get("/devices/42", None)).await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(String::from_utf8(response.body).unwrap().contains("spa"));

    let response = send(&app, get("/app.js", None)).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body, b"console.log(1)");

    let response = send(&app, get("/api/does-not-exist", None)).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(response.code(), "not_found");
    assert_eq!(response.json()["status"], 404);
}

#[tokio::test]
async fn without_a_built_ui_only_the_api_answers() {
    let response = send(&app(state(unreachable_pool()), None), get("/", None)).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
}
