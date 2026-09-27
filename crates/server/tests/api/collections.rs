//! Collections of shared credentials (#190): a tree of their own, apart from
//! the device folders. A grant on a collection holds for the credentials and
//! collections in it; a grant on a folder reaches no credential at all.

use axum::Router;
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{BOB_SID, OPS_SID, Response, authed, collection, send, sign_in_request};
use crate::terminal::{create, setup};

async fn call(app: &Router, token: &str, method: &str, uri: &str, body: Option<Value>) -> Response {
    send(app, authed(method, uri, body, token)).await
}

async fn sign_in(app: &Router, user: &str) -> String {
    send(app, sign_in_request(user, "right"))
        .await
        .session_token()
        .unwrap()
}

async fn grant(app: &Router, token: &str, kind: &str, id: &str, sid: &str, role: &str) {
    let principal_kind = if sid == OPS_SID { "group" } else { "user" };
    let response = call(
        app,
        token,
        "POST",
        "/api/grants",
        Some(json!({
            "object": { "kind": kind, "id": id },
            "principal_kind": principal_kind, "principal_sid": sid, "principal_name": "someone",
            "role": role,
        })),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{}",
        response.json()
    );
}

async fn tree(app: &Router, token: &str) -> Value {
    call(app, token, "GET", "/api/tree", None).await.json()
}

/// `(name, parent_id, role)` of each collection in the tree, by name.
fn collections(tree: &Value) -> Vec<(String, Value, Value)> {
    tree["collections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap().to_owned(),
                c["parent_id"].clone(),
                c["role"].clone(),
            )
        })
        .collect()
}

fn names(tree: &Value, list: &str) -> Vec<String> {
    tree[list]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap().to_owned())
        .collect()
}

