//! Just-in-time access: ask, decide, use, run out.

use axum::Router;
use axum::http::StatusCode;
use remotehub_server::app;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{BOB_SID, OPS_SID, Response, authed, get, send, sign_in_request, state};
use crate::terminal::create;

async fn token(app: &Router, user: &str) -> String {
    send(app, sign_in_request(user, "right"))
        .await
        .session_token()
        .unwrap()
}

async fn call(app: &Router, token: &str, method: &str, uri: &str, body: Option<Value>) -> Response {
    send(app, authed(method, uri, body, token)).await
}

/// alice (administrator) sets up the folder Sensitive with db01. bob sees
/// db01 (list); olaf's group manages the folder.
struct Lab {
    app: Router,
    pool: PgPool,
    alice: String,
    bob: String,
    olaf: String,
    folder: String,
    device: String,
}

async fn lab(pool: PgPool) -> Lab {
    let app = app(state(pool.clone()), None);
    let alice = token(&app, "alice").await;
    let folder = create(
        &app,
        &alice,
        "/api/folders",
        json!({ "parent_id": null, "name": "Sensitive" }),
    )
    .await;
    let device = create(
        &app,
        &alice,
        "/api/devices",
        json!({
            "folder_id": folder, "name": "db01", "protocol": "ssh", "host": "db01.example.com",
            "port": 22, "auth_mode": "ask", "credential_id": null,
        }),
    )
    .await;
    for (kind, id, sid, principal_kind, role) in [
        ("device", &device, BOB_SID, "user", "list"),
        ("folder", &folder, OPS_SID, "group", "manage"),
    ] {
        let response = call(
            &app,
            &alice,
            "POST",
            "/api/grants",
            Some(json!({
                "object": { "kind": kind, "id": id }, "principal_kind": principal_kind,
                "principal_sid": sid, "principal_name": "someone", "role": role,
            })),
        )
        .await;
        assert_eq!(response.status, StatusCode::NO_CONTENT);
    }
    let bob = token(&app, "bob").await;
    let olaf = token(&app, "olaf").await;
    Lab {
        app,
        pool,
        alice,
        bob,
        olaf,
        folder,
        device,
    }
}

impl Lab {
    async fn ask(&self, role: &str, minutes: i32, reason: &str) -> Response {
        call(
            &self.app,
            &self.bob,
            "POST",
            "/api/access-requests",
            Some(json!({
                "object": { "kind": "device", "id": self.device },
                "role": role, "minutes": minutes, "reason": reason,
            })),
        )
        .await
    }

    /// bob's role on db01, as the tree shows it.
    async fn bobs_role(&self) -> Value {
        let tree = send(&self.app, get("/api/tree", Some(&self.bob)))
            .await
            .json();
        tree["devices"][0]["role"].clone()
    }

