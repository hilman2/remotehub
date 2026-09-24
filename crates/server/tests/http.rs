//! HTTP behaviour of the whole application. Tests marked `#[sqlx::test]` get
//! a fresh database with all migrations (needs `DATABASE_URL`, provided by the
//! development compose and the local CI).

mod common;

use axum::http::StatusCode;
use common::{get, send, state, unreachable_pool};
use remotehub_server::{VERSION, app};
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
