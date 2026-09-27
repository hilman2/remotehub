//! Login profiles (#192): a login many devices share, a password or an SSH
//! key. A profile lies in a device folder and holds that folder's grants,
//! none of its own and never a collection's; one at the top level is for
//! administrators only. A device signs in only with a profile in its folder
//! or one above, and whoever links a profile or sends it to another target
//! must be allowed to use it.

use axum::Router;
use axum::http::StatusCode;
use remotehub_server::app;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{
    BOB_SID, OPS_SID, Response, authed, lab_key, profile, send, sign_in_request, state,
};

async fn call(app: &Router, token: &str, method: &str, uri: &str, body: Option<Value>) -> Response {
    send(app, authed(method, uri, body, token)).await
}

async fn sign_in(app: &Router, user: &str) -> String {
    send(app, sign_in_request(user, "right"))
        .await
        .session_token()
        .unwrap()
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

/// `(name, role)` of each profile in the tree, in its order (by name).
fn profiles(tree: &Value) -> Vec<(&str, &str)> {
    tree["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["name"].as_str().unwrap(), p["role"].as_str().unwrap()))
        .collect()
}

/// The tree's entry for the profile `id`.
fn entry(tree: &Value, id: &str) -> Value {
    tree["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .unwrap_or_else(|| panic!("no profile {id} in {tree}"))
        .clone()
}

/// The sealed fields of `owner` as `(version, field)`.
async fn sealed(pool: &PgPool, owner: &str) -> Vec<(i32, String)> {
    sqlx::query_as(
        "SELECT version, field FROM secret_fields WHERE owner_id = $1::uuid ORDER BY version, field",
    )
    .bind(owner)
    .fetch_all(pool)
    .await
    .unwrap()
}

const PASSWORD: &str = "Adm1n-Passw0rd!";

/// A password login as `profile` takes it.
fn login(username: &str) -> Value {
    json!({ "username": username, "password": PASSWORD })
}

/// An SSH device in `folder` that signs in with the login profile `profile`.
fn device(folder: &str, name: &str, profile: &str) -> Value {
    json!({
        "folder_id": folder, "name": name, "protocol": "ssh", "host": "host.example.com",
        "port": 22, "auth_mode": "profile", "profile_id": profile,
    })
}

/// The folders Customers/A, Customers/A/Servers and Customers/B; alice
/// administers.
struct Fixture {
    app: Router,
    alice: String,
    customers: String,
    a: String,
    servers: String,
    b: String,
}

async fn fixture(pool: PgPool) -> Fixture {
    let app = app(state(pool), None);
    let alice = sign_in(&app, "alice").await;
    let folder = |parent: Option<String>, name: &'static str| {
        let (app, alice) = (&app, &alice);
        async move {
            let body = json!({ "parent_id": parent, "name": name });
            create(app, alice, "/api/folders", body).await
        }
    };
    let customers = folder(None, "Customers").await;
    let a = folder(Some(customers.clone()), "A").await;
    let servers = folder(Some(a.clone()), "Servers").await;
    let b = folder(Some(customers.clone()), "B").await;
    Fixture {
        app,
        alice,
        customers,
        a,
        servers,
        b,
    }
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_profile_lies_where_one_may_edit_and_at_the_top_only_for_administrators(pool: PgPool) {
    let f = fixture(pool).await;
    let bob = sign_in(&f.app, "bob").await;
    let body = |folder: Option<&str>, name: &str| {
        let mut body = login("admin");
        body["folder_id"] = json!(folder);
        body["name"] = json!(name);
        body
    };
    let new = |folder: Option<&str>, name: &str| {
        let body = body(folder, name);
        let (app, bob) = (&f.app, &bob);
        async move { call(app, bob, "POST", "/api/profiles", Some(body)).await }
    };

    // A folder bob does not see does not exist for him; connect is not
    // enough to add to it.
    assert_eq!(new(Some(&f.a), "admin").await.code(), "not_found");
    grant(&f.app, &f.alice, "folder", &f.a, BOB_SID, "connect").await;
    assert_eq!(new(Some(&f.a), "admin").await.code(), "forbidden");
    grant(&f.app, &f.alice, "folder", &f.a, BOB_SID, "edit").await;
    let made = new(Some(&f.a), "admin").await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.json());
    let id = made.json()["id"].as_str().unwrap().to_owned();
    // edit on A holds below it.
    let below = new(Some(&f.servers), "admin").await;
    assert_eq!(below.status, StatusCode::CREATED, "{}", below.json());

    // The top level is for administrators, to create in or to move to; a
    // move needs edit on the target folder, too.
    assert_eq!(new(None, "admin").await.code(), "forbidden");
    let uri = format!("/api/profiles/{id}");
    let to_top = call(&f.app, &bob, "PUT", &uri, Some(body(None, "admin"))).await;
    assert_eq!(to_top.code(), "forbidden");
    let to_b = call(&f.app, &bob, "PUT", &uri, Some(body(Some(&f.b), "admin"))).await;
    assert_eq!(to_b.code(), "not_found");
    profile(
        &f.app,
        &f.alice,
        None,
        "domain admin",
        login("administrator"),
    )
    .await;

    // One name per folder, the top level included.
    let taken = new(Some(&f.a), "admin").await;
    assert_eq!(
        (taken.status, taken.code().as_str()),
        (StatusCode::CONFLICT, "name_taken")
    );
    let taken = call(
        &f.app,
        &f.alice,
        "POST",
        "/api/profiles",
        Some(body(None, "domain admin")),
    )
    .await;
    assert_eq!(taken.code(), "name_taken");

    for (extra, field) in [
        (json!({ "name": "  " }), "name"),
        (json!({ "username": "ad\u{7}min" }), "username"),
        (json!({ "password": "" }), "password"),
        (json!({ "secret_kind": "telnet" }), "secret_kind"),
        (json!({ "secret_kind": "ssh_key" }), "private_key"),
    ] {
        let mut body = body(Some(&f.a), "other");
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let refused = call(&f.app, &f.alice, "POST", "/api/profiles", Some(body)).await;
        assert_eq!(
            (refused.code().as_str(), &refused.json()["params"]["field"]),
            ("invalid_request", &json!(field)),
            "{extra}"
        );
    }

    // Every profile made is audited, and the password shows nowhere.
    let log = call(&f.app, &f.alice, "GET", "/api/audit", None)
        .await
        .json();
    let created: Vec<(&str, &str, &str)> = log
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == "profile.created")
        .map(|e| {
            (
                e["actor_name"].as_str().unwrap(),
                e["object_type"].as_str().unwrap(),
                e["details"]["name"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        created,
        [
            ("alice", "profile", "domain admin"),
            ("bob", "profile", "admin"),
            ("bob", "profile", "admin"),
        ]
    );
    assert!(!log.to_string().contains(PASSWORD));
    assert!(!tree(&f.app, &f.alice).await.to_string().contains(PASSWORD));
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_tree_lists_the_profiles_one_sees_with_their_folders_role(pool: PgPool) {
    let f = fixture(pool).await;
    let top = profile(&f.app, &f.alice, None, "domain admin", login("admin")).await;
    for (folder, name) in [(&f.customers, "customers admin"), (&f.a, "a admin")] {
        profile(&f.app, &f.alice, Some(folder), name, login("admin")).await;
    }
    let root = profile(&f.app, &f.alice, Some(&f.servers), "root", login("root")).await;
    let b_admin = profile(&f.app, &f.alice, Some(&f.b), "b admin", login("admin")).await;

    let all = tree(&f.app, &f.alice).await;
    assert_eq!(
        profiles(&all),
        [
            ("a admin", "manage"),
            ("b admin", "manage"),
            ("customers admin", "manage"),
            ("domain admin", "manage"),
            ("root", "manage"),
        ]
    );
    // What identifies the login, never its password.
    let shown = entry(&all, &root);
    assert!(
        shown["updated_at"].as_str().unwrap().ends_with('Z'),
        "{shown}"
    );
    assert_eq!(
        shown,
        json!({
            "id": root, "folder_id": f.servers, "name": "root", "username": "root", "domain": "",
            "secret_kind": "password", "key_algorithm": null, "key_fingerprint": null,
            "has_certificate": false, "updated_at": shown["updated_at"], "role": "manage",
        })
    );
    assert_eq!(entry(&all, &top)["folder_id"], Value::Null);

    // reveal on A holds for the profiles in A and below it …
    let bob = sign_in(&f.app, "bob").await;
    grant(&f.app, &f.alice, "folder", &f.a, BOB_SID, "reveal").await;
    assert_eq!(
        profiles(&tree(&f.app, &bob).await),
        [("a admin", "reveal"), ("root", "reveal")]
    );
    // … and a group's grant on Customers for every one below, but not for
    // the one at the top level.
    grant(&f.app, &f.alice, "folder", &f.customers, OPS_SID, "connect").await;
    let olaf = sign_in(&f.app, "olaf").await;
    assert_eq!(
        profiles(&tree(&f.app, &olaf).await),
        [
            ("a admin", "connect"),
            ("b admin", "connect"),
            ("customers admin", "connect"),
            ("root", "connect"),
        ]
    );

    // A grant on a device is none on the profile it signs in with.
    let b01 = create(
        &f.app,
        &f.alice,
        "/api/devices",
        device(&f.b, "b01", &b_admin),
    )
    .await;
    grant(&f.app, &f.alice, "device", &b01, BOB_SID, "manage").await;
    assert_eq!(
        profiles(&tree(&f.app, &bob).await),
        [("a admin", "reveal"), ("root", "reveal")]
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_device_uses_only_a_profile_in_its_folder_or_above(pool: PgPool) {
    let f = fixture(pool).await;
    let add = |folder: Option<&str>, name: &str| {
        let (app, alice) = (&f.app, &f.alice);
        let (folder, name) = (folder.map(str::to_owned), name.to_owned());
        async move { profile(app, alice, folder.as_deref(), &name, login("admin")).await }
    };
    let top = add(None, "domain admin").await;
    let customers = add(Some(&f.customers), "customers admin").await;
    let a = add(Some(&f.a), "a admin").await;
    let servers = add(Some(&f.servers), "servers admin").await;
    let new = |body: Value| {
        let (app, alice) = (&f.app, &f.alice);
        async move { call(app, alice, "POST", "/api/devices", Some(body)).await }
    };

    // The device's own folder, one above, and the top level: in reach.
    for (folder, name, profile) in [
        (&f.servers, "s1", &servers),
        (&f.servers, "s2", &a),
        (&f.servers, "s3", &customers),
        (&f.servers, "s4", &top),
        (&f.b, "b1", &customers),
        (&f.b, "b2", &top),
    ] {
        let made = new(device(folder, name, profile)).await;
        assert_eq!(made.status, StatusCode::CREATED, "{name}: {}", made.json());
    }
    // A sibling folder or one below: out of reach.
    for (folder, name, profile) in [
        (&f.b, "b3", &a),
        (&f.b, "b4", &servers),
        (&f.a, "a1", &servers),
    ] {
        let refused = new(device(folder, name, profile)).await;
        assert_eq!(
            (refused.status, refused.code().as_str()),
            (StatusCode::CONFLICT, "profile_out_of_reach"),
            "{name}"
        );
    }

    // A change is held to the same: s2 goes to B only with another profile.
    let s2 = tree(&f.app, &f.alice).await["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "s2")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let uri = format!("/api/devices/{s2}");
    let moved = call(&f.app, &f.alice, "PUT", &uri, Some(device(&f.b, "s2", &a))).await;
    assert_eq!(moved.code(), "profile_out_of_reach");
    let moved = call(
        &f.app,
        &f.alice,
        "PUT",
        &uri,
        Some(device(&f.b, "s2", &customers)),
    )
    .await;
    assert_eq!(moved.status, StatusCode::NO_CONTENT, "{}", moved.json());
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn linking_a_profile_or_retargeting_it_needs_connect_on_it(pool: PgPool) {
    let f = fixture(pool).await;
    // bob edits his team's folder A/Servers and may only list A.
    grant(&f.app, &f.alice, "folder", &f.servers, BOB_SID, "edit").await;
    grant(&f.app, &f.alice, "folder", &f.a, BOB_SID, "list").await;
    let a_admin = profile(&f.app, &f.alice, Some(&f.a), "a admin", login("admin")).await;
    let root = profile(&f.app, &f.alice, Some(&f.servers), "root", login("root")).await;
    let site = create(
        &f.app,
        &f.alice,
        "/api/connectors",
        json!({ "name": "Site" }),
    )
    .await;
    let bob = sign_in(&f.app, "bob").await;

    let linked = call(
        &f.app,
        &bob,
        "POST",
        "/api/devices",
        Some(device(&f.servers, "evil", &a_admin)),
    )
    .await;
    assert_eq!(
        (linked.status, linked.code().as_str()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
    create(
        &f.app,
        &bob,
        "/api/devices",
        device(&f.servers, "own", &root),
    )
    .await;

    // A device alice linked to a admin: bob may rename it, but not send the
    // profile to another host, port or connector.
    let db01 = create(
        &f.app,
        &f.alice,
        "/api/devices",
        device(&f.servers, "db01", &a_admin),
    )
    .await;
    let uri = format!("/api/devices/{db01}");
    let put = |changes: Value, profile: &str| {
        let mut body = device(&f.servers, "db01", profile);
        body.as_object_mut()
            .unwrap()
            .extend(changes.as_object().unwrap().clone());
        let (app, bob, uri) = (&f.app, &bob, &uri);
        async move { call(app, bob, "PUT", uri, Some(body)).await }
    };
    let renamed = put(json!({ "name": "db01 (old)" }), &a_admin).await;
    assert_eq!(renamed.status, StatusCode::NO_CONTENT, "{}", renamed.json());
    for changes in [
        json!({ "host": "attacker.example" }),
        json!({ "port": 2222 }),
        json!({ "connector_mode": "connector", "connector_id": site }),
    ] {
        assert_eq!(
            put(changes.clone(), &a_admin).await.code(),
            "forbidden",
            "{changes}"
        );
    }
    // Linking is using, on the same target, too: bob may switch to his own
    // profile, but not back.
    assert_eq!(put(json!({}), &root).await.status, StatusCode::NO_CONTENT);
    assert_eq!(put(json!({}), &a_admin).await.code(), "forbidden");

    // With connect on A, he may.
    grant(&f.app, &f.alice, "folder", &f.a, BOB_SID, "connect").await;
    let moved = put(json!({ "host": "db01.example.com" }), &a_admin).await;
    assert_eq!(moved.status, StatusCode::NO_CONTENT, "{}", moved.json());
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn an_ssh_key_profile_serves_ssh_devices_only(pool: PgPool) {
    let f = fixture(pool).await;
    let key = profile(
        &f.app,
        &f.alice,
        Some(&f.a),
        "git key",
        json!({ "secret_kind": "ssh_key", "username": "git", "private_key": lab_key("tester_ed25519") }),
    )
    .await;
    let with_protocol = |name: &str, profile: &str, protocol: &str, port: u16| {
        let mut body = device(&f.a, name, profile);
        body["protocol"] = json!(protocol);
        body["port"] = json!(port);
        body
    };
    for (protocol, port) in [("rdp", 3389), ("vnc", 5900), ("https", 443)] {
        let refused = call(
            &f.app,
            &f.alice,
            "POST",
            "/api/devices",
            Some(with_protocol(protocol, &key, protocol, port)),
        )
        .await;
        assert_eq!(
            (refused.code().as_str(), &refused.json()["params"]["field"]),
            ("invalid_request", &json!("profile_id")),
            "{protocol}"
        );
    }
    let git01 = create(
        &f.app,
        &f.alice,
        "/api/devices",
        device(&f.a, "git01", &key),
    )
    .await;
    let desktop = call(
        &f.app,
        &f.alice,
        "PUT",
        &format!("/api/devices/{git01}"),
        Some(with_protocol("git01", &key, "rdp", 3389)),
    )
    .await;
    assert_eq!(desktop.json()["params"]["field"], "profile_id");

    // Nor does a profile become a key while a desktop signs in with it.
    let desk = profile(&f.app, &f.alice, Some(&f.a), "desk", login("admin")).await;
    create(
        &f.app,
        &f.alice,
        "/api/devices",
        with_protocol("desk01", &desk, "rdp", 3389),
    )
    .await;
    let keyed = call(
        &f.app,
        &f.alice,
        "PUT",
        &format!("/api/profiles/{desk}"),
        Some(json!({
            "folder_id": f.a, "name": "desk", "username": "admin", "secret_kind": "ssh_key",
            "private_key": lab_key("tester_ed25519"),
        })),
    )
    .await;
    assert_eq!(
        (
            keyed.code().as_str(),
            &keyed.json()["params"]["field"],
            &keyed.json()["params"]["devices"]
        ),
        ("invalid_request", &json!("secret_kind"), &json!("desk01"))
    );
    assert_eq!(
        entry(&tree(&f.app, &f.alice).await, &desk)["secret_kind"],
        "password"
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn ssh_keys_are_checked_sealed_and_shown_only_by_fingerprint(pool: PgPool) {
    let f = fixture(pool.clone()).await;
    let key = lab_key("tester_ed25519_cert");
    let certificate = lab_key("tester_ed25519_cert-cert.pub");
    let body = |extra: Value| {
        let mut body = json!({
            "folder_id": f.a, "name": "key", "secret_kind": "ssh_key", "username": "tester",
        });
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
        let response = call(&f.app, &f.alice, "POST", "/api/profiles", Some(body(extra))).await;
        assert_eq!(response.code(), "invalid_request");
        assert_eq!(response.json()["params"]["field"], field);
    }

    let id = create(
        &f.app,
        &f.alice,
        "/api/profiles",
        body(json!({ "private_key": key, "certificate": certificate })),
    )
    .await;
    let everything = tree(&f.app, &f.alice).await;
    let shown = entry(&everything, &id);
    assert_eq!(shown["secret_kind"], "ssh_key");
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
    assert_eq!(
        sealed(&pool, &id).await,
        [(1, "certificate".to_owned()), (1, "private_key".to_owned())]
    );

    // Another kind needs its secret. A new secret replaces the old one: a
    // profile keeps only the secret its devices sign in with.
    let uri = format!("/api/profiles/{id}");
    let put = |extra: Value| call(&f.app, &f.alice, "PUT", &uri, Some(body(extra)));
    let change = put(json!({ "secret_kind": "password" })).await;
    assert_eq!(change.json()["params"]["field"], "password");
    let renew = put(json!({ "private_key": lab_key("tester_ed25519") })).await;
    assert_eq!(renew.status, StatusCode::NO_CONTENT, "{}", renew.json());
    let renewed = entry(&tree(&f.app, &f.alice).await, &id);
    assert_eq!(renewed["has_certificate"], false);
    assert_ne!(renewed["key_fingerprint"], shown["key_fingerprint"]);
    assert_eq!(sealed(&pool, &id).await, [(2, "private_key".to_owned())]);
    // Without a secret, the key stays.
    assert_eq!(put(json!({})).await.status, StatusCode::NO_CONTENT);
    let kept = entry(&tree(&f.app, &f.alice).await, &id);
    assert_eq!(kept["key_fingerprint"], renewed["key_fingerprint"]);
    assert_eq!(sealed(&pool, &id).await, [(2, "private_key".to_owned())]);
    // A password instead of the key.
    let password = put(json!({ "secret_kind": "password", "password": PASSWORD })).await;
    assert_eq!(
        password.status,
        StatusCode::NO_CONTENT,
        "{}",
        password.json()
    );
    let now = entry(&tree(&f.app, &f.alice).await, &id);
    assert_eq!(
        (
            &now["secret_kind"],
            &now["key_algorithm"],
            &now["key_fingerprint"],
            &now["has_certificate"]
        ),
        (
            &json!("password"),
            &Value::Null,
            &Value::Null,
            &json!(false)
        )
    );
    assert_eq!(sealed(&pool, &id).await, [(3, "password".to_owned())]);

    let log = call(&f.app, &f.alice, "GET", "/api/audit", None)
        .await
        .json();
    let updates: Vec<bool> = log
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == "profile.updated")
        .map(|e| e["details"]["secret_changed"].as_bool().unwrap())
        .collect();
    // Newest first; the refused change left nothing.
    assert_eq!(updates, [true, false, true]);
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_move_that_takes_a_profile_out_of_reach_is_refused(pool: PgPool) {
    let f = fixture(pool).await;
    let a_admin = profile(&f.app, &f.alice, Some(&f.a), "a admin", login("admin")).await;
    create(
        &f.app,
        &f.alice,
        "/api/devices",
        device(&f.servers, "s01", &a_admin),
    )
    .await;
    let move_servers = |parent: &str| {
        let uri = format!("/api/folders/{}", f.servers);
        let body = json!({ "parent_id": parent });
        let (app, alice) = (&f.app, &f.alice);
        async move { call(app, alice, "PATCH", &uri, Some(body)).await }
    };
    let move_profile = |folder: Value| {
        let uri = format!("/api/profiles/{a_admin}");
        let body = json!({ "folder_id": folder, "name": "a admin", "username": "admin" });
        let (app, alice) = (&f.app, &f.alice);
        async move { call(app, alice, "PUT", &uri, Some(body)).await }
    };
    let stranded = |response: Response| {
        (
            response.status,
            response.code(),
            response.json()["params"]["devices"].clone(),
        )
    };
    let refused = (
        StatusCode::CONFLICT,
        "profile_out_of_reach".to_owned(),
        json!("s01"),
    );

    // Servers into B would leave s01 without its profile in A, and so would
    // the profile moved into B.
    assert_eq!(stranded(move_servers(&f.b).await), refused);
    assert_eq!(stranded(move_profile(json!(f.b)).await), refused);
    let now = tree(&f.app, &f.alice).await;
    let servers = now["folders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == f.servers.as_str())
        .unwrap()
        .clone();
    assert_eq!(servers["parent_id"], f.a.as_str());
    assert_eq!(entry(&now, &a_admin)["folder_id"], f.a.as_str());

    // Down into the device's own folder, the profile goes along with it.
    let down = move_profile(json!(f.servers)).await;
    assert_eq!(down.status, StatusCode::NO_CONTENT, "{}", down.json());
    let along = move_servers(&f.b).await;
    assert_eq!(along.status, StatusCode::NO_CONTENT, "{}", along.json());
    // Up above the device or to the top level keeps it in reach, too.
    for folder in [json!(f.customers), Value::Null] {
        let up = move_profile(folder.clone()).await;
        assert_eq!(up.status, StatusCode::NO_CONTENT, "{folder}: {}", up.json());
    }
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_profile_goes_only_when_no_device_uses_it(pool: PgPool) {
    let f = fixture(pool.clone()).await;
    let a_admin = profile(&f.app, &f.alice, Some(&f.a), "a admin", login("admin")).await;
    let a01 = create(
        &f.app,
        &f.alice,
        "/api/devices",
        device(&f.a, "a01", &a_admin),
    )
    .await;
    let a02 = create(
        &f.app,
        &f.alice,
        "/api/devices",
        device(&f.servers, "a02", &a_admin),
    )
    .await;
    let uri = format!("/api/profiles/{a_admin}");

    // Deleting takes edit.
    grant(&f.app, &f.alice, "folder", &f.a, BOB_SID, "connect").await;
    let bob = sign_in(&f.app, "bob").await;
    assert_eq!(
        call(&f.app, &bob, "DELETE", &uri, None).await.code(),
        "forbidden"
    );
    let in_use = call(&f.app, &f.alice, "DELETE", &uri, None).await;
    assert_eq!(
        (
            in_use.status,
            in_use.code().as_str(),
            &in_use.json()["params"]["devices"]
        ),
        (StatusCode::CONFLICT, "profile_in_use", &json!("a01, a02"))
    );
    assert_eq!(sealed(&pool, &a_admin).await, [(1, "password".to_owned())]);

    // Once a01 asks and a02 is gone, the profile goes with its password.
    let mut asking = device(&f.a, "a01", &a_admin);
    asking["auth_mode"] = json!("ask");
    asking.as_object_mut().unwrap().remove("profile_id");
    let changed = call(
        &f.app,
        &f.alice,
        "PUT",
        &format!("/api/devices/{a01}"),
        Some(asking),
    )
    .await;
    assert_eq!(changed.status, StatusCode::NO_CONTENT, "{}", changed.json());
    let gone = call(
        &f.app,
        &f.alice,
        "DELETE",
        &format!("/api/devices/{a02}"),
        None,
    )
    .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT);
    let deleted = call(&f.app, &f.alice, "DELETE", &uri, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.json());
    assert!(sealed(&pool, &a_admin).await.is_empty());
    assert!(profiles(&tree(&f.app, &f.alice).await).is_empty());
    let log = call(&f.app, &f.alice, "GET", "/api/audit", None)
        .await
        .json();
    assert!(
        log.as_array()
            .unwrap()
            .iter()
            .any(|e| e["action"] == "profile.deleted" && e["object_id"] == a_admin.as_str())
    );

    // A folder with a profile in it is not empty.
    let b_admin = profile(&f.app, &f.alice, Some(&f.b), "b admin", login("admin")).await;
    let folder = format!("/api/folders/{}", f.b);
    let kept = call(&f.app, &f.alice, "DELETE", &folder, None).await;
    assert_eq!(
        (kept.status, kept.code().as_str()),
        (StatusCode::CONFLICT, "folder_not_empty")
    );
    call(
        &f.app,
        &f.alice,
        "DELETE",
        &format!("/api/profiles/{b_admin}"),
        None,
    )
    .await;
    assert_eq!(
        call(&f.app, &f.alice, "DELETE", &folder, None).await.status,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn revealing_a_profile_needs_reveal_and_is_audited(pool: PgPool) {
    let f = fixture(pool).await;
    let a_admin = profile(
        &f.app,
        &f.alice,
        Some(&f.a),
        "a admin",
        json!({ "username": "admin", "domain": "LAB", "password": PASSWORD }),
    )
    .await;
    let top = profile(&f.app, &f.alice, None, "domain admin", login("admin")).await;
    let key = profile(
        &f.app,
        &f.alice,
        Some(&f.a),
        "key",
        json!({
            "secret_kind": "ssh_key", "username": "tester",
            "private_key": lab_key("tester_ed25519_passphrase"), "passphrase": "Key-Passw0rd!",
        }),
    )
    .await;
    let reveal = |token: &str, id: &str, purpose: &str| {
        let uri = format!("/api/profiles/{id}/reveal");
        let body = json!({ "purpose": purpose });
        let (app, token) = (&f.app, token.to_owned());
        async move { call(app, &token, "POST", &uri, Some(body)).await }
    };

    let olaf = sign_in(&f.app, "olaf").await;
    assert_eq!(reveal(&olaf, &a_admin, "show").await.code(), "not_found");
    let bob = sign_in(&f.app, "bob").await;
    grant(&f.app, &f.alice, "folder", &f.a, BOB_SID, "connect").await;
    assert_eq!(reveal(&bob, &a_admin, "show").await.code(), "forbidden");

    grant(&f.app, &f.alice, "folder", &f.a, BOB_SID, "reveal").await;
    let shown = reveal(&bob, &a_admin, "show").await;
    assert_eq!(shown.status, StatusCode::OK);
    assert_eq!(shown.headers["cache-control"], "no-store");
    assert_eq!(
        shown.json(),
        json!({ "username": "admin", "domain": "LAB", "password": PASSWORD })
    );
    assert_eq!(
        reveal(&bob, &a_admin, "copy").await.json()["password"],
        PASSWORD
    );
    let keyed = reveal(&bob, &key, "show").await.json();
    assert!(
        keyed["private_key"]
            .as_str()
            .unwrap()
            .contains("OPENSSH PRIVATE KEY"),
        "{keyed}"
    );
    assert_eq!(
        (&keyed["passphrase"], keyed.get("password")),
        (&json!("Key-Passw0rd!"), None)
    );
    // The top level is the administrators' own; an odd purpose is refused.
    assert_eq!(reveal(&bob, &top, "show").await.code(), "not_found");
    let odd = reveal(&bob, &a_admin, "print").await;
    assert_eq!(odd.json()["params"]["field"], "purpose");

    let log = call(&f.app, &f.alice, "GET", "/api/audit", None)
        .await
        .json();
    let revealed: Vec<(&str, &str, &str, &str)> = log
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == "credential.revealed")
        .map(|e| {
            (
                e["actor_name"].as_str().unwrap(),
                e["object_type"].as_str().unwrap(),
                e["object_id"].as_str().unwrap(),
                e["details"]["purpose"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        revealed,
        [
            ("bob", "profile", key.as_str(), "show"),
            ("bob", "profile", a_admin.as_str(), "copy"),
            ("bob", "profile", a_admin.as_str(), "show"),
        ]
    );
    assert!(!log.to_string().contains(PASSWORD));
}

/// The two trees stay apart (#190): a collection's grants reach no profile,
/// not even one in a folder that shares the collection's id, as the
/// migration of #190 left folders and collections.
#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_folders_grants_reach_its_profiles_and_a_collections_do_not(pool: PgPool) {
    let f = fixture(pool.clone()).await;
    let a_admin = profile(&f.app, &f.alice, Some(&f.a), "a admin", login("admin")).await;
    sqlx::query("INSERT INTO collections (id, name) VALUES ($1::uuid, 'Twin')")
        .bind(&f.a)
        .execute(&pool)
        .await
        .unwrap();
    grant(&f.app, &f.alice, "collection", &f.a, BOB_SID, "manage").await;
    let bob = sign_in(&f.app, "bob").await;
    let reveal = format!("/api/profiles/{a_admin}/reveal");
    let show = || Some(json!({ "purpose": "show" }));

    assert!(profiles(&tree(&f.app, &bob).await).is_empty());
    assert_eq!(
        call(&f.app, &bob, "POST", &reveal, show()).await.code(),
        "not_found"
    );
    // Nor does a profile take a grant of its own.
    let own = call(
        &f.app,
        &f.alice,
        "POST",
        "/api/grants",
        Some(json!({
            "object": { "kind": "profile", "id": a_admin },
            "principal_kind": "user", "principal_sid": BOB_SID, "principal_name": "bob",
            "role": "reveal",
        })),
    )
    .await;
    assert_eq!(
        (own.code().as_str(), &own.json()["params"]["field"]),
        ("invalid_request", &json!("kind"))
    );

    grant(&f.app, &f.alice, "folder", &f.a, BOB_SID, "reveal").await;
    assert_eq!(profiles(&tree(&f.app, &bob).await), [("a admin", "reveal")]);
    let shown = call(&f.app, &bob, "POST", &reveal, show()).await;
    assert_eq!(shown.json()["password"], PASSWORD);
}
