//! Permission reports (#110): the folders Servers/Linux and the collection
//! Passwords with the root credential; the operators (olaf's group) may
//! connect to Servers and Passwords, bob may reveal the root credential on
//! its own. alice administers.

use axum::Router;
use axum::http::StatusCode;
use remotehub_server::app;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{BOB_SID, OPS_SID, Response, authed, collection, send, sign_in_request, state};

async fn token(app: &Router, user: &str) -> String {
    send(app, sign_in_request(user, "right"))
        .await
        .session_token()
        .unwrap()
}

async fn call(app: &Router, token: &str, method: &str, uri: &str, body: Option<Value>) -> Response {
    send(app, authed(method, uri, body, token)).await
}

async fn create(app: &Router, token: &str, uri: &str, body: Value) -> String {
    let response = call(app, token, "POST", uri, Some(body)).await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json());
    response.json()["id"].as_str().unwrap().to_owned()
}

async fn grant(
    app: &Router,
    token: &str,
    object: Value,
    sid: &str,
    kind: &str,
    name: &str,
    role: &str,
) {
    let body = json!({
        "object": object, "principal_kind": kind, "principal_sid": sid,
        "principal_name": name, "role": role,
    });
    let response = call(app, token, "POST", "/api/grants", Some(body)).await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{}",
        response.json()
    );
}

/// A person's id as the report lists them.
async fn person(app: &Router, token: &str, username: &str) -> String {
    let people = call(app, token, "GET", "/api/reports/people", None)
        .await
        .json();
    people
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["username"] == username)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// `(name, role, [(principal name, where, inherited)])` per object reached.
/// One object reached: its name, the role, and the reasons.
type Reached = (String, String, Vec<(String, String, bool)>);

