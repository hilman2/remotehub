//! Sign-in, sessions and sign-out over HTTP, with a fake directory.

use crate::common::{
    ADMINS_SID, ALICE_SID, ORIGIN, get, json, send, settings, sign_in_request, state,
};
use axum::http::{StatusCode, header};
use remotehub_server::{AppState, app};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
async fn sign_in_session_and_sign_out(pool: PgPool) {
    let app = app(state(pool.clone()), None);

    let response = send(&app, sign_in_request("alice", "right")).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.json(),
        json!({
            "username": "alice",
            "display_name": "Alice Admin",
            "kind": "directory",
            "admin": true,
            "roles": ["administrator"]
        })
    );
    let cookie = response.headers[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_owned();
    for attribute in [
        "__Host-remotehub-session=",
        "HttpOnly",
        "Secure",
        "SameSite=Strict",
        "Path=/",
    ] {
        assert!(
            cookie.contains(attribute),
            "{attribute} missing in {cookie}"
        );
    }
    let token = response.session_token().unwrap();

    let me = send(&app, get("/api/session", Some(&token))).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.json()["username"], "alice");

    let out = send(
        &app,
        json("DELETE", "/api/session", json!(null), Some(ORIGIN)),
    )
    .await;
    // Without the cookie nothing is ended, but the cookie is cleared anyway.
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert_eq!(
        send(&app, get("/api/session", Some(&token))).await.status,
        StatusCode::OK
    );

    let mut sign_out = json("DELETE", "/api/session", json!(null), Some(ORIGIN));
    sign_out.headers_mut().insert(
        header::COOKIE,
        format!("__Host-remotehub-session={token}").parse().unwrap(),
    );
    let out = send(&app, sign_out).await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert!(
        out.headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );

    let after = send(&app, get("/api/session", Some(&token))).await;
    assert_eq!(after.status, StatusCode::UNAUTHORIZED);
    assert_eq!(after.code(), "unauthenticated");
}

