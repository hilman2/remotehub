//! The audit log: what gets recorded, who may read it, and that it cannot be
//! changed unnoticed.

use crate::common::{ORIGIN, get, json, send, sign_in_request, state};
use axum::http::{StatusCode, header};
use remotehub_server::app;
use remotehub_server::audit::{self, Action, Actor, Entry};
use serde_json::{Value, json};
use sqlx::PgPool;

async fn sign_in(app: &axum::Router, user: &str) -> String {
    send(app, sign_in_request(user, "right"))
        .await
        .session_token()
        .unwrap()
}

fn with_cookie(
    mut request: axum::http::Request<axum::body::Body>,
    token: &str,
) -> axum::http::Request<axum::body::Body> {
    request.headers_mut().insert(
        header::COOKIE,
        format!("__Host-remotehub-session={token}").parse().unwrap(),
    );
    request
}

async fn entries(app: &axum::Router, token: &str) -> Vec<Value> {
    let response = send(app, get("/api/audit", Some(token))).await;
    assert_eq!(response.status, StatusCode::OK);
    response.json().as_array().unwrap().clone()
}

#[sqlx::test(migrations = "../../migrations")]
async fn sign_in_failure_and_sign_out_are_recorded(pool: PgPool) {
    let app = app(state(pool), None);
    send(&app, sign_in_request("alice", "wrong")).await;
    let bob = sign_in(&app, "bob").await;
    send(
        &app,
        with_cookie(
            json("DELETE", "/api/session", json!(null), Some(ORIGIN)),
            &bob,
        ),
    )
    .await;
    let alice = sign_in(&app, "alice").await;

    let log = entries(&app, &alice).await;
    let summary: Vec<(String, String)> = log
        .iter()
        .map(|e| {
            (
                e["action"].as_str().unwrap().to_owned(),
                e["actor_name"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("session.sign_in".to_owned(), "alice".to_owned()),
            ("session.sign_out".to_owned(), "bob".to_owned()),
            ("session.sign_in".to_owned(), "bob".to_owned()),
            ("session.sign_in_failed".to_owned(), "alice".to_owned()),
        ]
    );
    // Newest first; the failure says why, and never contains the password.
    assert_eq!(
        log[3]["details"],
        json!({ "reason": "invalid_credentials" })
    );
    assert!(!log.iter().any(|e| e.to_string().contains("wrong")));
    assert!(log[0]["at"].as_str().unwrap().ends_with('Z'));
}

#[sqlx::test(migrations = "../../migrations")]
async fn only_administrators_read_and_verify_the_log(pool: PgPool) {
    let app = app(state(pool), None);
    let bob = sign_in(&app, "bob").await;
    assert_eq!(
        send(&app, get("/api/audit", Some(&bob))).await.code(),
        "forbidden"
    );
    let verify = with_cookie(
        json("POST", "/api/audit/verify", json!(null), Some(ORIGIN)),
        &bob,
    );
    assert_eq!(send(&app, verify).await.status, StatusCode::FORBIDDEN);
    assert_eq!(
        send(&app, get("/api/audit", None)).await.code(),
        "unauthenticated"
    );

    let alice = sign_in(&app, "alice").await;
    let me = send(&app, get("/api/session", Some(&alice))).await.json();
    assert_eq!(me["admin"], true);
    let verify = with_cookie(
        json("POST", "/api/audit/verify", json!(null), Some(ORIGIN)),
        &alice,
    );
    let result = send(&app, verify).await;
    assert_eq!(result.json(), json!({ "entries": 2, "first_broken": null }));
    // The check is recorded, too.
    assert_eq!(entries(&app, &alice).await[0]["action"], "audit.verified");
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_log_refuses_changes(pool: PgPool) {
    record(&pool, "alice").await;
    for statement in [
        "UPDATE audit_log SET actor_name = 'mallory'",
        "DELETE FROM audit_log",
        "TRUNCATE audit_log",
    ] {
        let error = sqlx::query(statement).execute(&pool).await.unwrap_err();
        assert!(
            error.to_string().contains("append-only"),
            "{statement}: {error}"
        );
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn tampering_breaks_the_chain_from_that_entry_on(pool: PgPool) {
    for name in ["a", "b", "c", "d"] {
        record(&pool, name).await;
    }
    assert_eq!(audit::verify(&pool).await.unwrap().first_broken, None);

    // A superuser can switch the trigger off — the hash chain still notices.
    sqlx::query("ALTER TABLE audit_log DISABLE TRIGGER audit_log_no_change")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE audit_log SET actor_name = 'mallory' WHERE seq = 2")
        .execute(&pool)
        .await
        .unwrap();
    let verification = audit::verify(&pool).await.unwrap();
    assert_eq!(
        (verification.entries, verification.first_broken),
        (4, Some(2))
    );

    // Removing an entry breaks the link of its successor.
    sqlx::query("UPDATE audit_log SET actor_name = 'b' WHERE seq = 2")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(audit::verify(&pool).await.unwrap().first_broken, None);
    sqlx::query("DELETE FROM audit_log WHERE seq = 3")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(audit::verify(&pool).await.unwrap().first_broken, Some(4));
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_writers_keep_one_gapless_chain(pool: PgPool) {
    let writers: Vec<_> = (0..20)
        .map(|i| {
            let pool = pool.clone();
            tokio::spawn(async move {
                let mut tx = pool.begin().await.unwrap();
                record(&mut *tx, &format!("user{i}")).await;
                tx.commit().await.unwrap();
            })
        })
        .collect();
    for writer in writers {
        writer.await.unwrap();
    }
    let seqs: Vec<i64> = sqlx::query_scalar("SELECT seq FROM audit_log ORDER BY seq")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(seqs, (1..=20).collect::<Vec<i64>>());
    assert_eq!(audit::verify(&pool).await.unwrap().first_broken, None);
}

async fn record<'e>(db: impl sqlx::PgExecutor<'e>, name: &str) {
    audit::record(
        db,
        Entry {
            actor: Actor { id: None, name },
            action: Action::SignInFailed,
            object: None,
            details: json!({ "reason": "invalid_credentials" }),
            address: Some("10.0.0.1"),
        },
    )
    .await
    .unwrap();
}