    async fn requests(&self, token: &str) -> Value {
        send(&self.app, get("/api/access-requests", Some(token)))
            .await
            .json()
    }
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn an_approved_request_grants_access_until_it_runs_out(pool: PgPool) {
    let lab = lab(pool).await;
    assert_eq!(lab.bobs_role().await, "list");

    let asked = lab
        .ask("connect", 60, "Restart the database after the patch")
        .await;
    assert_eq!(asked.status, StatusCode::CREATED, "{}", asked.json());
    let id = asked.json()["id"].as_str().unwrap().to_owned();
    assert_eq!(
        lab.ask("connect", 60, "again").await.code(),
        "request_pending"
    );

    // olaf manages the folder and sees the request; bob sees it as his.
    let olafs = lab.requests(&lab.olaf).await;
    assert_eq!(olafs["to_decide"][0]["id"], id.as_str());
    assert_eq!(olafs["to_decide"][0]["requester_name"], "bob");
    assert_eq!(olafs["to_decide"][0]["object_name"], "db01");
    assert_eq!(
        olafs["to_decide"][0]["object"],
        json!({ "kind": "device", "id": lab.device })
    );
    assert_eq!(lab.requests(&lab.bob).await["mine"][0]["status"], "pending");
    // bob cannot decide it.
    let uri = format!("/api/access-requests/{id}/approve");
    assert_eq!(
        call(&lab.app, &lab.bob, "POST", &uri, None).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(lab.bobs_role().await, "list");

    assert_eq!(
        call(&lab.app, &lab.olaf, "POST", &uri, None).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(lab.bobs_role().await, "connect");
    let mine = &lab.requests(&lab.bob).await["mine"][0];
    assert_eq!(mine["status"], "approved");
    assert_eq!(mine["decider_name"], "olaf");
    assert!(
        mine["expires_at"].as_str().unwrap().ends_with('Z'),
        "{mine}"
    );
    assert_eq!(
        call(&lab.app, &lab.olaf, "POST", &uri, None).await.code(),
        "request_decided"
    );
    assert!(
        lab.requests(&lab.olaf).await["to_decide"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // The grant shows its end, and after it grants nothing.
    let grants = send(
        &lab.app,
        get(
            &format!("/api/grants?kind=device&id={}", lab.device),
            Some(&lab.alice),
        ),
    )
    .await
    .json();
    let jit = grants["direct"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["role"] == "connect")
        .unwrap_or_else(|| panic!("{grants}"));
    assert_eq!(jit["expires_at"], mine["expires_at"]);
    sqlx::query(
        "UPDATE grants SET expires_at = now() - interval '1 second' WHERE expires_at IS NOT NULL",
    )
    .execute(&lab.pool)
    .await
    .unwrap();
    assert_eq!(lab.bobs_role().await, "list");
    let grants = send(
        &lab.app,
        get(
            &format!("/api/grants?kind=device&id={}", lab.device),
            Some(&lab.alice),
        ),
    )
    .await
    .json();
    assert_eq!(grants["direct"].as_array().unwrap().len(), 1, "{grants}");

    let log = send(&lab.app, get("/api/audit", Some(&lab.alice)))
        .await
        .json()
        .to_string();
    for action in ["access.requested", "access.approved"] {
        assert!(log.contains(action), "{action}");
    }
    assert!(log.contains("Restart the database after the patch"));
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn denied_and_cancelled_requests_grant_nothing(pool: PgPool) {
    let lab = lab(pool).await;
    let id = lab.ask("reveal", 30, "Need the root password").await.json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let deny = format!("/api/access-requests/{id}/deny");
    assert_eq!(
        call(&lab.app, &lab.olaf, "POST", &deny, None).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(lab.bobs_role().await, "list");
    assert_eq!(lab.requests(&lab.bob).await["mine"][0]["status"], "denied");

    let id = lab.ask("reveal", 30, "Once more").await.json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let cancel = format!("/api/access-requests/{id}");
    // Only the requester takes it back.
    assert_eq!(
        call(&lab.app, &lab.olaf, "DELETE", &cancel, None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&lab.app, &lab.bob, "DELETE", &cancel, None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert!(
        lab.requests(&lab.olaf).await["to_decide"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let approve = format!("/api/access-requests/{id}/approve");
    assert_eq!(
        call(&lab.app, &lab.olaf, "POST", &approve, None)
            .await
            .code(),
        "request_decided"
    );
    assert_eq!(lab.bobs_role().await, "list");

    let log = send(&lab.app, get("/api/audit", Some(&lab.alice)))
        .await
        .json()
        .to_string();
    for action in ["access.denied", "access.cancelled"] {
        assert!(log.contains(action), "{action}");
    }
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn nobody_approves_their_own_request(pool: PgPool) {
    let lab = lab(pool).await;
    let id = lab.ask("connect", 60, "Maintenance").await.json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // bob becomes a manager of the folder while his request waits.
    let response = call(
        &lab.app,
        &lab.alice,
        "POST",
        "/api/grants",
        Some(json!({
            "object": { "kind": "folder", "id": lab.folder }, "principal_kind": "user",
            "principal_sid": BOB_SID, "principal_name": "bob", "role": "manage",
        })),
    )
    .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);
    assert!(
        lab.requests(&lab.bob).await["to_decide"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let approve = format!("/api/access-requests/{id}/approve");
    assert_eq!(
        call(&lab.app, &lab.bob, "POST", &approve, None)
            .await
            .code(),
        "own_request"
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn requests_ask_for_little_for_a_while_on_what_one_sees(pool: PgPool) {
    let lab = lab(pool).await;
    assert_eq!(
        lab.ask("edit", 60, "x").await.json()["params"]["field"],
        "role"
    );
    assert_eq!(
        lab.ask("list", 60, "x").await.json()["params"]["field"],
        "role"
    );
    assert_eq!(
        lab.ask("connect", 14, "x").await.json()["params"]["field"],
        "minutes"
    );
    assert_eq!(
        lab.ask("connect", 1441, "x").await.json()["params"]["field"],
        "minutes"
    );
    assert_eq!(
        lab.ask("connect", 60, "  ").await.json()["params"]["field"],
        "reason"
    );

    // A folder bob cannot see does not exist for him.
    let hidden = create(
        &lab.app,
        &lab.alice,
        "/api/folders",
        json!({ "parent_id": null, "name": "Hidden" }),
    )
    .await;
    let response = call(
        &lab.app,
        &lab.bob,
        "POST",
        "/api/access-requests",
        Some(json!({
            "object": { "kind": "folder", "id": hidden },
            "role": "connect", "minutes": 60, "reason": "x",
        })),
    )
    .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
}