#[sqlx::test(migrations = "../../migrations")]
async fn stores_the_user_by_sid_and_only_a_hash_of_the_token(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let token = send(&app, sign_in_request("alice", "right"))
        .await
        .session_token()
        .unwrap();
    // A second sign-in reuses the same user.
    send(&app, sign_in_request("alice", "right"))
        .await
        .session_token()
        .unwrap();

    let (sid, users): (String, i64) = sqlx::query_as("SELECT min(sid), count(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((sid.as_str(), users), (ALICE_SID, 1));

    let hashes: Vec<Vec<u8>> = sqlx::query_scalar("SELECT token_hash FROM sessions")
        .fetch_all(&pool)
        .await
        .unwrap();
    let expected: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    assert!(hashes.iter().any(|h| h.as_slice() == expected));
    assert!(hashes.iter().all(|h| h.as_slice() != token.as_bytes()));

    let groups: Vec<String> = sqlx::query_scalar("SELECT unnest(groups) FROM sessions LIMIT 1")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(groups, [ADMINS_SID]);
}

#[sqlx::test(migrations = "../../migrations")]
async fn failed_sign_ins_explain_only_what_is_safe(pool: PgPool) {
    let app = app(state(pool), None);
    for (user, password, status, code) in [
        (
            "alice",
            "wrong",
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
        ),
        (
            "nobody",
            "x",
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
        ),
        ("carol", "right", StatusCode::FORBIDDEN, "account_disabled"),
        (
            "offline",
            "x",
            StatusCode::SERVICE_UNAVAILABLE,
            "directory_unavailable",
        ),
    ] {
        let response = send(&app, sign_in_request(user, password)).await;
        assert_eq!(
            (response.status, response.code().as_str()),
            (status, code),
            "{user}"
        );
        assert!(response.session_token().is_none(), "{user}");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn repeated_failures_block_the_user_for_a_while(pool: PgPool) {
    let app = app(state(pool), None);
    for _ in 0..5 {
        assert_eq!(
            send(&app, sign_in_request("alice", "wrong")).await.code(),
            "invalid_credentials"
        );
    }
    let blocked = send(&app, sign_in_request("alice", "right")).await;
    assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(blocked.code(), "too_many_attempts");
    assert!(
        blocked.json()["params"]["retry_after_seconds"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(blocked.session_token().is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn foreign_origins_cannot_change_state(pool: PgPool) {
    let app = app(state(pool), None);
    let body = json!({ "username": "alice", "password": "right" });

    let foreign = send(
        &app,
        json(
            "POST",
            "/api/session",
            body.clone(),
            Some("https://evil.example"),
        ),
    )
    .await;
    assert_eq!(foreign.status, StatusCode::FORBIDDEN);
    assert_eq!(foreign.code(), "forbidden_origin");

    // Clients without a browser send no Origin.
    let without = send(&app, json("POST", "/api/session", body, None)).await;
    assert_eq!(without.status, StatusCode::OK);
}

#[sqlx::test(migrations = "../../migrations")]
async fn sessions_end_when_idle_or_expired(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let idle = send(&app, sign_in_request("alice", "right"))
        .await
        .session_token()
        .unwrap();
    let expired = send(&app, sign_in_request("alice", "right"))
        .await
        .session_token()
        .unwrap();
    let hash = |t: &str| -> Vec<u8> { Sha256::digest(t.as_bytes()).to_vec() };

    sqlx::query(
        "UPDATE sessions SET last_seen_at = now() - interval '31 minutes' WHERE token_hash = $1",
    )
    .bind(hash(&idle))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE sessions SET expires_at = now() - interval '1 second' WHERE token_hash = $1",
    )
    .bind(hash(&expired))
    .execute(&pool)
    .await
    .unwrap();

    for token in [&idle, &expired] {
        assert_eq!(
            send(&app, get("/api/session", Some(token))).await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        remotehub_server::session::purge(&pool, settings().session.idle)
            .await
            .unwrap(),
        2
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn invalid_bodies_and_missing_directory_are_problems(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let broken = send(
        &app,
        json(
            "POST",
            "/api/session",
            json!({ "user": "alice" }),
            Some(ORIGIN),
        ),
    )
    .await;
    assert_eq!(
        (broken.status, broken.code().as_str()),
        (StatusCode::BAD_REQUEST, "invalid_request")
    );

    let without_directory = app_without_directory(pool);
    let response = send(&without_directory, sign_in_request("alice", "right")).await;
    assert_eq!(response.code(), "directory_unavailable");
}

fn app_without_directory(pool: PgPool) -> axum::Router {
    let state = AppState::new(pool, None, settings(), crate::common::vault());
    app(state, None)
}

fn login_key(response: &crate::common::Response) -> Option<String> {
    response
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("__Host-remotehub-login-key="))
        .map(str::to_owned)
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_sign_in_password_is_kept_sealed_with_a_key_only_the_browser_has(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let response = send(&app, sign_in_request("alice", "right")).await;
    let cookie = login_key(&response).expect("key cookie");
    for attribute in ["HttpOnly", "Secure", "SameSite=Strict", "Path=/"] {
        assert!(
            cookie.contains(attribute),
            "{attribute} missing in {cookie}"
        );
    }

    let sealed: Vec<u8> = sqlx::query_scalar("SELECT login_secret FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!sealed.is_empty());
    assert!(!sealed.windows(5).any(|w| w == b"right"));
    // The key never reaches the database.
    let key = cookie.split(';').next().unwrap().split_once('=').unwrap().1;
    let dump: String = sqlx::query_scalar("SELECT string_agg(s::text, '') FROM sessions s")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!dump.contains(key));

    // Signing out removes the key cookie, too.
    let token = response.session_token().unwrap();
    let mut out = crate::common::authed("DELETE", "/api/session", None, &token);
    out.headers_mut().remove(header::CONTENT_TYPE);
    let out = send(&app, out).await;
    let cleared: Vec<&str> = out
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect();
    assert!(
        cleared
            .iter()
            .any(|c| c.starts_with("__Host-remotehub-login-key=;") && c.contains("Max-Age=0"))
    );
}

/// A sign-in from `client` that reaches the server through the reverse proxy
/// at 127.0.0.1.
fn through_proxy(
    username: &str,
    password: &str,
    client: &str,
) -> axum::http::Request<axum::body::Body> {
    let mut request = sign_in_request(username, password);
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            41000,
        ))));
    request
        .headers_mut()
        .insert("x-forwarded-for", client.parse().unwrap());
    request
}

#[sqlx::test(migrations = "../../migrations")]
async fn behind_a_trusted_proxy_every_client_counts_for_itself(pool: PgPool) {
    let mut settings = settings();
    settings.trusted_proxies = vec!["127.0.0.1".parse().unwrap()];
    let app = app(
        AppState::new(
            pool.clone(),
            Some(std::sync::Arc::new(crate::common::FakeDirectory)),
            settings,
            crate::common::vault(),
        ),
        None,
    );
    // One client fails for many names, until its address is blocked.
    let mut last = String::new();
    for n in 0..40 {
        last = send(
            &app,
            through_proxy(&format!("guess{n}"), "x", "203.0.113.7"),
        )
        .await
        .code();
    }
    assert_eq!(last, "too_many_attempts");
    // Another client behind the same proxy still signs in.
    let other = send(&app, through_proxy("alice", "right", "203.0.113.8")).await;
    assert_eq!(other.status, StatusCode::OK);

    let addresses: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT address FROM audit_log ORDER BY address")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(addresses, ["203.0.113.7", "203.0.113.8"]);
}

#[sqlx::test(migrations = "../../migrations")]
async fn keeping_the_password_can_be_switched_off(pool: PgPool) {
    let mut settings = settings();
    settings.own_account_connections = false;
    let app = app(
        AppState::new(
            pool.clone(),
            Some(std::sync::Arc::new(crate::common::FakeDirectory)),
            settings,
            crate::common::vault(),
        ),
        None,
    );
    let response = send(&app, sign_in_request("alice", "right")).await;
    assert!(response.session_token().is_some());
    assert!(login_key(&response).is_none());
    let sealed: Option<Vec<u8>> = sqlx::query_scalar("SELECT login_secret FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(sealed.is_none());
}