async fn credential(app: &Router, token: &str, collection: &str, name: &str) -> String {
    create(
        app,
        token,
        "/api/credentials",
        json!({ "collection_id": collection, "name": name, "username": name, "password": "S3cret!" }),
    )
    .await
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn administrators_create_collections_at_the_top_managers_inside_theirs(pool: PgPool) {
    let (_, app, alice, _) = setup(pool).await;
    let bob = sign_in(&app, "bob").await;
    let new = |parent: Option<&str>, name: &str| {
        let body = json!({ "parent_id": parent, "name": name });
        let (app, bob) = (&app, &bob);
        async move { call(app, bob, "POST", "/api/collections", Some(body)).await }
    };

    let vault = collection(&app, &alice, None, "Vault").await;
    let other = collection(&app, &alice, None, "Other").await;
    assert_eq!(new(None, "Mine").await.code(), "forbidden");
    // A collection bob does not see does not exist for him.
    assert_eq!(new(Some(&vault), "Team").await.code(), "not_found");

    grant(&app, &alice, "collection", &vault, BOB_SID, "manage").await;
    grant(&app, &alice, "collection", &other, BOB_SID, "edit").await;
    let team = new(Some(&vault), "Team").await;
    assert_eq!(team.status, StatusCode::CREATED, "{}", team.json());
    // edit fills a collection with credentials, but does not arrange it.
    assert_eq!(new(Some(&other), "Team").await.code(), "forbidden");
    // Managing a collection is not the right to put something at the top.
    assert_eq!(new(None, "Mine").await.code(), "forbidden");
    let taken = new(Some(&vault), "Team").await;
    assert_eq!(
        (taken.status, taken.code().as_str()),
        (StatusCode::CONFLICT, "name_taken")
    );
    assert_eq!(new(Some(&vault), "  ").await.code(), "invalid_request");
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_collection_grant_reaches_what_is_in_it_and_a_folder_grant_no_credential(pool: PgPool) {
    let (_, app, alice, folder) = setup(pool).await;
    let vault = collection(&app, &alice, None, "Vault").await;
    let linux = collection(&app, &alice, Some(&vault), "Linux").await;
    let root = credential(&app, &alice, &linux, "root").await;
    create(
        &app,
        &alice,
        "/api/devices",
        json!({
            "folder_id": folder, "name": "web01", "protocol": "ssh", "host": "web01", "port": 22,
            "auth_mode": "stored", "credential_id": root,
        }),
    )
    .await;
    let bob = sign_in(&app, "bob").await;
    let reveal = format!("/api/credentials/{root}/reveal");
    let show = || Some(json!({ "purpose": "show" }));

    // manage on the folder of the device that uses it: still no credential.
    grant(&app, &alice, "folder", &folder, BOB_SID, "manage").await;
    let seen = tree(&app, &bob).await;
    assert_eq!(names(&seen, "devices"), ["web01"]);
    assert_eq!(names(&seen, "collections"), Vec::<String>::new());
    assert_eq!(names(&seen, "credentials"), Vec::<String>::new());
    assert_eq!(
        call(&app, &bob, "POST", &reveal, show()).await.code(),
        "not_found"
    );

    // reveal on the collection above holds for the one inside and its
    // credential.
    grant(&app, &alice, "collection", &vault, BOB_SID, "reveal").await;
    let seen = tree(&app, &bob).await;
    assert_eq!(
        collections(&seen),
        [
            ("Linux".into(), json!(vault), json!("reveal")),
            ("Vault".into(), Value::Null, json!("reveal")),
        ]
    );
    assert_eq!(names(&seen, "credentials"), ["root"]);
    assert_eq!(seen["credentials"][0]["role"], "reveal");
    let shown = call(&app, &bob, "POST", &reveal, show()).await;
    assert_eq!(shown.json()["password"], "S3cret!");
    // reveal is not edit: nothing new goes in.
    let added = call(
        &app,
        &bob,
        "POST",
        "/api/credentials",
        Some(json!({ "collection_id": linux, "name": "x", "password": "x" })),
    )
    .await;
    assert_eq!(added.code(), "forbidden");
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_tree_lists_only_the_collections_a_user_sees(pool: PgPool) {
    let (_, app, alice, _) = setup(pool).await;
    let vault = collection(&app, &alice, None, "Vault").await;
    let team = collection(&app, &alice, Some(&vault), "Team").await;
    let other = collection(&app, &alice, None, "Other").await;
    let db = credential(&app, &alice, &team, "db").await;
    credential(&app, &alice, &other, "elsewhere").await;
    let bob = sign_in(&app, "bob").await;
    assert!(collections(&tree(&app, &bob).await).is_empty());

    // A grant on the credential alone shows the collections on the way to
    // it, without a role.
    grant(&app, &alice, "credential", &db, BOB_SID, "list").await;
    let seen = tree(&app, &bob).await;
    assert_eq!(
        collections(&seen),
        [
            ("Team".into(), json!(vault), Value::Null),
            ("Vault".into(), Value::Null, Value::Null),
        ]
    );
    assert_eq!(names(&seen, "credentials"), ["db"]);
    assert_eq!(seen["credentials"][0]["collection_id"], team.as_str());
    assert!(seen["credentials"][0].get("folder_id").is_none());

    grant(&app, &alice, "collection", &team, BOB_SID, "connect").await;
    assert_eq!(
        collections(&tree(&app, &bob).await),
        [
            ("Team".into(), json!(vault), json!("connect")),
            ("Vault".into(), Value::Null, Value::Null),
        ]
    );
    // Administrators see all of them.
    assert_eq!(
        names(&tree(&app, &alice).await, "collections"),
        ["Other", "Team", "Vault"]
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_collection_does_not_move_into_itself(pool: PgPool) {
    let (_, app, alice, _) = setup(pool).await;
    let vault = collection(&app, &alice, None, "Vault").await;
    let team = collection(&app, &alice, Some(&vault), "Team").await;
    let deep = collection(&app, &alice, Some(&team), "Deep").await;
    let patch = |id: &str, body: Value| {
        let uri = format!("/api/collections/{id}");
        let (app, alice) = (&app, &alice);
        async move { call(app, alice, "PATCH", &uri, Some(body)).await }
    };

    for target in [&deep, &team, &vault] {
        let refused = patch(&vault, json!({ "parent_id": target })).await;
        assert_eq!(refused.code(), "invalid_request", "{target}");
        assert_eq!(refused.json()["params"]["field"], "parent_id", "{target}");
    }

    // Out of the way and renamed on the way: fine.
    let moved = patch(&deep, json!({ "parent_id": null, "name": "Shallow" })).await;
    assert_eq!(moved.status, StatusCode::NO_CONTENT, "{}", moved.json());
    let seen = collections(&tree(&app, &alice).await);
    assert!(seen.contains(&("Shallow".into(), Value::Null, json!("manage"))));
    assert!(seen.contains(&("Team".into(), json!(vault), json!("manage"))));
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn only_an_empty_collection_goes_and_every_change_is_audited(pool: PgPool) {
    let (_, app, alice, _) = setup(pool).await;
    let vault = collection(&app, &alice, None, "Vault").await;
    let team = collection(&app, &alice, Some(&vault), "Team").await;
    let db = credential(&app, &alice, &team, "db").await;
    let delete = |uri: String| {
        let (app, alice) = (&app, &alice);
        async move { call(app, alice, "DELETE", &uri, None).await }
    };

    // A collection inside or a credential keeps it.
    for id in [&vault, &team] {
        let kept = delete(format!("/api/collections/{id}")).await;
        assert_eq!(
            (kept.status, kept.code().as_str()),
            (StatusCode::CONFLICT, "folder_not_empty"),
            "{id}"
        );
    }
    let renamed = call(
        &app,
        &alice,
        "PATCH",
        &format!("/api/collections/{team}"),
        Some(json!({ "name": "Team B" })),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::NO_CONTENT);
    assert_eq!(
        delete(format!("/api/credentials/{db}")).await.status,
        StatusCode::NO_CONTENT
    );
    for id in [&team, &vault] {
        let gone = delete(format!("/api/collections/{id}")).await;
        assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.json());
    }
    assert!(collections(&tree(&app, &alice).await).is_empty());

    let log = call(&app, &alice, "GET", "/api/audit", None).await.json();
    let changes: Vec<(&str, &str, &str)> = log
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["object_type"] == "collection")
        .map(|e| {
            (
                e["action"].as_str().unwrap(),
                e["object_id"].as_str().unwrap(),
                e["actor_name"].as_str().unwrap(),
            )
        })
        .collect();
    // Newest first; the refused deletions left nothing.
    assert_eq!(
        changes,
        [
            ("collection.deleted", vault.as_str(), "alice"),
            ("collection.deleted", team.as_str(), "alice"),
            ("collection.updated", team.as_str(), "alice"),
            ("collection.created", team.as_str(), "alice"),
            ("collection.created", vault.as_str(), "alice"),
        ]
    );
}

/// `(principal_sid, role)` of each grant in one part of a grant list, each
/// checked to name a collection and nothing else.
fn on_collections(part: &Value) -> Vec<(String, String)> {
    part.as_array()
        .unwrap()
        .iter()
        .map(|g| {
            for column in ["folder_id", "device_id", "credential_id"] {
                assert!(g[column].is_null(), "{g}");
            }
            assert!(g["collection_id"].is_string(), "{g}");
            (
                g["principal_sid"].as_str().unwrap().to_owned(),
                g["role"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// The two trees stay apart in grant lists, too: the migration made
/// collections out of folders with the same ids, so a folder may well share
/// its id with a collection, and its grants must not show up there.
#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn collection_grants_list_what_they_inherit_and_never_a_folders(pool: PgPool) {
    let (_, app, alice, folder) = setup(pool.clone()).await;
    let vault = collection(&app, &alice, None, "Vault").await;
    let team = collection(&app, &alice, Some(&vault), "Team").await;
    let db = credential(&app, &alice, &team, "db").await;
    // A folder with the vault's id, as the migration leaves them.
    sqlx::query("INSERT INTO folders (id, name) VALUES ($1::uuid, 'Twin')")
        .bind(&vault)
        .execute(&pool)
        .await
        .unwrap();
    grant(&app, &alice, "collection", &vault, OPS_SID, "connect").await;
    grant(&app, &alice, "collection", &team, BOB_SID, "list").await;
    grant(&app, &alice, "folder", &vault, BOB_SID, "manage").await;
    grant(&app, &alice, "folder", &folder, OPS_SID, "edit").await;
    let grants = |kind: &str, id: &str| {
        let uri = format!("/api/grants?kind={kind}&id={id}");
        let (app, alice) = (&app, &alice);
        async move {
            let response = call(app, alice, "GET", &uri, None).await;
            assert_eq!(
                response.status,
                StatusCode::OK,
                "{uri}: {}",
                response.json()
            );
            response.json()
        }
    };

    let of_team = grants("collection", &team).await;
    assert_eq!(
        on_collections(&of_team["direct"]),
        [(BOB_SID.to_owned(), "list".to_owned())]
    );
    assert_eq!(
        on_collections(&of_team["inherited"]),
        [(OPS_SID.to_owned(), "connect".to_owned())]
    );
    let of_db = grants("credential", &db).await;
    assert_eq!(of_db["direct"], json!([]));
    let mut inherited = on_collections(&of_db["inherited"]);
    inherited.sort();
    assert_eq!(
        inherited,
        [
            (BOB_SID.to_owned(), "list".to_owned()),
            (OPS_SID.to_owned(), "connect".to_owned()),
        ]
    );

    // And the other way round: the folder lists its own grant only.
    let of_twin = grants("folder", &vault).await;
    assert_eq!(of_twin["inherited"], json!([]));
    let direct = of_twin["direct"].as_array().unwrap();
    assert_eq!(direct.len(), 1, "{of_twin}");
    assert_eq!(
        (&direct[0]["folder_id"], &direct[0]["principal_sid"]),
        (&json!(vault), &json!(BOB_SID))
    );

    // bob manages the folder, not the collection of the same id: that one
    // he sees only on the way to Team, and without a role it does not exist
    // for him.
    let bob = sign_in(&app, "bob").await;
    let seen = tree(&app, &bob).await;
    assert_eq!(names(&seen, "folders"), ["Twin"]);
    assert_eq!(
        collections(&seen),
        [
            ("Team".into(), json!(vault), json!("list")),
            ("Vault".into(), Value::Null, Value::Null),
        ]
    );
    let renamed = call(
        &app,
        &bob,
        "PATCH",
        &format!("/api/collections/{vault}"),
        Some(json!({ "name": "Mine" })),
    )
    .await;
    assert_eq!(renamed.code(), "not_found");
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn access_requests_name_no_collection(pool: PgPool) {
    let (_, app, alice, _) = setup(pool).await;
    let vault = collection(&app, &alice, None, "Vault").await;
    let bob = sign_in(&app, "bob").await;
    let asked = call(
        &app,
        &bob,
        "POST",
        "/api/access-requests",
        Some(json!({
            "object": { "kind": "collection", "id": vault },
            "role": "connect", "minutes": 60, "reason": "night shift",
        })),
    )
    .await;
    assert_eq!(asked.code(), "invalid_request");
    assert_eq!(asked.json()["params"]["field"], "kind");
}
