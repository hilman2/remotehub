//! Groups of remotehub's own (#105): alice administers, bob is a directory
//! user without groups, olaf is in the directory group RH Operators, Ada is
//! a local account (the stand-in for Kratos from `accounts`).

use axum::Router;
use axum::http::StatusCode;
use remotehub_server::{AppState, break_glass};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::accounts::{ADA, setup, sign_in};
use crate::common::{
    BOB_SID, OPS_SID, ORIGIN, Response, authed, get, json as json_request, send, settings,
    sign_in_request, vault,
};

/// An ID nothing has.
const NOBODY: &str = "00000000-0000-4000-8000-00000000dead";

async fn token(app: &Router, request: axum::http::Request<axum::body::Body>) -> String {
    let response = send(app, request).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json());
    response.session_token().unwrap()
}

async fn call(app: &Router, token: &str, method: &str, uri: &str, body: Option<Value>) -> Response {
    send(app, authed(method, uri, body, token)).await
}

async fn created(app: &Router, token: &str, uri: &str, body: Value) -> String {
    let response = call(app, token, "POST", uri, Some(body)).await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json());
    response.json()["id"].as_str().unwrap().to_owned()
}

async fn add_member(app: &Router, admin: &str, group: &str, sid: &str, kind: &str) -> Response {
    let uri = format!("/api/groups/{group}/members/{sid}");
    let body = json!({ "principal_kind": kind, "principal_name": "someone" });
    call(app, admin, "PUT", &uri, Some(body)).await
}

