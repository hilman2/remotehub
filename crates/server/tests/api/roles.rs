//! Roles for remotehub itself (#106): alice administers through RH Admins
//! (fixture `set_up`), bob is a directory user without groups, olaf is in
//! RH Operators, Ada is a local account.

use axum::Router;
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::accounts::{ADA, setup, sign_in};
use crate::common::{BOB_SID, OPS_SID, Response, authed, send, sign_in_request};

async fn token(app: &Router, request: axum::http::Request<axum::body::Body>) -> (String, Value) {
    let response = send(app, request).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json());
    (response.session_token().unwrap(), response.json())
}

async fn call(app: &Router, token: &str, method: &str, uri: &str, body: Option<Value>) -> Response {
    send(app, authed(method, uri, body, token)).await
}

async fn assign(app: &Router, admin: &str, role: &str, sid: &str, kind: &str) -> Response {
    let uri = format!("/api/roles/{role}/members/{sid}");
    let body = json!({ "principal_kind": kind, "principal_name": "someone" });
    call(app, admin, "PUT", &uri, Some(body)).await
}

/// `admin` and `roles` of `/api/session` for `token`.
async fn me(app: &Router, token: &str) -> (Value, Value) {
    let me = call(app, token, "GET", "/api/session", None).await.json();
    (me["admin"].clone(), me["roles"].clone())
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn an_auditor_reads_the_audit_log_and_nothing_else(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let (alice, _) = token(&app, sign_in_request("alice", "right")).await;
    let (bob, _) = token(&app, sign_in_request("bob", "right")).await;
    for (method, uri) in [("GET", "/api/audit"), ("POST", "/api/audit/verify")] {
        let response = call(&app, &bob, method, uri, None).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    let response = call(&app, &bob, "GET", "/api/roles", None).await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
    let response = assign(&app, &bob, "auditor", BOB_SID, "user").await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let response = assign(&app, &alice, "auditor", BOB_SID, "user").await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{}",
        response.json()
    );
    // The open session has the role at once.
    assert_eq!(me(&app, &bob).await, (json!(false), json!(["auditor"])));
    for (method, uri) in [("GET", "/api/audit"), ("POST", "/api/audit/verify")] {
        let response = call(&app, &bob, method, uri, None).await;
        assert_eq!(response.status, StatusCode::OK, "{method} {uri}");
    }
    for (method, uri) in [("GET", "/api/users"), ("GET", "/api/roles")] {
        let response = call(&app, &bob, method, uri, None).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    // A new sign-in answers with the role right away.
    let (_, signed_in) = token(&app, sign_in_request("bob", "right")).await;
    assert_eq!(signed_in["roles"], json!(["auditor"]));

    let uri = format!("/api/roles/auditor/members/{BOB_SID}");
    let response = call(&app, &alice, "DELETE", &uri, None).await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);
    assert_eq!(me(&app, &bob).await, (json!(false), json!([])));
    let response = call(&app, &bob, "GET", "/api/audit", None).await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
    let response = call(&app, &alice, "DELETE", &uri, None).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);

    let log = call(&app, &alice, "GET", "/api/audit", None).await.json();
    let actions: Vec<&str> = log
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["action"].as_str())
        .filter(|a| a.starts_with("role."))
        .collect();
    assert_eq!(actions, ["role.revoked", "role.assigned"]);
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn roles_reach_members_of_groups_and_local_accounts(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let (alice, _) = token(&app, sign_in_request("alice", "right")).await;
    let (olaf, _) = token(&app, sign_in_request("olaf", "right")).await;
    let (ada, _) = token(&app, sign_in("aal2")).await;

    // A directory group, through a group of remotehub's own.
    let group = call(
        &app,
        &alice,
        "POST",
        "/api/groups",
        Some(json!({ "name": "Admins" })),
    )
    .await;
    let group = group.json()["id"].as_str().unwrap().to_owned();
    let uri = format!("/api/groups/{group}/members/{OPS_SID}");
    let member = json!({ "principal_kind": "group", "principal_name": "RH Operators" });
    call(&app, &alice, "PUT", &uri, Some(member)).await;
    let response = assign(
        &app,
        &alice,
        "administrator",
        &format!("group:{group}"),
        "group",
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{}",
        response.json()
    );
    assert_eq!(
        me(&app, &olaf).await,
        (json!(true), json!(["administrator"]))
    );
    assert_eq!(
        call(&app, &olaf, "GET", "/api/users", None).await.status,
        StatusCode::OK
    );

    let response = assign(
        &app,
        &alice,
        "security_officer",
        &format!("local:{ADA}"),
        "user",
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{}",
        response.json()
    );
    assert_eq!(
        me(&app, &ada).await,
        (json!(false), json!(["security_officer"]))
    );

    let listed = call(&app, &alice, "GET", "/api/roles", None).await.json();
    let members: Vec<(String, usize)> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["role"].as_str().unwrap().to_owned(),
                r["members"].as_array().unwrap().len(),
            )
        })
        .collect();
    assert_eq!(
        members,
        [
            // With the two of the fixture.
            ("administrator".to_owned(), 3),
            ("auditor".to_owned(), 0),
            ("security_officer".to_owned(), 1)
        ]
    );

    // A deleted group takes its roles along.
    call(
        &app,
        &alice,
        "DELETE",
        &format!("/api/groups/{group}"),
        None,
    )
    .await;
    assert_eq!(me(&app, &olaf).await, (json!(false), json!([])));
    let listed = call(&app, &alice, "GET", "/api/roles", None).await.json();
    let administrators: Vec<&str> = listed[0]["members"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["sid"].as_str())
        .collect();
    assert_eq!(administrators.len(), 2, "{listed}");
    assert!(!administrators.contains(&format!("group:{group}").as_str()));
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_last_administrator_keeps_the_role(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let (alice, _) = token(&app, sign_in_request("alice", "right")).await;
    let revoke = |sid: &str| format!("/api/roles/administrator/members/{sid}");
    let local = revoke("local:5b0c3cb1-7a0e-4a5e-9d0a-0000000000a2");
    let local = call(&app, &alice, "DELETE", &local, None).await;
    assert_eq!(local.status, StatusCode::NO_CONTENT, "{}", local.json());
    let last = call(&app, &alice, "DELETE", &revoke("S-1-5-21-1-2-3-1201"), None).await;
    assert_eq!(last.status, StatusCode::CONFLICT);
    assert_eq!(last.code(), "last_administrator");
    // Nothing changed: alice still administers.
    assert_eq!(me(&app, &alice).await.0, json!(true));
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn roles_go_only_to_principals_that_exist(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let (alice, _) = token(&app, sign_in_request("alice", "right")).await;
    let response = assign(&app, &alice, "janitor", BOB_SID, "user").await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    for (sid, kind) in [
        ("local:00000000-0000-4000-8000-00000000dead", "user"),
        ("group:00000000-0000-4000-8000-00000000dead", "group"),
        ("bob", "user"),
    ] {
        let response = assign(&app, &alice, "auditor", sid, kind).await;
        assert_eq!(
            (response.status, response.json()["params"]["field"].as_str()),
            (StatusCode::BAD_REQUEST, Some("principal_sid")),
            "{sid}"
        );
    }
}
