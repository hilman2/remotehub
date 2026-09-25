//! User management for administrators (#104), with the stand-in for Kratos
//! from `accounts`: alice administers, bob comes from the directory, Ada is
//! a local account.

use axum::Router;
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::accounts::{ADA, Calls, INVITED, TAKEN, setup, sign_in};
use crate::common::{authed, get as get_request, send, sign_in_request};

async fn token(app: &Router, request: axum::http::Request<axum::body::Body>) -> String {
    let response = send(app, request).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json());
    response.session_token().unwrap()
}

/// The users as `GET /api/users` lists them for `admin`.
async fn users(app: &Router, admin: &str) -> Vec<Value> {
    let response = send(app, get_request("/api/users", Some(admin))).await;
    assert_eq!(response.status, StatusCode::OK);
    response.json().as_array().unwrap().clone()
}

async fn id_of(app: &Router, admin: &str, username: &str) -> String {
    users(app, admin)
        .await
        .into_iter()
        .find(|u| u["username"] == username)
        .unwrap_or_else(|| panic!("{username} is listed"))["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn taken(calls: &Calls) -> Vec<String> {
    std::mem::take(&mut *calls.lock().unwrap())
}

async fn actions(app: &Router, admin: &str) -> Vec<String> {
    send(app, get_request("/api/audit", Some(admin)))
        .await
        .json()
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["action"].as_str())
        .filter(|a| a.starts_with("user.") || a.starts_with("account."))
        .map(str::to_owned)
        .collect()
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn only_administrators_see_and_manage_users(pool: PgPool) {
    let (app, calls) = setup(pool).await;
    let bob = token(&app, sign_in_request("bob", "right")).await;
    let ada = token(&app, sign_in("aal2")).await;
    let alice = token(&app, sign_in_request("alice", "right")).await;
    let ada_id = id_of(&app, &alice, "ada@example.com").await;

    for own in [&bob, &ada] {
        for (method, uri) in [
            ("GET", "/api/users".to_owned()),
            ("POST", "/api/users/invite".to_owned()),
            ("POST", format!("/api/users/{ada_id}/block")),
            ("DELETE", format!("/api/users/{ada_id}/sessions")),
            ("POST", format!("/api/users/{ada_id}/recovery")),
            ("DELETE", format!("/api/users/{ada_id}")),
        ] {
            let body = (method == "POST").then(|| json!({ "email": "eve@example.com" }));
            let response = send(&app, authed(method, &uri, body, own)).await;
            assert_eq!(response.status, StatusCode::FORBIDDEN, "{method} {uri}");
        }
    }
    assert!(taken(&calls).is_empty());

    let listed: Vec<(String, String, bool, i64)> = users(&app, &alice)
        .await
        .iter()
        .map(|u| {
            (
                u["username"].as_str().unwrap().to_owned(),
                u["kind"].as_str().unwrap().to_owned(),
                u["blocked"].as_bool().unwrap(),
                u["sessions"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("ada@example.com".to_owned(), "local".to_owned(), false, 1),
            ("alice".to_owned(), "directory".to_owned(), false, 1),
            ("bob".to_owned(), "directory".to_owned(), false, 1),
        ]
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_blocked_user_loses_their_sessions_and_stays_out(pool: PgPool) {
    let (app, calls) = setup(pool).await;
    let bob = token(&app, sign_in_request("bob", "right")).await;
    let ada = token(&app, sign_in("aal2")).await;
    let alice = token(&app, sign_in_request("alice", "right")).await;
    let bob_id = id_of(&app, &alice, "bob").await;
    let ada_id = id_of(&app, &alice, "ada@example.com").await;
    let alice_id = id_of(&app, &alice, "alice").await;

    let block =
        |id: &str, method: &str| authed(method, &format!("/api/users/{id}/block"), None, &alice);
    assert_eq!(
        send(&app, block(&bob_id, "POST")).await.status,
        StatusCode::NO_CONTENT
    );
    assert!(
        taken(&calls).is_empty(),
        "a directory user is not in Kratos"
    );
    assert_eq!(
        send(&app, block(&ada_id, "POST")).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        taken(&calls),
        [
            format!("PATCH /admin/identities/{ADA} inactive"),
            format!("DELETE /admin/identities/{ADA}/sessions"),
        ]
    );
    // Nobody locks themselves out.
    assert_eq!(
        send(&app, block(&alice_id, "POST")).await.status,
        StatusCode::FORBIDDEN
    );

    for (name, own) in [("bob", &bob), ("ada", &ada)] {
        let response = send(&app, get_request("/api/session", Some(own))).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED, "{name}");
    }
    for (name, again) in [
        ("bob", sign_in_request("bob", "right")),
        ("ada", sign_in("aal2")),
    ] {
        let response = send(&app, again).await;
        assert_eq!(
            (response.status, response.code().as_str()),
            (StatusCode::FORBIDDEN, "account_disabled"),
            "{name}"
        );
        assert!(response.session_token().is_none(), "{name}");
    }
    let blocked: Vec<(String, i64)> = users(&app, &alice)
        .await
        .iter()
        .filter(|u| u["blocked"] == true)
        .map(|u| {
            (
                u["username"].as_str().unwrap().to_owned(),
                u["sessions"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        blocked,
        [("ada@example.com".to_owned(), 0), ("bob".to_owned(), 0)]
    );

    assert_eq!(
        send(&app, block(&bob_id, "DELETE")).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&app, block(&ada_id, "DELETE")).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        taken(&calls),
        [format!("PATCH /admin/identities/{ADA} active")]
    );
    token(&app, sign_in_request("bob", "right")).await;
    token(&app, sign_in("aal2")).await;

    let mut actions = actions(&app, &alice).await;
    actions.sort();
    assert_eq!(
        actions,
        [
            "user.blocked",
            "user.blocked",
            "user.unblocked",
            "user.unblocked"
        ]
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn sessions_end_on_request(pool: PgPool) {
    let (app, calls) = setup(pool).await;
    let bob = token(&app, sign_in_request("bob", "right")).await;
    let ada = token(&app, sign_in("aal2")).await;
    let alice = token(&app, sign_in_request("alice", "right")).await;
    for (name, own) in [("bob", &bob), ("ada@example.com", &ada)] {
        let id = id_of(&app, &alice, name).await;
        let request = authed("DELETE", &format!("/api/users/{id}/sessions"), None, &alice);
        assert_eq!(send(&app, request).await.status, StatusCode::NO_CONTENT);
        let response = send(&app, get_request("/api/session", Some(own))).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED, "{name}");
    }
    assert_eq!(
        taken(&calls),
        [format!("DELETE /admin/identities/{ADA}/sessions")]
    );
    // Ending sessions does not block: both sign in again.
    token(&app, sign_in_request("bob", "right")).await;
    token(&app, sign_in("aal2")).await;
    assert_eq!(actions(&app, &alice).await, ["user.sessions_ended"; 2]);
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn only_local_accounts_get_a_recovery_code_or_are_deleted(pool: PgPool) {
    let (app, calls) = setup(pool).await;
    token(&app, sign_in_request("bob", "right")).await;
    let ada = token(&app, sign_in("aal2")).await;
    let alice = token(&app, sign_in_request("alice", "right")).await;
    let bob_id = id_of(&app, &alice, "bob").await;
    let ada_id = id_of(&app, &alice, "ada@example.com").await;

    for (method, uri) in [
        ("POST", format!("/api/users/{bob_id}/recovery")),
        ("DELETE", format!("/api/users/{bob_id}")),
    ] {
        let response = send(&app, authed(method, &uri, None, &alice)).await;
        assert_eq!(
            (response.status, response.code().as_str()),
            (StatusCode::BAD_REQUEST, "invalid_request"),
            "{method} {uri}"
        );
    }
    assert!(taken(&calls).is_empty());

    let request = authed(
        "POST",
        &format!("/api/users/{ada_id}/recovery"),
        None,
        &alice,
    );
    let response = send(&app, request).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.json(),
        json!({
            "link": "https://remotehub.test/sign-in/recovery?flow=f",
            "code": "123456",
            "expires_at": "2026-09-27T00:00:00Z",
        })
    );
    assert_eq!(
        taken(&calls),
        [
            format!("DELETE /admin/identities/{ADA}/credentials/totp"),
            format!("DELETE /admin/identities/{ADA}/credentials/lookup_secret"),
            format!("DELETE /admin/identities/{ADA}/credentials/webauthn"),
            format!("DELETE /admin/identities/{ADA}/sessions"),
            "POST /admin/recovery/code".to_owned(),
        ]
    );
    let response = send(&app, get_request("/api/session", Some(&ada))).await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);

    let request = authed("DELETE", &format!("/api/users/{ada_id}"), None, &alice);
    assert_eq!(send(&app, request).await.status, StatusCode::NO_CONTENT);
    assert_eq!(taken(&calls), [format!("DELETE /admin/identities/{ADA}")]);
    assert!(
        users(&app, &alice)
            .await
            .iter()
            .all(|u| u["username"] != "ada@example.com")
    );
    let request = authed("DELETE", &format!("/api/users/{ada_id}"), None, &alice);
    assert_eq!(send(&app, request).await.status, StatusCode::NOT_FOUND);
    assert_eq!(
        actions(&app, &alice).await,
        ["account.deleted", "account.recovery_issued"]
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn an_invitation_hands_out_its_code_once(pool: PgPool) {
    let (app, calls) = setup(pool).await;
    let alice = token(&app, sign_in_request("alice", "right")).await;
    let invite = |body: Value| authed("POST", "/api/users/invite", Some(body), &alice);

    for email in ["", "no-at-sign", "two words@example.com"] {
        let response = send(&app, invite(json!({ "email": email }))).await;
        assert_eq!(
            (response.status, response.json()["params"]["field"].as_str()),
            (StatusCode::BAD_REQUEST, Some("email")),
            "{email:?}"
        );
    }
    assert!(taken(&calls).is_empty());

    let response = send(&app, invite(json!({ "email": TAKEN }))).await;
    assert_eq!(
        (response.status, response.code().as_str()),
        (StatusCode::CONFLICT, "name_taken")
    );
    taken(&calls);

    let response = send(
        &app,
        invite(json!({ "email": " Eve@Example.com ", "name": "Eve" })),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED);
    assert_eq!(response.json()["code"], "123456");
    assert_eq!(
        taken(&calls),
        ["POST /admin/identities", "POST /admin/recovery/code"]
    );
    let log = send(&app, get_request("/api/audit", Some(&alice)))
        .await
        .json();
    let invited = log
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["action"] == "account.invited")
        .unwrap();
    assert_eq!(
        invited["details"],
        json!({ "email": "eve@example.com", "identity_id": INVITED })
    );
    assert!(
        !log.to_string().contains("123456"),
        "the code is not logged"
    );

    // The account is listed before its first sign-in, under its address.
    let invited: Vec<Value> = users(&app, &alice)
        .await
        .into_iter()
        .filter(|u| u["kind"] == "local")
        .collect();
    assert_eq!(invited.len(), 1);
    assert_eq!(
        (
            &invited[0]["username"],
            &invited[0]["display_name"],
            &invited[0]["last_sign_in_at"]
        ),
        (&json!("eve@example.com"), &json!("Eve"), &Value::Null)
    );
}
