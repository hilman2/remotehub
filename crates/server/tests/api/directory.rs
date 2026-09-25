//! The directory connection on the settings page (#144): only administrators
//! set it, a check stands before every save, the password never comes back,
//! and a saved change signs in from the next request on.

use axum::Router;
use axum::http::StatusCode;
use remotehub_server::break_glass;
use remotehub_server::{AppState, app};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{ORIGIN, authed, get, json, send, sign_in_request, state};

/// An administrator that outlives every directory session: a break-glass
/// account.
async fn administrator(pool: &PgPool, state: &AppState, app: &Router) -> String {
    let issued = break_glass::create(pool, &state.vault, "keeper")
        .await
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let body = json!({
        "username": "keeper",
        "password": issued.password.as_str(),
        "code": break_glass::code_at(&issued.totp_secret, now).unwrap(),
    });
    send(
        app,
        json("POST", "/api/session/break-glass", body, Some(ORIGIN)),
    )
    .await
    .session_token()
    .expect("the break-glass account signs in")
}

/// The test lab's domain controller, as the settings page would send it.
fn lab(overrides: Value) -> Value {
    let mut settings = json!({
        "url": std::env::var("REMOTEHUB_TEST_LDAP_URL")
            .expect("REMOTEHUB_TEST_LDAP_URL points to the test lab's domain controller"),
        "bind_dn": "svc-remotehub@remotehub.test",
        "password": "Svc-Passw0rd!",
        "base_dn": "DC=remotehub,DC=test",
    });
    for (key, value) in overrides.as_object().unwrap() {
        settings[key] = value.clone();
    }
    settings
}