fn reached(report: &Value) -> Vec<Reached> {
    report["reach"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let reasons = r["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .map(|why| {
                    (
                        why["principal_name"].as_str().unwrap().to_owned(),
                        why["on_name"].as_str().unwrap().to_owned(),
                        why["inherited"].as_bool().unwrap(),
                    )
                })
                .collect();
            (
                r["name"].as_str().unwrap().to_owned(),
                r["role"].as_str().unwrap().to_owned(),
                reasons,
            )
        })
        .collect()
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_report_names_what_someone_reaches_and_why(pool: PgPool) {
    let app = app(state(pool), None);
    let alice = token(&app, "alice").await;
    let bob = token(&app, "bob").await;
    token(&app, "olaf").await;
    let servers = create(
        &app,
        &alice,
        "/api/folders",
        json!({ "parent_id": null, "name": "Servers" }),
    )
    .await;
    let linux = create(
        &app,
        &alice,
        "/api/folders",
        json!({ "parent_id": servers, "name": "Linux" }),
    )
    .await;
    let passwords = collection(&app, &alice, None, "Passwords").await;
    let root = create(
        &app,
        &alice,
        "/api/credentials",
        json!({ "collection_id": passwords, "name": "root", "username": "root", "password": "T0p-Secret!" }),
    )
    .await;
    for object in [
        json!({ "kind": "folder", "id": servers }),
        json!({ "kind": "collection", "id": passwords }),
    ] {
        grant(
            &app,
            &alice,
            object,
            OPS_SID,
            "group",
            "RH Operators",
            "connect",
        )
        .await;
    }
    // Granted under a name he no longer has: the report says today's.
    let credential = json!({ "kind": "credential", "id": root });
    grant(
        &app,
        &alice,
        credential,
        BOB_SID,
        "user",
        "Robert (old name)",
        "reveal",
    )
    .await;

    // Auditors and administrators only.
    for uri in ["/api/reports/people", "/api/reports/folders"] {
        assert_eq!(
            call(&app, &bob, "GET", uri, None).await.status,
            StatusCode::FORBIDDEN,
            "{uri}"
        );
    }

    // olaf, through his directory group: Servers and Passwords and
    // everything below them, sorted by path.
    let olaf = person(&app, &alice, "olaf").await;
    let report = call(
        &app,
        &alice,
        "GET",
        &format!("/api/reports/users/{olaf}"),
        None,
    )
    .await
    .json();
    assert_eq!(report["groups_from"], "directory");
    assert_eq!(report["administrator"], false);
    let ops =
        |on: &str, inherited: bool| vec![("RH Operators".to_owned(), on.to_owned(), inherited)];
    assert_eq!(
        reached(&report),
        [
            (
                "Passwords".into(),
                "connect".into(),
                ops("Passwords", false)
            ),
            ("Servers".into(), "connect".into(), ops("Servers", false)),
            ("root".into(), "connect".into(), ops("Passwords", true)),
            ("Linux".into(), "connect".into(), ops("Servers", true)),
        ]
    );

    // bob: the credential alone, by his own grant; the way to it is no reach.
    let bob_id = person(&app, &alice, "bob").await;
    let report = call(
        &app,
        &alice,
        "GET",
        &format!("/api/reports/users/{bob_id}"),
        None,
    )
    .await
    .json();
    assert_eq!(
        reached(&report),
        [(
            "root".into(),
            "reveal".into(),
            vec![("Bob Helpdesk".into(), "root".into(), false)]
        )]
    );
    assert_eq!(report["reach"][0]["path"], json!(["Passwords"]));

    // alice reaches everything as an administrator, without grants.
    let alice_id = person(&app, &alice, "alice").await;
    let report = call(
        &app,
        &alice,
        "GET",
        &format!("/api/reports/users/{alice_id}"),
        None,
    )
    .await
    .json();
    assert_eq!(report["administrator"], true);
    assert_eq!(report["reach"].as_array().unwrap().len(), 4);
    assert!(
        report["reach"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["role"] == "manage")
    );

    // Who reaches Linux: the operators, from Servers; the grant on Passwords
    // and bob's grant lie in the other tree.
    let report = call(
        &app,
        &alice,
        "GET",
        &format!("/api/reports/folders/{linux}"),
        None,
    )
    .await
    .json();
    assert_eq!(report["folder"]["path"], json!(["Servers", "Linux"]));
    let holders: Vec<(&str, &str, bool)> = report["holders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| {
            (
                h["principal_name"].as_str().unwrap(),
                h["role"].as_str().unwrap(),
                h["inherited"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(holders, [("RH Operators", "connect", true)]);
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_group_of_remotehubs_own_counts_and_shows_its_members(pool: PgPool) {
    let app = app(state(pool), None);
    let alice = token(&app, "alice").await;
    token(&app, "bob").await;
    let team = create(
        &app,
        &alice,
        "/api/groups",
        json!({ "name": "Night shift" }),
    )
    .await;
    let member = json!({ "principal_kind": "user", "principal_name": "Bob Helpdesk" });
    let added = call(
        &app,
        &alice,
        "PUT",
        &format!("/api/groups/{team}/members/{BOB_SID}"),
        Some(member),
    )
    .await;
    assert_eq!(added.status, StatusCode::NO_CONTENT, "{}", added.json());
    let folder = create(
        &app,
        &alice,
        "/api/folders",
        json!({ "parent_id": null, "name": "Backups" }),
    )
    .await;
    let object = json!({ "kind": "folder", "id": folder });
    grant(
        &app,
        &alice,
        object,
        &format!("group:{team}"),
        "group",
        "Night shift",
        "list",
    )
    .await;

    let bob = person(&app, &alice, "bob").await;
    let report = call(
        &app,
        &alice,
        "GET",
        &format!("/api/reports/users/{bob}"),
        None,
    )
    .await
    .json();
    assert_eq!(
        reached(&report),
        [(
            "Backups".into(),
            "list".into(),
            vec![("Night shift".into(), "Backups".into(), false)]
        )]
    );
    let report = call(
        &app,
        &alice,
        "GET",
        &format!("/api/reports/folders/{folder}"),
        None,
    )
    .await
    .json();
    assert_eq!(report["holders"][0]["members"], json!(["Bob Helpdesk"]));
}
