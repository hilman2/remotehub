//! What users picked after searching (#81): their own picks only, counted,
//! and trimmed to the newest.

use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{authed, get, send, sign_in_request, state};
use remotehub_server::app;

const DEVICE: &str = "device:0d9c2f3e-4b1a-4c55-9a7e-0f2b3c4d5e6f";

async fn picks(app: &axum::Router, token: &str) -> Value {
    send(app, get("/api/search/picks", Some(token)))
        .await
        .json()
}

async fn pick(app: &axum::Router, token: &str, key: &str, query: &str) -> u16 {
    send(
        app,
        authed(
            "POST",
            "/api/search/picks",
            Some(json!({ "key": key, "query": query })),
            token,
        ),
    )
    .await
    .status
    .as_u16()
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn picks_are_counted_per_query_and_stay_with_their_user(pool: PgPool) {
    let app = app(state(pool), None);
    let alice = send(&app, sign_in_request("alice", "right"))
        .await
        .session_token()
        .unwrap();
    let bob = send(&app, sign_in_request("bob", "right"))
        .await
        .session_token()
        .unwrap();

    assert_eq!(pick(&app, &alice, DEVICE, "dc").await, 204);
    assert_eq!(pick(&app, &alice, DEVICE, "dc").await, 204);
    assert_eq!(pick(&app, &alice, DEVICE, "").await, 204);
    let mine = picks(&app, &alice).await;
    let counts: Vec<(String, i64)> = mine
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["query"].as_str().unwrap().to_owned(),
                p["count"].as_i64().unwrap(),
            )
        })
        .collect();
    assert!(counts.contains(&("dc".to_owned(), 2)), "{mine}");
    assert!(counts.contains(&(String::new(), 1)), "{mine}");
    assert!(
        mine[0]["last"].as_i64().unwrap() > 1_700_000_000_000,
        "{mine}"
    );
    assert_eq!(picks(&app, &bob).await, json!([]));

    for (key, query) in [
        ("user:0d9c2f3e-4b1a-4c55-9a7e-0f2b3c4d5e6f", "x"),
        ("device:nope", "x"),
        (DEVICE, &"x".repeat(101)),
        (DEVICE, "tab\there"),
    ] {
        assert_eq!(pick(&app, &alice, key, query).await, 400, "{key} {query:?}");
    }
    assert_eq!(pick(&app, "no-session", DEVICE, "dc").await, 401);
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn only_the_newest_picks_are_kept(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let alice = send(&app, sign_in_request("alice", "right"))
        .await
        .session_token()
        .unwrap();
    // 300 old picks, straight into the table.
    sqlx::query(
        "INSERT INTO search_picks (user_id, key, query, last_at)
         SELECT u.id, $1, 'q' || n, now() - interval '1 day' - n * interval '1 minute'
         FROM users u, generate_series(1, 300) n WHERE u.username = 'alice'",
    )
    .bind(DEVICE)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(pick(&app, &alice, DEVICE, "newest").await, 204);
    let kept = picks(&app, &alice).await;
    let queries: Vec<&str> = kept
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["query"].as_str().unwrap())
        .collect();
    assert_eq!(queries.len(), 300);
    assert_eq!(queries[0], "newest");
    // The oldest one made room.
    assert!(!queries.contains(&"q300"), "{kept}");
    assert!(queries.contains(&"q1"), "{kept}");
}