/// The names of the top folders `token` sees.
async fn folders(app: &Router, token: &str) -> Vec<String> {
    call(app, token, "GET", "/api/tree", None).await.json()["folders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap().to_owned())
        .collect()
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn only_administrators_manage_groups(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let alice = token(&app, sign_in_request("alice", "right")).await;
    let bob = token(&app, sign_in_request("bob", "right")).await;
    for (method, body) in [("GET", None), ("POST", Some(json!({ "name": "Web" })))] {
        let response = call(&app, &bob, method, "/api/groups", body).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{method}");
    }

    let web = created(&app, &alice, "/api/groups", json!({ "name": "Web team" })).await;
    let taken = call(
        &app,
        &alice,
        "POST",
        "/api/groups",
        Some(json!({ "name": "WEB TEAM" })),
    )
    .await;
    assert_eq!(
        (taken.status, taken.code().as_str()),
        (StatusCode::CONFLICT, "name_taken")
    );
    let ops = created(&app, &alice, "/api/groups", json!({ "name": "Ops" })).await;
    let rename = json!({ "name": "Web team", "description": "Agency sites" });
    let clash = call(
        &app,
        &alice,
        "PATCH",
        &format!("/api/groups/{ops}"),
        Some(rename.clone()),
    )
    .await;
    assert_eq!(clash.code(), "name_taken");
    let response = call(
        &app,
        &alice,
        "PATCH",
        &format!("/api/groups/{web}"),
        Some(rename),
    )
    .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);

    let listed = call(&app, &alice, "GET", "/api/groups", None).await.json();
    assert_eq!(
        listed,
        json!([
            { "id": ops, "name": "Ops", "description": "", "members": [] },
            { "id": web, "name": "Web team", "description": "Agency sites", "members": [] },
        ])
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_grant_to_a_group_reaches_its_members_at_once(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let alice = token(&app, sign_in_request("alice", "right")).await;
    let bob = token(&app, sign_in_request("bob", "right")).await;
    let olaf = token(&app, sign_in_request("olaf", "right")).await;
    let ada = token(&app, sign_in("aal2")).await;
    let folder = created(
        &app,
        &alice,
        "/api/folders",
        json!({ "parent_id": null, "name": "Sites" }),
    )
    .await;
    let group = created(&app, &alice, "/api/groups", json!({ "name": "Web team" })).await;
    let grant = json!({
        "object": { "kind": "folder", "id": folder },
        "principal_kind": "group", "principal_sid": format!("group:{group}"),
        "principal_name": "Web team", "role": "list",
    });
    let response = call(&app, &alice, "POST", "/api/grants", Some(grant)).await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{}",
        response.json()
    );
    for (name, own) in [("bob", &bob), ("olaf", &olaf), ("ada", &ada)] {
        assert!(folders(&app, own).await.is_empty(), "{name}");
    }

    // A directory user, a directory group and a local account; the open
    // sessions see the folder without signing in again.
    for (sid, kind) in [
        (BOB_SID.to_owned(), "user"),
        (OPS_SID.to_owned(), "group"),
        (format!("local:{ADA}"), "user"),
    ] {
        let response = add_member(&app, &alice, &group, &sid, kind).await;
        assert_eq!(
            response.status,
            StatusCode::NO_CONTENT,
            "{sid}: {}",
            response.json()
        );
    }
    for (name, own) in [("bob", &bob), ("olaf", &olaf), ("ada", &ada)] {
        assert_eq!(folders(&app, own).await, ["Sites"], "{name}");
    }

    let uri = format!("/api/groups/{group}/members/{BOB_SID}");
    assert_eq!(
        call(&app, &alice, "DELETE", &uri, None).await.status,
        StatusCode::NO_CONTENT
    );
    assert!(folders(&app, &bob).await.is_empty());
    assert_eq!(
        call(&app, &alice, "DELETE", &uri, None).await.status,
        StatusCode::NOT_FOUND
    );

    // Deleting the group takes its grant along.
    let uri = format!("/api/groups/{group}");
    assert_eq!(
        call(&app, &alice, "DELETE", &uri, None).await.status,
        StatusCode::NO_CONTENT
    );
    assert!(folders(&app, &olaf).await.is_empty());
    let grants = call(
        &app,
        &alice,
        "GET",
        &format!("/api/grants?kind=folder&id={folder}"),
        None,
    )
    .await;
    assert_eq!(grants.json()["direct"], json!([]), "{}", grants.json());

    let actions: Vec<String> = call(&app, &alice, "GET", "/api/audit", None)
        .await
        .json()
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["action"].as_str())
        .filter(|a| a.starts_with("group."))
        .map(str::to_owned)
        .collect();
    assert_eq!(
        actions,
        [
            "group.deleted",
            "group.member_removed",
            "group.member_added",
            "group.member_added",
            "group.member_added",
            "group.created",
        ]
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn members_are_users_and_directory_groups_that_exist(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let alice = token(&app, sign_in_request("alice", "right")).await;
    token(&app, sign_in("aal2")).await;
    let group = created(&app, &alice, "/api/groups", json!({ "name": "Web team" })).await;
    let other = created(&app, &alice, "/api/groups", json!({ "name": "Ops" })).await;
    for (sid, kind) in [
        (format!("group:{other}"), "group"),
        (format!("local:{ADA}"), "group"),
        (format!("local:{NOBODY}"), "user"),
        ("alice".to_owned(), "user"),
        (BOB_SID.to_owned(), "computer"),
    ] {
        let response = add_member(&app, &alice, &group, &sid, kind).await;
        assert_eq!(
            (response.status, response.json()["params"]["field"].as_str()),
            (StatusCode::BAD_REQUEST, Some("principal_sid")),
            "{sid} as {kind}"
        );
    }
    let nowhere = add_member(&app, &alice, NOBODY, BOB_SID, "user").await;
    assert_eq!(nowhere.status, StatusCode::NOT_FOUND);

    // Grants and purpose rules name only principals that exist, as they are.
    let folder = created(
        &app,
        &alice,
        "/api/folders",
        json!({ "parent_id": null, "name": "Sites" }),
    )
    .await;
    for (sid, kind) in [
        (format!("group:{NOBODY}"), "group"),
        (format!("group:{group}"), "user"),
        (format!("local:{NOBODY}"), "user"),
    ] {
        let grant = json!({
            "object": { "kind": "folder", "id": folder },
            "principal_kind": kind, "principal_sid": sid, "principal_name": "someone", "role": "list",
        });
        let response = call(&app, &alice, "POST", "/api/grants", Some(grant)).await;
        assert_eq!(
            (response.status, response.json()["params"]["field"].as_str()),
            (StatusCode::BAD_REQUEST, Some("principal_sid")),
            "grant to {sid} as {kind}"
        );
        let rule = json!({ "principal_kind": kind, "principal_name": "someone" });
        let uri = format!("/api/purpose-principals/{sid}");
        let response = call(&app, &alice, "PUT", &uri, Some(rule)).await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "purpose for {sid} as {kind}"
        );
    }
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_group_can_be_asked_for_a_purpose(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let alice = token(&app, sign_in_request("alice", "right")).await;
    let bob = token(&app, sign_in_request("bob", "right")).await;
    let group = created(
        &app,
        &alice,
        "/api/groups",
        json!({ "name": "Contractors" }),
    )
    .await;
    let rule = json!({ "principal_kind": "group", "principal_name": "Contractors" });
    let uri = format!("/api/purpose-principals/group:{group}");
    let response = call(&app, &alice, "PUT", &uri, Some(rule)).await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{}",
        response.json()
    );
    let purpose = |token: String| {
        let app = app.clone();
        async move {
            call(&app, &token, "GET", "/api/tree", None).await.json()["purpose_required"] == true
        }
    };
    assert!(!purpose(bob.clone()).await);
    add_member(&app, &alice, &group, BOB_SID, "user").await;
    assert!(purpose(bob).await);
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_search_finds_own_groups_and_local_accounts(pool: PgPool) {
    let (app, _) = setup(pool.clone()).await;
    let alice = token(&app, sign_in_request("alice", "right")).await;
    token(&app, sign_in("aal2")).await;
    let group = created(
        &app,
        &alice,
        "/api/groups",
        json!({ "name": "RH Web", "description": "Agency sites" }),
    )
    .await;
    let found = call(&app, &alice, "GET", "/api/directory/principals?q=rh", None).await;
    assert_eq!(
        found.json(),
        json!([
            { "kind": "group", "sid": format!("group:{group}"), "name": "RH Web", "detail": "Agency sites" },
            { "kind": "group", "sid": "S-1-5-21-1-2-3-1201", "name": "RH Admins", "detail": null },
            { "kind": "group", "sid": OPS_SID, "name": "RH Operators", "detail": null },
        ])
    );
    let found = call(
        &app,
        &alice,
        "GET",
        "/api/directory/principals?q=example",
        None,
    )
    .await;
    assert_eq!(
        found.json(),
        json!([{ "kind": "user", "sid": format!("local:{ADA}"), "name": "Ada", "detail": "ada@example.com" }])
    );
    // A pattern character matches only itself.
    let found = call(&app, &alice, "GET", "/api/directory/principals?q=%25", None).await;
    assert_eq!(found.json(), json!([]));

    // Without a directory, remotehub's own principals are all there is.
    let state = AppState::new(pool.clone(), None, settings(), vault());
    let issued = break_glass::create(&pool, &state.vault, "emergency")
        .await
        .unwrap();
    let plain = remotehub_server::app(state, None);
    let code = break_glass::code_at(&issued.totp_secret, now()).unwrap();
    let body =
        json!({ "username": "emergency", "password": issued.password.as_str(), "code": code });
    let emergency = token(
        &plain,
        json_request("POST", "/api/session/break-glass", body, Some(ORIGIN)),
    )
    .await;
    let found = send(
        &plain,
        get("/api/directory/principals?q=ada", Some(&emergency)),
    )
    .await;
    assert_eq!(found.status, StatusCode::OK, "{}", found.json());
    assert_eq!(found.json()[0]["sid"], format!("local:{ADA}"));
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
