//! The setup wizard (#143): a fresh installation serves only the wizard, the
//! setup code makes exactly one administrator, and once setup is complete it
//! never opens again.

use axum::Router;
use axum::http::{StatusCode, header};
use remotehub_server::{break_glass, setup};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::accounts::{self, Calls, INVITED};
use crate::common::{ORIGIN, authed, get, json, send, sign_in_request, vault};

fn administrator(code: &str, email: &str) -> axum::http::Request<axum::body::Body> {
    json(
        "POST",
        "/api/setup/administrator",
        json!({ "code": code, "email": email, "name": "Ines Initial" }),
        Some(ORIGIN),
    )
}

/// A fresh installation with the stand-in for Kratos, and its setup code.
async fn fresh(pool: &PgPool) -> (Router, Calls, String) {
    let (app, calls) = accounts::setup(pool.clone()).await;
    let code = setup::new_code(pool).await.unwrap();
    (app, calls, code)
}

/// Step 1 done, and the new administrator signed in: their session token.
async fn administrator_signed_in(app: &Router, code: &str) -> String {
    let created = send(app, administrator(code, "ines@example.com")).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json());
    send(app, accounts::sign_in("invited"))
        .await
        .session_token()
        .expect("the administrator signs in")
}

async fn phase(app: &Router) -> Value {
    send(app, get("/api/setup", None)).await.json()
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_fresh_installation_serves_only_the_wizard(pool: PgPool) {
    let (app, _, _) = fresh(&pool).await;
    assert_eq!(phase(&app).await, json!({ "phase": "pending", "step": 0 }));
    assert_eq!(
        send(&app, get("/api/health", None)).await.status,
        StatusCode::OK
    );
    // Signing in works, but the session reaches nothing.
    let alice = send(&app, sign_in_request("alice", "right"))
        .await
        .session_token()
        .unwrap();
    for request in [
        get("/api/tree", None),
        authed("GET", "/api/tree", None, &alice),
        authed("GET", "/api/users", None, &alice),
        authed(
            "PUT",
            "/api/roles/administrator/members/S-1-5-21-1-2-3-1105",
            Some(json!({})),
            &alice,
        ),
    ] {
        let response = send(&app, request).await;
        assert_eq!(response.status, StatusCode::CONFLICT);
        assert_eq!(response.code(), "setup_pending");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_wrong_code_is_refused_and_counted(pool: PgPool) {
    let (app, calls, code) = fresh(&pool).await;
    let check = |code: &str| {
        json(
            "POST",
            "/api/setup/code",
            json!({ "code": code }),
            Some(ORIGIN),
        )
    };
    assert_eq!(
        send(&app, check(&code)).await.status,
        StatusCode::NO_CONTENT
    );
    let wrong = send(&app, administrator("wrong", "ines@example.com")).await;
    assert_eq!(wrong.status, StatusCode::FORBIDDEN);
    assert_eq!(wrong.code(), "setup_code_invalid");
    // Kratos never heard of it.
    assert!(calls.lock().unwrap().is_empty());

    for _ in 0..29 {
        assert_eq!(
            send(&app, check("wrong")).await.code(),
            "setup_code_invalid"
        );
    }
    // Used up: even the right code waits now.
    assert_eq!(send(&app, check(&code)).await.code(), "too_many_attempts");
    assert_eq!(
        send(&app, administrator(&code, "ines@example.com"))
            .await
            .code(),
        "too_many_attempts"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_code_makes_one_administrator(pool: PgPool) {
    let (app, _, code) = fresh(&pool).await;
    let (first, second) = tokio::join!(
        send(&app, administrator(&code, "ines@example.com")),
        send(&app, administrator(&code, "ivan@example.com")),
    );
    let mut statuses = [first.status, second.status];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::CREATED, StatusCode::FORBIDDEN]);
    let created = if first.status == StatusCode::CREATED {
        first
    } else {
        second
    };
    assert_eq!(created.json()["code"], "123456");

    let administrators: Vec<String> = sqlx::query_scalar(
        "SELECT principal_sid FROM role_assignments WHERE role = 'administrator'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(administrators, [format!("local:{INVITED}")]);
    assert_eq!(
        phase(&app).await,
        json!({ "phase": "administrator", "step": 1 })
    );
    // The code is spent.
    let again = send(&app, administrator(&code, "ines@example.com")).await;
    assert_eq!(again.code(), "setup_code_invalid");
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_new_code_replaces_an_administrator_who_never_signed_in(pool: PgPool) {
    let (app, calls, code) = fresh(&pool).await;
    let first = send(&app, administrator(&code, "ines@exmaple.com")).await;
    assert_eq!(first.status, StatusCode::CREATED);

    let code = setup::new_code(&pool).await.unwrap();
    calls.lock().unwrap().clear();
    let second = send(&app, administrator(&code, "ines@example.com")).await;
    assert_eq!(second.status, StatusCode::CREATED, "{}", second.json());
    assert_eq!(
        calls.lock().unwrap().first().map(String::as_str),
        Some(format!("DELETE /admin/identities/{INVITED}").as_str())
    );
    let accounts: Vec<(String, String)> =
        sqlx::query_as("SELECT kind, username FROM users ORDER BY created_at")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        accounts,
        [
            ("deleted".to_owned(), "ines@exmaple.com".to_owned()),
            ("local".to_owned(), "ines@example.com".to_owned()),
        ]
    );

    // Once the administrator has signed in, a new code changes nothing.
    send(&app, accounts::sign_in("invited")).await;
    let code = setup::new_code(&pool).await.unwrap();
    let third = send(&app, administrator(&code, "ivan@example.com")).await;
    assert_eq!(third.code(), "setup_done");
}

#[sqlx::test(migrations = "../../migrations")]
async fn setup_makes_no_break_glass_account_when_there_is_one(pool: PgPool) {
    let (app, _, code) = fresh(&pool).await;
    let token = administrator_signed_in(&app, &code).await;
    break_glass::create(&pool, &vault(), "night-shift")
        .await
        .unwrap();
    let response = send(&app, authed("POST", "/api/setup/break-glass", None, &token)).await;
    assert_eq!(response.code(), "break_glass_exists");
    assert_eq!(break_glass::list(&pool).await.unwrap().len(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_administrator_finishes_setup_and_it_stays_closed(pool: PgPool) {
    let (app, _, code) = fresh(&pool).await;
    let token = administrator_signed_in(&app, &code).await;

    // Others with a session are no administrators.
    let alice = send(&app, sign_in_request("alice", "right"))
        .await
        .session_token()
        .unwrap();
    let refused = send(
        &app,
        authed("PUT", "/api/setup/step", Some(json!({ "step": 3 })), &alice),
    )
    .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);

    let step = send(
        &app,
        authed("PUT", "/api/setup/step", Some(json!({ "step": 3 })), &token),
    )
    .await;
    assert_eq!(step.status, StatusCode::NO_CONTENT);
    assert_eq!(phase(&app).await["step"], 3);

    let issued = send(&app, authed("POST", "/api/setup/break-glass", None, &token)).await;
    assert_eq!(issued.status, StatusCode::CREATED);
    assert_eq!(issued.headers[header::CACHE_CONTROL], "no-store");
    let issued = issued.json();
    assert_eq!(issued["username"], "emergency");
    assert_eq!(issued["password"].as_str().unwrap().len(), 24);
    assert!(
        issued["totp_uri"]
            .as_str()
            .unwrap()
            .starts_with("otpauth://totp/")
    );
    let twice = send(&app, authed("POST", "/api/setup/break-glass", None, &token)).await;
    assert_eq!(twice.code(), "break_glass_exists");

    let done = send(&app, authed("POST", "/api/setup/complete", None, &token)).await;
    assert_eq!(done.status, StatusCode::NO_CONTENT);
    assert_eq!(phase(&app).await, json!({ "phase": "complete", "step": 6 }));
    let actions: Vec<String> = sqlx::query_scalar("SELECT action FROM audit_log ORDER BY seq")
        .fetch_all(&pool)
        .await
        .unwrap();
    for action in [
        "account.invited",
        "role.assigned",
        "break_glass.created",
        "setup.completed",
    ] {
        assert!(
            actions.iter().any(|a| a == action),
            "{action} in {actions:?}"
        );
    }

    // Closed for good: no code, no step, no second break-glass account.
    assert!(matches!(
        setup::new_code(&pool).await,
        Err(setup::SetupError::Complete)
    ));
    for (method, uri) in [
        ("POST", "/api/setup/complete"),
        ("POST", "/api/setup/break-glass"),
    ] {
        let response = send(&app, authed(method, uri, None, &token)).await;
        assert_eq!(response.code(), "setup_done", "{method} {uri}");
    }
    let refused = send(
        &app,
        authed("PUT", "/api/setup/step", Some(json!({ "step": 2 })), &token),
    )
    .await;
    assert_eq!(refused.code(), "setup_done");
    assert_eq!(
        send(&app, authed("GET", "/api/tree", None, &token))
            .await
            .status,
        StatusCode::OK
    );
}
