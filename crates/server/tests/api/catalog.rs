//! Folders, devices, credentials and grants over HTTP — with the permission
//! rules of authorize() applied end to end.

use crate::common::{BOB_SID, OPS_SID, authed, send, sign_in_request, state};
use axum::Router;
use axum::http::StatusCode;
use remotehub_server::app;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn sign_in(app: &Router, user: &str) -> String {
    send(app, sign_in_request(user, "right"))
        .await
        .session_token()
        .unwrap()
}

async fn call(
    app: &Router,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> crate::common::Response {
    send(app, authed(method, uri, body, token)).await
}

async fn create(app: &Router, token: &str, uri: &str, body: Value) -> String {
    let response = call(app, token, "POST", uri, Some(body)).await;
    assert_eq!(
        response.status,
        StatusCode::CREATED,
        "{uri}: {}",
        response.json()
    );
    response.json()["id"].as_str().unwrap().to_owned()
}

async fn tree(app: &Router, token: &str) -> Value {
    call(app, token, "GET", "/api/tree", None).await.json()
}

fn names(tree: &Value, list: &str) -> Vec<String> {
    tree[list]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap().to_owned())
        .collect()
}

async fn grant(
    app: &Router,
    token: &str,
    kind: &str,
    id: &str,
    sid: &str,
    principal_kind: &str,
    role: &str,
) {
    let response = call(
        app,
        token,
        "POST",
        "/api/grants",
        Some(json!({
            "object": { "kind": kind, "id": id },
            "principal_kind": principal_kind, "principal_sid": sid, "principal_name": "someone", "role": role,
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

/// Servers/Linux with web01 (stored root credential) and Servers/Windows with dc01.
struct Fixture {
    app: Router,
    alice: String,
    servers: String,
    linux: String,
    windows: String,
    web01: String,
    root_pw: String,
}

async fn fixture(pool: PgPool) -> Fixture {
    let app = app(state(pool), None);
    let alice = sign_in(&app, "alice").await;
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
    let windows = create(
        &app,
        &alice,
        "/api/folders",
        json!({ "parent_id": servers, "name": "Windows" }),
    )
    .await;
    let root_pw = create(
        &app,
        &alice,
        "/api/credentials",
        json!({ "folder_id": linux, "name": "root", "username": "root", "password": "T0p-Secret!" }),
    )
    .await;
    let web01 = create(
        &app,
        &alice,
        "/api/devices",
        json!({
            "folder_id": linux, "name": "web01", "protocol": "ssh", "host": "web01.example.com",
            "port": 22, "auth_mode": "stored", "credential_id": root_pw,
        }),
    )
    .await;
    create(
        &app,
        &alice,
        "/api/devices",
        json!({
            "folder_id": windows, "name": "dc01", "protocol": "rdp", "host": "dc01.example.com",
            "port": 3389, "auth_mode": "own", "credential_id": null,
        }),
    )
    .await;
    Fixture {
        app,
        alice,
        servers,
        linux,
        windows,
        web01,
        root_pw,
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn administrators_see_everything_others_only_what_is_granted(pool: PgPool) {
    let f = fixture(pool).await;
    let all = tree(&f.app, &f.alice).await;
    assert_eq!(names(&all, "folders"), ["Linux", "Servers", "Windows"]);
    assert_eq!(names(&all, "devices"), ["dc01", "web01"]);
    assert_eq!(names(&all, "credentials"), ["root"]);
    assert_eq!(all["may_create_top_level"], true);

    let bob = sign_in(&f.app, "bob").await;
    let nothing = tree(&f.app, &bob).await;
    assert_eq!(names(&nothing, "folders"), Vec::<String>::new());
    assert_eq!(nothing["may_create_top_level"], false);

    // A grant on one device shows it, plus the folders on the way (without a role).
    grant(
        &f.app, &f.alice, "device", &f.web01, BOB_SID, "user", "connect",
    )
    .await;
    let some = tree(&f.app, &bob).await;
    assert_eq!(names(&some, "devices"), ["web01"]);
    assert_eq!(some["devices"][0]["role"], "connect");
    assert_eq!(names(&some, "folders"), ["Linux", "Servers"]);
    assert!(
        some["folders"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["role"].is_null())
    );
    assert_eq!(names(&some, "credentials"), Vec::<String>::new());
}

/// Every folder starts closed; what a user opens stays open for that user
/// only, and only folders they see (#83).
#[sqlx::test(migrations = "../../migrations")]
async fn each_user_keeps_their_own_open_folders(pool: PgPool) {
    let f = fixture(pool).await;
    let open = |token: String| {
        let app = f.app.clone();
        async move {
            let tree = tree(&app, &token).await;
            let mut ids: Vec<String> = tree["open"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap().to_owned())
                .collect();
            ids.sort();
            ids
        }
    };
    let set = |token: String, folder: String, value: bool| {
        let app = f.app.clone();
        async move {
            call(
                &app,
                &token,
                "PUT",
                &format!("/api/folders/{folder}/open"),
                Some(json!({ "open": value })),
            )
            .await
            .status
        }
    };
    assert_eq!(open(f.alice.clone()).await, Vec::<String>::new());

    for folder in [&f.servers, &f.linux, &f.linux] {
        assert_eq!(
            set(f.alice.clone(), folder.clone(), true).await,
            StatusCode::NO_CONTENT
        );
    }
    let mut both = vec![f.servers.clone(), f.linux.clone()];
    both.sort();
    assert_eq!(open(f.alice.clone()).await, both);
    assert_eq!(
        set(f.alice.clone(), f.linux.clone(), false).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        open(f.alice.clone()).await,
        std::slice::from_ref(&f.servers)
    );

    // bob sees none of these folders: none of them opens for bob, and
    // alice's choice is not bob's.
    let bob = sign_in(&f.app, "bob").await;
    assert_eq!(
        set(bob.clone(), f.windows.clone(), true).await,
        StatusCode::NOT_FOUND
    );
    grant(
        &f.app, &f.alice, "device", &f.web01, BOB_SID, "user", "connect",
    )
    .await;
    assert_eq!(open(bob.clone()).await, Vec::<String>::new());
    // A folder bob sees only on the way to web01 opens.
    assert_eq!(
        set(bob.clone(), f.linux.clone(), true).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(open(bob).await, std::slice::from_ref(&f.linux));
}

#[sqlx::test(migrations = "../../migrations")]
async fn group_grants_hold_for_everything_below(pool: PgPool) {
    let f = fixture(pool).await;
    grant(
        &f.app, &f.alice, "folder", &f.servers, OPS_SID, "group", "connect",
    )
    .await;
    let olaf = sign_in(&f.app, "olaf").await;
    let seen = tree(&f.app, &olaf).await;
    assert_eq!(names(&seen, "devices"), ["dc01", "web01"]);
    assert_eq!(names(&seen, "credentials"), ["root"]);
    assert!(
        seen["devices"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["role"] == "connect")
    );

    // connect does not allow editing or creating.
    let edit = call(
        &f.app,
        &olaf,
        "DELETE",
        &format!("/api/devices/{}", f.web01),
        None,
    )
    .await;
    assert_eq!(edit.code(), "forbidden");
    let new = call(
        &f.app,
        &olaf,
        "POST",
        "/api/folders",
        Some(json!({ "parent_id": f.linux, "name": "x" })),
    )
    .await;
    assert_eq!(new.code(), "forbidden");
}

#[sqlx::test(migrations = "../../migrations")]
async fn invisible_objects_do_not_exist_for_the_caller(pool: PgPool) {
    let f = fixture(pool).await;
    let bob = sign_in(&f.app, "bob").await;
    for (method, uri) in [
        ("DELETE", format!("/api/devices/{}", f.web01)),
        ("DELETE", format!("/api/credentials/{}", f.root_pw)),
        ("DELETE", format!("/api/folders/{}", f.linux)),
        ("GET", format!("/api/grants?kind=folder&id={}", f.linux)),
    ] {
        let response = call(&f.app, &bob, method, &uri, None).await;
        assert_eq!(
            (response.status, response.code().as_str()),
            (StatusCode::NOT_FOUND, "not_found"),
            "{uri}"
        );
    }
    let top = call(
        &f.app,
        &bob,
        "POST",
        "/api/folders",
        Some(json!({ "parent_id": null, "name": "Mine" })),
    )
    .await;
    assert_eq!(top.code(), "forbidden");
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_credential_only_goes_where_its_users_may_send_it(pool: PgPool) {
    let f = fixture(pool).await;
    let bob = sign_in(&f.app, "bob").await;
    grant(
        &f.app, &f.alice, "folder", &f.linux, BOB_SID, "user", "edit",
    )
    .await;
    grant(
        &f.app, &f.alice, "folder", &f.windows, BOB_SID, "user", "edit",
    )
    .await;
    let device = |host: &str, folder: &str, credential: &str| {
        json!({
            "folder_id": folder, "name": "web01", "protocol": "ssh", "host": host, "port": 22,
            "auth_mode": "stored", "credential_id": credential,
        })
    };

    // With edit on Linux, bob may use the root credential there …
    let ok = call(
        &f.app,
        &bob,
        "PUT",
        &format!("/api/devices/{}", f.web01),
        Some(device("web01.example.com", &f.linux, &f.root_pw)),
    )
    .await;
    assert_eq!(ok.status, StatusCode::NO_CONTENT);

    // … but a credential he may only list must not be pointed at a new host.
    let other = create(
        &f.app,
        &f.alice,
        "/api/credentials",
        json!({ "folder_id": f.servers, "name": "domain admin", "password": "Adm1n!" }),
    )
    .await;
    grant(
        &f.app,
        &f.alice,
        "credential",
        &other,
        BOB_SID,
        "user",
        "list",
    )
    .await;
    let linked = call(
        &f.app,
        &bob,
        "POST",
        "/api/devices",
        Some(json!({
            "folder_id": f.windows, "name": "evil", "protocol": "ssh", "host": "attacker.example",
            "port": 22, "auth_mode": "stored", "credential_id": other,
        })),
    )
    .await;
    assert_eq!(linked.code(), "forbidden");

    // Moving a device with a stored credential to another host needs connect on it.
    let admin_device = create(
        &f.app,
        &f.alice,
        "/api/devices",
        json!({
            "folder_id": f.windows, "name": "dc02", "protocol": "ssh", "host": "dc02.example.com",
            "port": 22, "auth_mode": "stored", "credential_id": other,
        }),
    )
    .await;
    let redirected = call(
        &f.app,
        &bob,
        "PUT",
        &format!("/api/devices/{admin_device}"),
        Some(json!({
            "folder_id": f.windows, "name": "dc02", "protocol": "ssh", "host": "attacker.example",
            "port": 22, "auth_mode": "stored", "credential_id": other,
        })),
    )
    .await;
    assert_eq!(redirected.code(), "forbidden");
    // Renaming it without touching the target is fine.
    let renamed = call(
        &f.app,
        &bob,
        "PUT",
        &format!("/api/devices/{admin_device}"),
        Some(json!({
            "folder_id": f.windows, "name": "dc02 (old)", "protocol": "ssh", "host": "dc02.example.com",
            "port": 22, "auth_mode": "stored", "credential_id": other,
        })),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrations = "../../migrations")]
async fn passwords_are_sealed_versioned_and_never_returned(pool: PgPool) {
    let f = fixture(pool.clone()).await;
    let everything = tree(&f.app, &f.alice).await.to_string();
    assert!(!everything.contains("T0p-Secret!"));

    let change = |password: Option<&str>| {
        let mut body =
            json!({ "folder_id": f.linux, "name": "root", "username": "root", "domain": "" });
        if let Some(p) = password {
            body["password"] = json!(p);
        }
        body
    };
    let uri = format!("/api/credentials/{}", f.root_pw);
    assert_eq!(
        call(&f.app, &f.alice, "PUT", &uri, Some(change(None)))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(tree(&f.app, &f.alice).await["credentials"][0]["version"], 1);
    assert_eq!(
        call(&f.app, &f.alice, "PUT", &uri, Some(change(Some("N3w!"))))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(tree(&f.app, &f.alice).await["credentials"][0]["version"], 2);

    let sealed: Vec<(i32, Vec<u8>)> = sqlx::query_as(
        "SELECT version, ciphertext FROM secret_fields WHERE field = 'password' ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(sealed.iter().map(|(v, _)| *v).collect::<Vec<_>>(), [1, 2]);
    assert!(
        sealed
            .iter()
            .all(|(_, c)| !c.windows(4).any(|w| w == b"N3w!"))
    );

    let log = call(&f.app, &f.alice, "GET", "/api/audit", None)
        .await
        .json()
        .to_string();
    assert!(log.contains("credential.updated") && log.contains("\"secret_changed\":true"));
    assert!(!log.contains("T0p-Secret!") && !log.contains("N3w!"));

    // Deleting the credential leaves its devices asking for credentials.
    assert_eq!(
        call(&f.app, &f.alice, "DELETE", &uri, None).await.status,
        StatusCode::NO_CONTENT
    );
    let web01 = tree(&f.app, &f.alice).await["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "web01")
        .cloned()
        .unwrap();
    assert_eq!(
        (
            web01["auth_mode"].as_str(),
            web01["credential_id"].is_null()
        ),
        (Some("ask"), true)
    );
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM secret_fields")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn folders_keep_their_tree_intact(pool: PgPool) {
    let f = fixture(pool).await;
    let taken = call(
        &f.app,
        &f.alice,
        "POST",
        "/api/folders",
        Some(json!({ "parent_id": f.servers, "name": "Linux" })),
    )
    .await;
    assert_eq!(
        (taken.status, taken.code().as_str()),
        (StatusCode::CONFLICT, "name_taken")
    );

    let not_empty = call(
        &f.app,
        &f.alice,
        "DELETE",
        &format!("/api/folders/{}", f.linux),
        None,
    )
    .await;
    assert_eq!(
        (not_empty.status, not_empty.code().as_str()),
        (StatusCode::CONFLICT, "folder_not_empty")
    );

    let into_itself = call(
        &f.app,
        &f.alice,
        "PATCH",
        &format!("/api/folders/{}", f.servers),
        Some(json!({ "parent_id": f.linux })),
    )
    .await;
    assert_eq!(into_itself.code(), "invalid_request");

    let renamed = call(
        &f.app,
        &f.alice,
        "PATCH",
        &format!("/api/folders/{}", f.windows),
        Some(json!({ "name": "Win" })),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::NO_CONTENT);
    let to_top = call(
        &f.app,
        &f.alice,
        "PATCH",
        &format!("/api/folders/{}", f.windows),
        Some(json!({ "parent_id": null })),
    )
    .await;
    assert_eq!(to_top.status, StatusCode::NO_CONTENT);
    let moved = tree(&f.app, &f.alice).await["folders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "Win")
        .cloned()
        .unwrap();
    assert!(moved["parent_id"].is_null());
}

#[sqlx::test(migrations = "../../migrations")]
async fn grants_are_listed_with_what_they_inherit_and_can_be_removed(pool: PgPool) {
    let f = fixture(pool).await;
    grant(
        &f.app, &f.alice, "folder", &f.servers, OPS_SID, "group", "connect",
    )
    .await;
    grant(
        &f.app, &f.alice, "device", &f.web01, BOB_SID, "user", "list",
    )
    .await;
    // Granting again changes the role instead of adding a second grant.
    grant(
        &f.app, &f.alice, "device", &f.web01, BOB_SID, "user", "edit",
    )
    .await;

    let grants = call(
        &f.app,
        &f.alice,
        "GET",
        &format!("/api/grants?kind=device&id={}", f.web01),
        None,
    )
    .await
    .json();
    assert_eq!(grants["direct"].as_array().unwrap().len(), 1);
    assert_eq!(grants["direct"][0]["role"], "edit");
    assert_eq!(grants["inherited"][0]["principal_sid"], OPS_SID);

    let id = grants["direct"][0]["id"].as_str().unwrap();
    let bob = sign_in(&f.app, "bob").await;
    assert_eq!(names(&tree(&f.app, &bob).await, "devices"), ["web01"]);
    assert_eq!(
        call(
            &f.app,
            &f.alice,
            "DELETE",
            &format!("/api/grants/{id}"),
            None
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        names(&tree(&f.app, &bob).await, "devices"),
        Vec::<String>::new()
    );

    let invalid = call(
        &f.app,
        &f.alice,
        "POST",
        "/api/grants",
        Some(json!({
            "object": { "kind": "device", "id": f.web01 },
            "principal_kind": "user", "principal_sid": "not-a-sid", "principal_name": "x", "role": "edit",
        })),
    )
    .await;
    assert_eq!(invalid.code(), "invalid_request");
}

#[sqlx::test(migrations = "../../migrations")]
async fn only_people_who_manage_something_search_the_directory(pool: PgPool) {
    let f = fixture(pool).await;
    let bob = sign_in(&f.app, "bob").await;
    assert_eq!(
        call(&f.app, &bob, "GET", "/api/directory/principals?q=rh", None)
            .await
            .code(),
        "forbidden"
    );

    let found = call(
        &f.app,
        &f.alice,
        "GET",
        "/api/directory/principals?q=rh",
        None,
    )
    .await
    .json();
    let kinds: Vec<(&str, &str)> = found
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["kind"].as_str().unwrap(), p["name"].as_str().unwrap()))
        .collect();
    assert_eq!(kinds, [("group", "RH Admins"), ("group", "RH Operators")]);
}

#[sqlx::test(migrations = "../../migrations")]
async fn devices_are_validated(pool: PgPool) {
    let f = fixture(pool).await;
    let base = json!({
        "folder_id": f.linux, "name": "x", "protocol": "ssh", "host": "x.example.com",
        "port": 22, "auth_mode": "ask", "credential_id": null,
    });
    for (field, value) in [
        ("protocol", json!("telnet")),
        ("host", json!("bad host")),
        ("host", json!("")),
        ("port", json!(0)),
        ("auth_mode", json!("stored")),
        ("name", json!("  ")),
    ] {
        let mut body = base.clone();
        body[field] = value;
        let response = call(&f.app, &f.alice, "POST", "/api/devices", Some(body)).await;
        assert_eq!(response.code(), "invalid_request", "{field}");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn rdp_devices_keep_a_keyboard_layout_of_guacd(pool: PgPool) {
    let f = fixture(pool).await;
    let device = |protocol: &str, layout: Value| {
        json!({
            "folder_id": f.linux, "name": format!("{protocol} {layout}"), "protocol": protocol,
            "host": "x.example.com", "port": 3389, "auth_mode": "ask", "credential_id": null,
            "keyboard_layout": layout,
        })
    };
    let german = create(
        &f.app,
        &f.alice,
        "/api/devices",
        device("rdp", json!("de-de-qwertz")),
    )
    .await;
    let default = create(&f.app, &f.alice, "/api/devices", device("rdp", Value::Null)).await;
    // VNC and SSH send characters: a layout would mean nothing.
    let vnc = create(
        &f.app,
        &f.alice,
        "/api/devices",
        device("vnc", json!("de-de-qwertz")),
    )
    .await;
    let response = call(
        &f.app,
        &f.alice,
        "POST",
        "/api/devices",
        Some(device("rdp", json!("klingon"))),
    )
    .await;
    assert_eq!(response.code(), "invalid_request");
    assert_eq!(response.json()["params"]["field"], "keyboard_layout");

    let tree = tree(&f.app, &f.alice).await;
    let layout = |id: &str| {
        tree["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == id)
            .unwrap()["keyboard_layout"]
            .clone()
    };
    assert_eq!(layout(&german), "de-de-qwertz");
    assert_eq!(layout(&default), Value::Null);
    assert_eq!(layout(&vnc), Value::Null);
}

/// A file of the lab's SSH target (deploy/testlab/ssh).
fn lab_key(file: &str) -> String {
    let dir = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo and nextest"),
    );
    std::fs::read_to_string(dir.join("../../deploy/testlab/ssh").join(file)).unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn ssh_keys_are_checked_sealed_and_shown_only_by_fingerprint(pool: PgPool) {
    let f = fixture(pool.clone()).await;
    let key = lab_key("tester_ed25519_cert");
    let certificate = lab_key("tester_ed25519_cert-cert.pub");
    let body = |extra: Value| {
        let mut body =
            json!({ "folder_id": f.linux, "name": "key", "kind": "ssh_key", "username": "tester" });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        body
    };

    for (extra, field) in [
        (json!({}), "private_key"),
        (json!({ "private_key": "not a key" }), "private_key"),
        (
            json!({ "private_key": lab_key("tester_ed25519_passphrase") }),
            "passphrase",
        ),
        (
            json!({ "private_key": lab_key("tester_ed25519_passphrase"), "passphrase": "wrong" }),
            "passphrase",
        ),
        (
            json!({ "private_key": lab_key("tester_ed25519"), "certificate": certificate }),
            "certificate",
        ),
    ] {
        let response = call(
            &f.app,
            &f.alice,
            "POST",
            "/api/credentials",
            Some(body(extra)),
        )
        .await;
        assert_eq!(response.code(), "invalid_request");
        assert_eq!(response.json()["params"]["field"], field);
    }

    let id = create(
        &f.app,
        &f.alice,
        "/api/credentials",
        body(json!({ "private_key": key, "certificate": certificate })),
    )
    .await;
    let everything = tree(&f.app, &f.alice).await;
    let shown = everything["credentials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id.as_str())
        .unwrap()
        .clone();
    assert_eq!(shown["kind"], "ssh_key");
    assert_eq!(shown["key_algorithm"], "ssh-ed25519");
    assert!(
        shown["key_fingerprint"]
            .as_str()
            .unwrap()
            .starts_with("SHA256:")
    );
    assert_eq!(shown["has_certificate"], true);
    let secret_line = key.lines().nth(1).unwrap();
    assert!(!everything.to_string().contains(secret_line));
    let log = call(&f.app, &f.alice, "GET", "/api/audit", None)
        .await
        .json()
        .to_string();
    assert!(!log.contains(secret_line));

    let fields: Vec<String> = sqlx::query_scalar(
        "SELECT field FROM secret_fields WHERE owner_id = $1::uuid ORDER BY field",
    )
    .bind(&id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(fields, ["certificate", "private_key"]);

    // The kind stays; a new key makes a new version.
    let change = call(
        &f.app,
        &f.alice,
        "PUT",
        &format!("/api/credentials/{id}"),
        Some(body(json!({ "kind": "password" }))),
    )
    .await;
    assert_eq!(change.json()["params"]["field"], "kind");
    let renew = call(
        &f.app,
        &f.alice,
        "PUT",
        &format!("/api/credentials/{id}"),
        Some(body(json!({ "private_key": lab_key("tester_ed25519") }))),
    )
    .await;
    assert_eq!(renew.status, StatusCode::NO_CONTENT);
    let renewed = tree(&f.app, &f.alice).await["credentials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id.as_str())
        .unwrap()
        .clone();
    assert_eq!(
        (
            renewed["version"].as_i64(),
            renewed["has_certificate"].as_bool()
        ),
        (Some(2), Some(false))
    );
}

/// The sealed versions of a device's own password, oldest first.
async fn device_secrets(pool: &PgPool, device: &str) -> Vec<i32> {
    sqlx::query_scalar(
        "SELECT version FROM secret_fields WHERE owner_id = $1::uuid AND field = 'password'
         ORDER BY version",
    )
    .bind(device)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_device_keeps_credentials_of_its_own(pool: PgPool) {
    let f = fixture(pool.clone()).await;
    let device = |host: &str, password: Option<&str>| {
        let mut body = json!({
            "folder_id": f.linux, "name": "db01", "protocol": "ssh", "host": host, "port": 22,
            "auth_mode": "device", "credential_id": null, "username": " admin ", "domain": "LAB",
        });
        if let Some(p) = password {
            body["password"] = json!(p);
        }
        body
    };
    let refused = call(
        &f.app,
        &f.alice,
        "POST",
        "/api/devices",
        Some(device("db01", None)),
    )
    .await;
    assert_eq!(refused.json()["params"]["field"], "password");
    let db01 = create(
        &f.app,
        &f.alice,
        "/api/devices",
        device("db01", Some("Own-S3cret!")),
    )
    .await;

    let shown = tree(&f.app, &f.alice).await;
    let row = shown["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == db01.as_str())
        .unwrap();
    assert_eq!(
        (row["username"].as_str(), row["domain"].as_str()),
        (Some("admin"), Some("LAB"))
    );
    assert!(!shown.to_string().contains("Own-S3cret!"));
    assert_eq!(device_secrets(&pool, &db01).await, [1]);

    // The same target keeps its password; another target needs it again.
    let uri = format!("/api/devices/{db01}");
    let put = |body: Value| call(&f.app, &f.alice, "PUT", &uri, Some(body));
    assert_eq!(
        put(device("db01", None)).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(device_secrets(&pool, &db01).await, [1]);
    assert_eq!(
        put(device("evil.example", None)).await.json()["params"]["field"],
        "password"
    );
    assert_eq!(
        put(device("db01.lab", Some("N3w-S3cret!"))).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(device_secrets(&pool, &db01).await, [2]);
    let sealed: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT ciphertext FROM secret_fields WHERE owner_id = $1::uuid")
            .bind(&db01)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(sealed.iter().all(|c| !c.windows(4).any(|w| w == b"N3w-")));

    // Another way to sign in drops them; coming back needs the password.
    let mut asking = device("db01.lab", None);
    asking["auth_mode"] = json!("ask");
    assert_eq!(put(asking).await.status, StatusCode::NO_CONTENT);
    assert!(device_secrets(&pool, &db01).await.is_empty());
    assert_eq!(
        put(device("db01.lab", None)).await.json()["params"]["field"],
        "password"
    );
    assert_eq!(
        put(device("db01.lab", Some("Th1rd!"))).await.status,
        StatusCode::NO_CONTENT
    );

    let log = call(&f.app, &f.alice, "GET", "/api/audit", None)
        .await
        .json()
        .to_string();
    assert!(
        !log.contains("Own-S3cret!") && !log.contains("N3w-S3cret!") && !log.contains("Th1rd!")
    );

    // Deleting the device deletes its password.
    assert_eq!(
        call(&f.app, &f.alice, "DELETE", &uri, None).await.status,
        StatusCode::NO_CONTENT
    );
    assert!(device_secrets(&pool, &db01).await.is_empty());
}
