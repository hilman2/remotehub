//! Confirming with the second factor (#241, #242, #243): a locked session
//! is unlocked this way, and showing a secret takes a confirmation of the
//! last minute. alice administers, bob is a directory user, Ada a local
//! account in the stand-in Kratos of `accounts.rs`.

use axum::Router;
use axum::http::StatusCode;
use remotehub_server::break_glass;
use remotehub_server::{AppState, app};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::accounts::{ADA_KEY, ADA_TOTP};
use crate::common::{ORIGIN, Response, collection, json as request, send, state};
use crate::second_factor::{Key, b64, call, code, enroll, token};

/// Lets the session of `token` idle past the idle time.
async fn idle(db: &PgPool, token: &str) {
    sqlx::query(
        "UPDATE sessions SET last_seen_at = now() - interval '31 minutes' WHERE token_hash = $1",
    )
    .bind(Sha256::digest(token.as_bytes()).to_vec())
    .execute(db)
    .await
    .unwrap();
}

async fn start(app: &Router, token: &str) -> Value {
    let started = call(app, token, "POST", "/api/session/confirm/start", None).await;
    assert_eq!(started.status, StatusCode::OK, "{}", started.json());
    started.json()
}

async fn confirm(app: &Router, token: &str, body: Value) -> Response {
    call(app, token, "POST", "/api/session/confirm", Some(body)).await
}

fn problem(response: &Response) -> (StatusCode, String) {
    (response.status, response.code())
}

