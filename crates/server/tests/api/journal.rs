//! The device journal and who states a purpose before connecting (#90).

use axum::Router;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{BOB_SID, OPS_SID, authed, get, send, sign_in_request};
use crate::terminal::{create, setup};

async fn sign_in(app: &Router, username: &str) -> String {
    send(app, sign_in_request(username, "right"))
        .await
        .session_token()
        .unwrap()
}

async fn purpose_required(app: &Router, token: &str) -> bool {
    send(app, get("/api/tree", Some(token))).await.json()["purpose_required"]
        .as_bool()
        .unwrap()
}

fn require(sid: &str, kind: &str, token: &str) -> axum::http::Request<axum::body::Body> {
    authed(
        "PUT",
        &format!("/api/purpose-principals/{sid}"),
        Some(json!({ "principal_kind": kind, "principal_name": "someone" })),
        token,
    )
}

async fn actions(app: &Router, token: &str) -> Vec<String> {
    let log = send(app, get("/api/audit", Some(token))).await.json();
    log.as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap().to_owned())
        .collect()
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn administrators_choose_who_states_a_purpose(pool: PgPool) {
    let (_, app, alice, _) = setup(pool).await;
    let bob = sign_in(&app, "bob").await;
    let olaf = sign_in(&app, "olaf").await;

    // Only administrators see and change the list.
    assert_eq!(
        send(&app, get("/api/purpose-principals", Some(&bob)))
            .await
            .status,
        403
    );
    assert_eq!(
        send(&app, require(OPS_SID, "group", &bob)).await.status,
        403
    );
    assert_eq!(
        send(&app, require("S-1-not-a-sid", "group", &alice))
            .await
            .code(),
        "invalid_request"
    );

    // A group: its members state a purpose, others do not.
    assert!(!purpose_required(&app, &olaf).await);
    assert_eq!(
        send(&app, require(OPS_SID, "group", &alice)).await.status,
        204
    );
    assert_eq!(
        send(&app, require(OPS_SID, "group", &alice)).await.status,
        204
    );
    assert!(purpose_required(&app, &olaf).await);
    assert!(!purpose_required(&app, &bob).await);
    // A user, by their own SID.
    assert_eq!(
        send(&app, require(BOB_SID, "user", &alice)).await.status,
        204
    );
    assert!(purpose_required(&app, &bob).await);
    assert!(!purpose_required(&app, &alice).await);

    let listed = send(&app, get("/api/purpose-principals", Some(&alice)))
        .await
        .json();
    let sids: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["principal_sid"].as_str().unwrap())
        .collect();
    assert_eq!(sids.len(), 2, "{listed}");
    assert!(sids.contains(&OPS_SID) && sids.contains(&BOB_SID));

    let path = format!("/api/purpose-principals/{OPS_SID}");
    assert_eq!(
        send(&app, authed("DELETE", &path, None, &bob)).await.status,
        403
    );
    assert_eq!(
        send(&app, authed("DELETE", &path, None, &alice))
            .await
            .status,
        204
    );
    assert_eq!(
        send(&app, authed("DELETE", &path, None, &alice))
            .await
            .status,
        404
    );
    assert!(!purpose_required(&app, &olaf).await);

    // Adding twice is one change.
    let actions = actions(&app, &alice).await;
    let count = |action: &str| actions.iter().filter(|a| *a == action).count();
    assert_eq!(count("purpose.required"), 2, "{actions:?}");
    assert_eq!(count("purpose.waived"), 1, "{actions:?}");
}

async fn grant(app: &Router, token: &str, device: &str, sid: &str, role: &str) {
    let response = send(
        app,
        authed(
            "POST",
            "/api/grants",
            Some(json!({
                "object": { "kind": "device", "id": device },
                "principal_kind": "user", "principal_sid": sid, "principal_name": "someone", "role": role,
            })),
            token,
        ),
    )
    .await;
    assert_eq!(response.status, 204, "{}", response.json());
}

fn note(device: &str, text: &str, token: &str) -> axum::http::Request<axum::body::Body> {
    authed(
        "POST",
        &format!("/api/devices/{device}/journal"),
        Some(json!({ "text": text })),
        token,
    )
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn who_may_connect_reads_the_journal_and_leaves_notes(pool: PgPool) {
    let (_, app, alice, folder) = setup(pool).await;
    let device = create(
        &app,
        &alice,
        "/api/devices",
        json!({
            "folder_id": folder, "name": "web01", "protocol": "ssh", "host": "web01",
            "port": 22, "auth_mode": "ask", "credential_id": null,
        }),
    )
    .await;
    let journal = format!("/api/devices/{device}/journal");

    assert_eq!(
        send(&app, note(&device, "  Replaced the disk.  ", &alice))
            .await
            .status,
        204
    );
    assert_eq!(
        send(&app, note(&device, "   ", &alice)).await.code(),
        "invalid_request"
    );
    assert_eq!(
        send(&app, note(&device, &"x".repeat(2001), &alice))
            .await
            .code(),
        "invalid_request"
    );
    let entries = send(&app, get(&journal, Some(&alice))).await.json();
    assert_eq!(entries.as_array().unwrap().len(), 1, "{entries}");
    assert_eq!(entries[0]["kind"], "note");
    assert_eq!(entries[0]["text"], "Replaced the disk.");
    assert_eq!(entries[0]["username"], "alice");

    // Someone who only sees the device neither reads nor writes; someone who
    // does not see it does not learn it exists.
    let bob = sign_in(&app, "bob").await;
    let olaf = sign_in(&app, "olaf").await;
    grant(&app, &alice, &device, BOB_SID, "list").await;
    assert_eq!(send(&app, get(&journal, Some(&bob))).await.status, 403);
    assert_eq!(send(&app, note(&device, "hi", &bob)).await.status, 403);
    assert_eq!(send(&app, get(&journal, Some(&olaf))).await.status, 404);

    grant(&app, &alice, &device, BOB_SID, "connect").await;
    assert_eq!(
        send(&app, note(&device, "Checked the logs.", &bob))
            .await
            .status,
        204
    );
    let entries: Value = send(&app, get(&journal, Some(&bob))).await.json();
    let texts: Vec<&str> = entries
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, ["Checked the logs.", "Replaced the disk."]);
    // Notes are the journal, not the audit log's business; the audit log
    // has no copy of them.
    assert!(
        !send(&app, get("/api/audit", Some(&alice)))
            .await
            .json()
            .to_string()
            .contains("Checked the logs.")
    );
}
