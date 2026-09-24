//! HTTP behaviour of the whole application. Tests marked `#[sqlx::test]` get
//! a fresh database with all migrations (needs `DATABASE_URL`, provided by the
//! development compose and the local CI).

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use remotehub_server::{AppState, VERSION, app};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

async fn get(app: Router, uri: &str) -> (StatusCode, Vec<u8>) {
    let response = app
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body.to_vec())
}

/// A pool whose database does not exist, for behaviour without a database.
fn unreachable_pool() -> PgPool {
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(500))
        .connect_lazy("postgres://nobody:nothing@127.0.0.1:1/none")
        .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn health_reports_version_and_database(pool: PgPool) {
    let (status, body) = get(app(AppState { db: pool }, None), "/api/health").await;
    assert_eq!(status, StatusCode::OK);
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        body,
        json!({ "status": "ok", "version": VERSION, "database": "ok" })
    );
}

#[tokio::test]
async fn health_is_unavailable_without_database() {
    let (status, body) = get(
        app(
            AppState {
                db: unreachable_pool(),
            },
            None,
        ),
        "/api/health",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["database"], "unavailable");
    assert_eq!(body["version"], VERSION);
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
    let app = app(
        AppState {
            db: unreachable_pool(),
        },
        Some(web.path()),
    );

    let (status, body) = get(app.clone(), "/devices/42").await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8(body).unwrap().contains("spa"));

    let (status, body) = get(app.clone(), "/app.js").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"console.log(1)");

    let (status, body) = get(app, "/api/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let problem: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(problem["code"], "not_found");
    assert_eq!(problem["status"], 404);
}

#[tokio::test]
async fn without_a_built_ui_only_the_api_answers() {
    let (status, _) = get(
        app(
            AppState {
                db: unreachable_pool(),
            },
            None,
        ),
        "/",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