/// The audit entries of `user`, oldest first: action and details.
async fn audited(app: &Router, admin: &str, user: &str) -> Vec<(String, Value)> {
    let log = call(app, admin, "GET", "/api/audit", None).await.json();
    let mut entries: Vec<(String, Value)> = log
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["actor_name"] == user && e["action"].as_str().unwrap().contains("confirm"))
        .map(|e| {
            (
                e["action"].as_str().unwrap().to_owned(),
                e["details"].clone(),
            )
        })
        .collect();
    entries.reverse();
    entries
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_locked_session_is_unlocked_with_a_code_of_the_app(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let alice = token(&app, "alice", json!({})).await;
    let bob = token(&app, "bob", json!({})).await;
    let secret = enroll(&app, &bob).await;
    idle(&pool, &bob).await;
    let locked = call(&app, &bob, "GET", "/api/tree", None).await;
    assert_eq!(
        problem(&locked),
        (StatusCode::UNAUTHORIZED, "session_locked".into())
    );

    assert_eq!(start(&app, &bob).await, json!({ "app": true, "key": null }));
    let wrong = confirm(&app, &bob, json!({ "code": code(&secret, 300) })).await;
    assert_eq!(
        problem(&wrong),
        (StatusCode::UNAUTHORIZED, "second_factor_invalid".into())
    );
    let right = code(&secret, 0);
    let unlocked = confirm(&app, &bob, json!({ "code": right })).await;
    assert_eq!(
        unlocked.status,
        StatusCode::NO_CONTENT,
        "{}",
        unlocked.json()
    );
    assert_eq!(
        call(&app, &bob, "GET", "/api/tree", None).await.status,
        StatusCode::OK
    );
    // Each code confirms once.
    let again = confirm(&app, &bob, json!({ "code": right })).await;
    assert_eq!(again.code(), "second_factor_invalid");
    // Without an answer, nothing is checked.
    let empty = confirm(&app, &bob, json!({})).await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);

    assert_eq!(
        audited(&app, &alice, "bob").await,
        [
            (
                "session.confirm_failed".into(),
                json!({ "method": "app", "ended": false })
            ),
            (
                "session.confirmed".into(),
                json!({ "method": "app", "unlocked": true })
            ),
            (
                "session.confirm_failed".into(),
                json!({ "method": "app", "ended": false })
            ),
        ]
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn five_wrong_answers_in_a_row_end_the_session(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let bob = token(&app, "bob", json!({})).await;
    let secret = enroll(&app, &bob).await;
    let wrong = || json!({ "code": code(&secret, 600) });
    for _ in 0..4 {
        let refused = confirm(&app, &bob, wrong()).await;
        assert_eq!(refused.code(), "second_factor_invalid");
    }
    let ended = confirm(&app, &bob, wrong()).await;
    assert_eq!(
        problem(&ended),
        (StatusCode::UNAUTHORIZED, "unauthenticated".into())
    );
    assert!(ended.headers.get_all("set-cookie").iter().any(|c| {
        c.to_str()
            .unwrap()
            .starts_with("__Host-remotehub-session=;")
    }));
    let me = call(&app, &bob, "GET", "/api/session", None).await;
    assert_eq!(me.code(), "unauthenticated");
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_key_confirms_only_when_it_checks_its_holder(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let bob = token(&app, "bob", json!({})).await;
    let factors = || call(&app, &bob, "GET", "/api/session/factors", None);
    assert_eq!(
        factors().await.json(),
        json!({ "app": false, "keys": false })
    );
    let mut key = Key::new(3);
    let offer = call(&app, &bob, "POST", "/api/account/security-keys/offer", None)
        .await
        .json();
    let body = json!({
        "challenge_id": offer["challenge_id"], "name": "Desk key",
        "credential": key.register(&offer["options"]),
    });
    let added = call(&app, &bob, "POST", "/api/account/security-keys", Some(body)).await;
    assert_eq!(added.status, StatusCode::NO_CONTENT, "{}", added.json());
    assert_eq!(
        factors().await.json(),
        json!({ "app": false, "keys": true })
    );
    idle(&pool, &bob).await;

    let started = start(&app, &bob).await;
    assert_eq!(started["app"], false);
    let options = &started["key"]["options"]["publicKey"];
    assert_eq!(options["userVerification"], "required");
    assert_eq!(options["allowCredentials"][0]["id"], b64(&key.id));
    // A touch alone is not enough.
    let answer = |key: &mut Key, started: &Value| {
        json!({ "key": {
            "challenge_id": started["key"]["challenge_id"],
            "credential": key.sign(&started["key"]["options"]),
        } })
    };
    let touched = confirm(&app, &bob, answer(&mut key, &started)).await;
    assert_eq!(touched.code(), "second_factor_invalid");

    key.verifies = true;
    let started = start(&app, &bob).await;
    let verified = answer(&mut key, &started);
    let unlocked = confirm(&app, &bob, verified.clone()).await;
    assert_eq!(
        unlocked.status,
        StatusCode::NO_CONTENT,
        "{}",
        unlocked.json()
    );
    assert_eq!(
        call(&app, &bob, "GET", "/api/tree", None).await.status,
        StatusCode::OK
    );
    // Its challenge is used up.
    let replayed = confirm(&app, &bob, verified).await;
    assert_eq!(replayed.code(), "second_factor_invalid");
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn showing_a_secret_takes_a_confirmation_of_the_last_minute(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let alice = token(&app, "alice", json!({})).await;
    let secret = enroll(&app, &alice).await;
    let vault = collection(&app, &alice, None, "Vault").await;
    let created = call(
        &app,
        &alice,
        "POST",
        "/api/credentials",
        Some(json!({
            "collection_id": vault, "name": "router", "username": "admin", "password": "S3cret!",
            "totp": "otpauth://totp/router?secret=JBSWY3DPEHPK3PXP",
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json());
    let id = created.json()["id"].as_str().unwrap().to_owned();
    let reveal = format!("/api/credentials/{id}/reveal");
    let one_time = format!("/api/credentials/{id}/code");
    let show = || Some(json!({ "purpose": "show" }));

    for uri in [&reveal, &one_time] {
        let asked = call(&app, &alice, "POST", uri, show()).await;
        assert_eq!(
            problem(&asked),
            (StatusCode::FORBIDDEN, "confirmation_required".into()),
            "{uri}"
        );
    }
    // Listing the versions shows no secret.
    let versions = format!("/api/credentials/{id}/versions");
    assert_eq!(
        call(&app, &alice, "GET", &versions, None).await.status,
        StatusCode::OK
    );

    let confirmed = confirm(&app, &alice, json!({ "code": code(&secret, 0) })).await;
    assert_eq!(confirmed.status, StatusCode::NO_CONTENT);
    for uri in [&reveal, &one_time] {
        let shown = call(&app, &alice, "POST", uri, show()).await;
        assert_eq!(shown.status, StatusCode::OK, "{uri}: {}", shown.json());
    }

    sqlx::query("UPDATE sessions SET confirmed_at = now() - interval '61 seconds'")
        .execute(&pool)
        .await
        .unwrap();
    let later = call(&app, &alice, "POST", &reveal, show()).await;
    assert_eq!(later.code(), "confirmation_required");
    // A confirmation is not something to wait for when unlocked.
    let unlocked = call(&app, &alice, "GET", "/api/tree", None).await;
    assert_eq!(unlocked.status, StatusCode::OK);
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn devices_keep_whether_they_ask_for_a_confirmation(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let alice = token(&app, "alice", json!({})).await;
    let folder = call(
        &app,
        &alice,
        "POST",
        "/api/folders",
        Some(json!({ "parent_id": null, "name": "Domain" })),
    )
    .await
    .json()["id"]
        .clone();
    let device = |asks: bool| {
        json!({
            "folder_id": folder, "name": "dc01", "protocol": "rdp", "host": "dc01", "port": 3389,
            "auth_mode": "ask", "requires_confirmation": asks,
        })
    };
    let created = call(&app, &alice, "POST", "/api/devices", Some(device(true))).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json());
    let id = created.json()["id"].as_str().unwrap().to_owned();
    let listed = || async {
        call(&app, &alice, "GET", "/api/tree", None).await.json()["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == id.as_str())
            .unwrap()["requires_confirmation"]
            .clone()
    };
    assert_eq!(listed().await, true);
    let uri = format!("/api/devices/{id}");
    let changed = call(&app, &alice, "PUT", &uri, Some(device(false))).await;
    assert_eq!(changed.status, StatusCode::NO_CONTENT, "{}", changed.json());
    assert_eq!(listed().await, false);
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_local_account_confirms_with_what_kratos_keeps(pool: PgPool) {
    let (app, _calls) = crate::accounts::setup(pool.clone()).await;
    let signed_in = send(&app, crate::accounts::sign_in("aal2")).await;
    assert_eq!(signed_in.status, StatusCode::OK, "{}", signed_in.json());
    let ada = signed_in.session_token().unwrap();
    idle(&pool, &ada).await;

    let started = start(&app, &ada).await;
    assert_eq!(started["app"], true);
    let mut key = Key::new(ADA_KEY);
    assert_eq!(
        started["key"]["options"]["publicKey"]["allowCredentials"][0]["id"],
        b64(&key.id)
    );
    let right = code(ADA_TOTP, 0);
    let unlocked = confirm(&app, &ada, json!({ "code": right })).await;
    assert_eq!(
        unlocked.status,
        StatusCode::NO_CONTENT,
        "{}",
        unlocked.json()
    );
    let again = confirm(&app, &ada, json!({ "code": right })).await;
    assert_eq!(again.code(), "second_factor_invalid");

    key.verifies = true;
    let started = start(&app, &ada).await;
    let answer = json!({ "key": {
        "challenge_id": started["key"]["challenge_id"],
        "credential": key.sign(&started["key"]["options"]),
    } });
    let confirmed = confirm(&app, &ada, answer).await;
    assert_eq!(
        confirmed.status,
        StatusCode::NO_CONTENT,
        "{}",
        confirmed.json()
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_break_glass_account_confirms_with_its_code(pool: PgPool) {
    let state: AppState = state(pool.clone());
    let issued = break_glass::create(&pool, &state.vault, "emergency")
        .await
        .unwrap();
    let app = app(state, None);
    let at = |offset: i64| code(&issued.totp_secret, offset);
    let body = json!({
        "username": "emergency", "password": issued.password.as_str(), "code": at(-30),
    });
    let signed_in = send(
        &app,
        request("POST", "/api/session/break-glass", body, Some(ORIGIN)),
    )
    .await;
    assert_eq!(signed_in.status, StatusCode::OK, "{}", signed_in.json());
    let token = signed_in.session_token().unwrap();
    idle(&pool, &token).await;

    assert_eq!(
        start(&app, &token).await,
        json!({ "app": true, "key": null })
    );
    let unlocked = confirm(&app, &token, json!({ "code": at(0) })).await;
    assert_eq!(
        unlocked.status,
        StatusCode::NO_CONTENT,
        "{}",
        unlocked.json()
    );
}