fn field(response: &crate::common::Response) -> String {
    response.json()["params"]["field"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn only_administrators_set_the_directory_and_the_form_is_checked(pool: PgPool) {
    let app = app(state(pool), None);
    let bob = send(&app, sign_in_request("bob", "right"))
        .await
        .session_token()
        .unwrap();
    let settings =
        json!({ "url": "ldaps://dc", "bind_dn": "x", "password": "y", "base_dn": "DC=x" });
    for (method, uri) in [
        ("GET", "/api/settings/directory"),
        ("PUT", "/api/settings/directory"),
        ("POST", "/api/settings/directory/check"),
        ("DELETE", "/api/settings/directory"),
    ] {
        let body = (method != "GET" && method != "DELETE").then(|| settings.clone());
        let response = send(&app, authed(method, uri, body, &bob)).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{method} {uri}");
    }

    let alice = send(&app, sign_in_request("alice", "right"))
        .await
        .session_token()
        .unwrap();
    for (change, wrong) in [
        (json!({ "url": "https://dc" }), "url"),
        (json!({ "url": "ldap://dc" }), "starttls"),
        (json!({ "bind_dn": " " }), "bind_dn"),
        (json!({ "base_dn": "" }), "base_dn"),
        (json!({ "ca_pem": "not a certificate" }), "ca_pem"),
        (json!({ "user_filter": "memberOf=x" }), "user_filter"),
        (json!({ "timeout_seconds": 0 }), "timeout_seconds"),
        // Nothing stored yet stands in for a missing password.
        (json!({ "password": "" }), "password"),
    ] {
        let mut body = settings.clone();
        for (key, value) in change.as_object().unwrap() {
            body[key] = value.clone();
        }
        let response = send(
            &app,
            authed("PUT", "/api/settings/directory", Some(body), &alice),
        )
        .await;
        assert_eq!(response.code(), "invalid_request", "{change}");
        assert_eq!(field(&response), wrong, "{change}");
    }
    let none = send(
        &app,
        authed("DELETE", "/api/settings/directory", None, &alice),
    )
    .await;
    assert_eq!(none.status, StatusCode::NOT_FOUND);
    let stored = send(&app, authed("GET", "/api/settings/directory", None, &alice)).await;
    assert_eq!(stored.json(), json!({ "connection": null }));
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
#[ignore = "needs the test lab"]
async fn the_directory_is_checked_stored_and_used_without_a_restart(pool: PgPool) {
    // No directory at first: nobody signs in as a directory user.
    let state = AppState::new(
        pool.clone(),
        None,
        crate::common::settings(),
        crate::common::vault(),
    );
    let app = app(state.clone(), None);
    let admin = administrator(&pool, &state, &app).await;
    let lab_alice = || sign_in_request("alice", "Alice-Passw0rd!");
    assert_eq!(
        send(&app, lab_alice()).await.code(),
        "directory_unavailable"
    );

    // The system's roots do not know the lab's CA: the check says so, and
    // offers the certificate the domain controller presented.
    let checked = send(
        &app,
        authed(
            "POST",
            "/api/settings/directory/check",
            Some(lab(json!({}))),
            &admin,
        ),
    )
    .await
    .json();
    assert_eq!(checked["failure"]["step"], "tls", "{checked}");
    assert_eq!(checked["failure"]["reason"], "unknown_ca");
    let presented = checked["failure"]["ca"]["pem"].as_str().unwrap().to_owned();

    // A wrong password fails the check, and nothing is stored.
    let wrong = send(
        &app,
        authed(
            "PUT",
            "/api/settings/directory",
            Some(lab(json!({ "ca_pem": presented, "password": "wrong" }))),
            &admin,
        ),
    )
    .await;
    assert_eq!(wrong.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(wrong.json()["params"]["step"], "bind");
    assert_eq!(wrong.json()["params"]["reason"], "invalid_credentials");
    let stored = send(&app, authed("GET", "/api/settings/directory", None, &admin)).await;
    assert_eq!(stored.json(), json!({ "connection": null }));

    let saved = send(
        &app,
        authed(
            "PUT",
            "/api/settings/directory",
            Some(lab(json!({ "ca_pem": presented }))),
            &admin,
        ),
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.json());
    assert!(saved.json()["users"].as_u64().unwrap() >= 5);

    // The password never comes back.
    let stored = send(&app, authed("GET", "/api/settings/directory", None, &admin)).await;
    let text = String::from_utf8(stored.body.clone()).unwrap();
    assert!(
        !text.contains("Svc-Passw0rd!") && !text.contains("password"),
        "{text}"
    );
    assert_eq!(
        stored.json()["connection"]["bind_dn"],
        "svc-remotehub@remotehub.test"
    );
    let sealed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM secret_fields WHERE field = 'bind_password'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(sealed, 1);

    // Without a restart, the lab's alice signs in.
    let alice = send(&app, lab_alice()).await;
    assert_eq!(alice.status, StatusCode::OK, "{}", alice.json());
    let alice = alice.session_token().unwrap();

    // Saved again without a password: the stored one serves, and the
    // session stays, since URL and base DN are the same.
    let again = send(
        &app,
        authed(
            "PUT",
            "/api/settings/directory",
            Some(lab(
                json!({ "ca_pem": presented, "password": null, "timeout_seconds": 5 }),
            )),
            &admin,
        ),
    )
    .await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.json());
    assert_eq!(
        send(&app, get("/api/session", Some(&alice))).await.status,
        StatusCode::OK
    );

    // Another base DN ends every directory session.
    let moved = send(
        &app,
        authed(
            "PUT",
            "/api/settings/directory",
            Some(lab(json!({
                "ca_pem": presented,
                "password": null,
                "timeout_seconds": 5,
                "base_dn": "CN=Users,DC=remotehub,DC=test",
            }))),
            &admin,
        ),
    )
    .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.json());
    assert_eq!(
        send(&app, get("/api/session", Some(&alice))).await.status,
        StatusCode::UNAUTHORIZED
    );

    // Removed, the directory is gone at once, and so are the sessions.
    let alice = send(&app, lab_alice()).await.session_token().unwrap();
    let removed = send(
        &app,
        authed("DELETE", "/api/settings/directory", None, &admin),
    )
    .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert_eq!(
        send(&app, get("/api/session", Some(&alice))).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&app, lab_alice()).await.code(),
        "directory_unavailable"
    );
    let sealed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM secret_fields WHERE field = 'bind_password'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(sealed, 0);

    let actions: Vec<(String, Value)> = sqlx::query_as(
        "SELECT action, details FROM audit_log WHERE action LIKE 'directory.%' ORDER BY seq",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(actions.len(), 4, "{actions:?}");
    assert_eq!(actions[1].1["fields"], json!(["timeout_seconds"]));
    assert_eq!(actions[2].1["fields"], json!(["base_dn"]));
    assert_eq!(actions[2].1["sessions_ended"], 1);
    assert!(!format!("{actions:?}").contains("Svc-Passw0rd!"));
}
