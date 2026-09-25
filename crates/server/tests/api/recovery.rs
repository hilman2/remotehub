//! The organisation recovery key (#95, #96, ADR 0009): alice administers,
//! bob owns a vault, olaf becomes a security officer.

use axum::Router;
use axum::http::StatusCode;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::sync::Arc;

use remotehub_server::{AppState, app, escrow as kept};
use remotehub_vault::{FileKeyring, KeyProvider, Vault, escrow, generate_key_line};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{
    ALICE_SID, FakeDirectory, OLAF_SID, Response, authed, send, settings, sign_in_request, state,
};

async fn token(app: &Router, user: &str) -> String {
    send(app, sign_in_request(user, "right"))
        .await
        .session_token()
        .unwrap()
}

async fn call(app: &Router, token: &str, method: &str, uri: &str, body: Option<Value>) -> Response {
    send(app, authed(method, uri, body, token)).await
}

/// 65 bytes that look like an uncompressed point; the server does not check
/// the ephemeral keys in vault wraps, only the browser that opens them.
fn point(fill: u8) -> String {
    let mut bytes = vec![fill; 65];
    bytes[0] = 4;
    STANDARD.encode(bytes)
}

/// The private key `[fill; 32]`: a scalar for any small `fill`.
fn private(fill: u8) -> [u8; 32] {
    [fill; 32]
}

/// The recovery key belonging to `private(fill)`.
fn public(fill: u8) -> String {
    STANDARD.encode(escrow::public_key_of(&private(fill)).unwrap())
}

const WRAPPED: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gISIjJCUmJw==";
const NONCE: &str = "AAECAwQFBgcICQoL";
const CIPHERTEXT: &str = "c2VhbGVkIGJ5IHRoZSBicm93c2Vy";
const ENTRY: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

async fn unlock(app: &Router, token: &str, kind: &str, params: Value) -> Response {
    let body = json!({ "kind": kind, "params": params, "wrapped_key": WRAPPED });
    call(app, token, "POST", "/api/personal/unlocks", Some(body)).await
}

async fn new_key(app: &Router, admin: &str, fill: u8) -> String {
    let body = json!({ "public_key": public(fill) });
    let created = call(app, admin, "POST", "/api/recovery-keys", Some(body)).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json());
    created.json()["id"].as_str().unwrap().to_owned()
}

/// bob's vault with a passphrase and a recovery key, wrapped for `key`.
async fn covered_vault(app: &Router, bob: &str, key: &str) {
    for kind in ["passphrase", "recovery"] {
        let added = unlock(app, bob, kind, json!({})).await;
        assert_eq!(added.status, StatusCode::CREATED, "{}", added.json());
    }
    let params = json!({ "key_id": key, "ephemeral": point(7) });
    let added = unlock(app, bob, "organisation", params).await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.json());
}

async fn assign(app: &Router, admin: &str, role: &str, sid: &str) {
    let uri = format!("/api/roles/{role}/members/{sid}");
    let body = json!({ "principal_kind": "user", "principal_name": "someone" });
    let response = call(app, admin, "PUT", &uri, Some(body)).await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{}",
        response.json()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_vault_is_wrapped_for_the_newest_key_and_keeps_that_wrap(pool: PgPool) {
    let app = app(state(pool), None);
    let alice = token(&app, "alice").await;
    let bob = token(&app, "bob").await;
    let vault = call(&app, &bob, "GET", "/api/personal/vault", None).await;
    assert_eq!(vault.json()["organisation_key"], Value::Null);
    let early = unlock(&app, &bob, "organisation", json!({ "key_id": null })).await;
    assert_eq!(early.json()["params"]["field"], "params");

    // Only administrators handle keys.
    let body = json!({ "public_key": public(1) });
    for (method, uri) in [
        ("GET", "/api/recovery-keys"),
        ("POST", "/api/recovery-keys"),
    ] {
        let response = call(&app, &bob, method, uri, Some(body.clone())).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    // 65 bytes, but not a point on the curve.
    let bent = json!({ "public_key": point(1) });
    let refused = call(&app, &alice, "POST", "/api/recovery-keys", Some(bent)).await;
    assert_eq!(refused.json()["params"]["field"], "public_key");

    let first = new_key(&app, &alice, 1).await;
    let vault = call(&app, &bob, "GET", "/api/personal/vault", None).await;
    assert_eq!(
        vault.json()["organisation_key"],
        json!({ "id": first, "public_key": public(1) })
    );
    let short = json!({ "key_id": first, "ephemeral": STANDARD.encode([4u8; 33]) });
    assert_eq!(
        unlock(&app, &bob, "organisation", short).await.json()["params"]["field"],
        "params"
    );
    covered_vault(&app, &bob, &first).await;

    // The wrap is the company's: the owner cannot remove it, and it does not
    // count as the owner's last way in.
    let unlocks = call(&app, &bob, "GET", "/api/personal/vault", None)
        .await
        .json()["unlocks"]
        .clone();
    let id_of = |kind: &str| {
        unlocks
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["kind"] == kind)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let uri = |kind: &str| format!("/api/personal/unlocks/{}", id_of(kind));
    let kept = call(&app, &bob, "DELETE", &uri("organisation"), None).await;
    assert_eq!(kept.status, StatusCode::FORBIDDEN);
    let gone = call(&app, &bob, "DELETE", &uri("passphrase"), None).await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT);
    let last = call(&app, &bob, "DELETE", &uri("recovery"), None).await;
    assert_eq!(last.code(), "last_unlock");

    // A new key: wraps for the old one are refused from now on, and the old
    // one stays while a vault depends on it.
    let second = new_key(&app, &alice, 2).await;
    let old = json!({ "key_id": first, "ephemeral": point(7) });
    assert_eq!(
        unlock(&app, &bob, "organisation", old).await.status,
        StatusCode::BAD_REQUEST
    );
    let keys = call(&app, &alice, "GET", "/api/recovery-keys", None)
        .await
        .json();
    let counts: Vec<(&str, i64)> = keys["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| (k["id"].as_str().unwrap(), k["vaults"].as_i64().unwrap()))
        .collect();
    assert_eq!(counts, [(second.as_str(), 0), (first.as_str(), 1)]);
    assert_eq!(keys["vaults"][0]["username"], "bob");
    assert_eq!(keys["vaults"][0]["key_id"], first.as_str());
    for key in [&first, &second] {
        let refused = call(
            &app,
            &alice,
            "DELETE",
            &format!("/api/recovery-keys/{key}"),
            None,
        )
        .await;
        assert_eq!(refused.code(), "recovery_key_in_use");
    }
    let params = json!({ "key_id": second, "ephemeral": point(7) });
    assert_eq!(
        unlock(&app, &bob, "organisation", params).await.status,
        StatusCode::CREATED
    );
    let deleted = call(
        &app,
        &alice,
        "DELETE",
        &format!("/api/recovery-keys/{first}"),
        None,
    )
    .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_recovery_needs_a_security_officer_who_did_not_ask(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let alice = token(&app, "alice").await;
    let bob = token(&app, "bob").await;
    let olaf = token(&app, "olaf").await;
    let key = new_key(&app, &alice, 1).await;
    covered_vault(&app, &bob, &key).await;
    let sealed = json!({ "nonce": NONCE, "ciphertext": CIPHERTEXT });
    let entry = format!("/api/personal/entries/{ENTRY}");
    call(&app, &bob, "PUT", &entry, Some(sealed.clone())).await;
    let file = format!("/api/personal/attachments/{ENTRY}");
    call(&app, &bob, "PUT", &file, Some(sealed.clone())).await;
    let keys = call(&app, &alice, "GET", "/api/recovery-keys", None)
        .await
        .json();
    let bob_id = keys["vaults"][0]["user_id"].as_str().unwrap().to_owned();

    let ask = |user: &str| json!({ "user_id": user, "kind": "passphrase", "reason": "Forgot it" });
    let refused = call(
        &app,
        &bob,
        "POST",
        "/api/vault-recoveries",
        Some(ask(&bob_id)),
    )
    .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    let asked = call(
        &app,
        &alice,
        "POST",
        "/api/vault-recoveries",
        Some(ask(&bob_id)),
    )
    .await;
    assert_eq!(asked.status, StatusCode::CREATED, "{}", asked.json());
    let id = asked.json()["id"].as_str().unwrap().to_owned();
    let again = call(
        &app,
        &alice,
        "POST",
        "/api/vault-recoveries",
        Some(ask(&bob_id)),
    )
    .await;
    assert_eq!(again.code(), "recovery_open");
    let olaf_id: String = sqlx::query_scalar("SELECT id::text FROM users WHERE username = 'olaf'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let bare = call(
        &app,
        &alice,
        "POST",
        "/api/vault-recoveries",
        Some(ask(&olaf_id)),
    )
    .await;
    assert_eq!(bare.code(), "vault_not_covered");

    let open = format!("/api/vault-recoveries/{id}/vault");
    let approve = format!("/api/vault-recoveries/{id}/approve");
    assert_eq!(
        call(&app, &alice, "GET", &open, None).await.code(),
        "recovery_not_ready"
    );
    // Administrators do not approve; a security officer does, but not their
    // own request.
    assert_eq!(
        call(&app, &alice, "POST", &approve, None).await.status,
        StatusCode::FORBIDDEN
    );
    assign(&app, &alice, "security_officer", ALICE_SID).await;
    assert_eq!(
        call(&app, &alice, "POST", &approve, None).await.code(),
        "own_request"
    );
    assign(&app, &alice, "security_officer", OLAF_SID).await;
    let listed = call(&app, &olaf, "GET", "/api/vault-recoveries", None)
        .await
        .json();
    assert_eq!(listed[0]["status"], "pending");
    assert_eq!(listed[0]["mine"], false);
    let approved = call(&app, &olaf, "POST", &approve, None).await;
    assert_eq!(
        approved.status,
        StatusCode::NO_CONTENT,
        "{}",
        approved.json()
    );
    assert_eq!(
        call(&app, &olaf, "POST", &approve, None).await.code(),
        "request_decided"
    );

    // Only the requester opens it.
    assert_eq!(
        call(&app, &olaf, "GET", &open, None).await.status,
        StatusCode::FORBIDDEN
    );
    let opened = call(&app, &alice, "GET", &open, None).await.json();
    assert_eq!(opened["unlock"]["wrapped_key"], WRAPPED);
    assert_eq!(opened["unlock"]["public_key"], public(1));
    assert_eq!(opened["unlock"]["params"]["key_id"], key.as_str());
    assert_eq!(opened["entries"][0]["ciphertext"], CIPHERTEXT);
    let attachment = format!("/api/vault-recoveries/{id}/attachments/{ENTRY}");
    assert_eq!(
        call(&app, &alice, "GET", &attachment, None).await.json(),
        sealed
    );

    // A forgotten passphrase: the owner's recovery key is replaced by a
    // one-time one.
    let complete = format!("/api/vault-recoveries/{id}/complete");
    let missing = call(&app, &alice, "POST", &complete, Some(json!({}))).await;
    assert_eq!(missing.json()["params"]["field"], "wrapped_key");
    let one_time = "b25lLXRpbWUgcmVjb3Zlcnkga2V5IHdyYXA=";
    let done = call(
        &app,
        &alice,
        "POST",
        &complete,
        Some(json!({ "wrapped_key": one_time })),
    )
    .await;
    assert_eq!(done.status, StatusCode::NO_CONTENT, "{}", done.json());
    let unlocks = call(&app, &bob, "GET", "/api/personal/vault", None)
        .await
        .json()["unlocks"]
        .clone();
    let recovery: Vec<&Value> = unlocks
        .as_array()
        .unwrap()
        .iter()
        .filter(|u| u["kind"] == "recovery")
        .collect();
    assert_eq!(recovery.len(), 1);
    assert_eq!(recovery[0]["wrapped_key"], one_time);
    assert_eq!(recovery[0]["params"], json!({ "one_time": true }));
    assert_eq!(
        call(&app, &alice, "GET", &open, None).await.code(),
        "recovery_not_ready"
    );

    let log = call(&app, &alice, "GET", "/api/audit", None).await.json();
    let actions: Vec<&str> = log
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["action"].as_str())
        .filter(|a| a.starts_with("vault_recovery."))
        .collect();
    assert_eq!(
        actions,
        [
            "vault_recovery.completed",
            "vault_recovery.opened",
            "vault_recovery.approved",
            "vault_recovery.requested"
        ]
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_approval_holds_for_a_day(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let alice = token(&app, "alice").await;
    let bob = token(&app, "bob").await;
    let olaf = token(&app, "olaf").await;
    let key = new_key(&app, &alice, 1).await;
    covered_vault(&app, &bob, &key).await;
    assign(&app, &alice, "security_officer", OLAF_SID).await;
    let keys = call(&app, &alice, "GET", "/api/recovery-keys", None)
        .await
        .json();
    let body =
        json!({ "user_id": keys["vaults"][0]["user_id"], "kind": "handover", "reason": "Left" });
    let asked = call(&app, &alice, "POST", "/api/vault-recoveries", Some(body)).await;
    let id = asked.json()["id"].as_str().unwrap().to_owned();
    call(
        &app,
        &olaf,
        "POST",
        &format!("/api/vault-recoveries/{id}/approve"),
        None,
    )
    .await;
    let open = format!("/api/vault-recoveries/{id}/vault");
    assert_eq!(
        call(&app, &alice, "GET", &open, None).await.status,
        StatusCode::OK
    );

    sqlx::query("UPDATE vault_recoveries SET approved_at = now() - interval '25 hours'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, &alice, "GET", &open, None).await.code(),
        "recovery_not_ready"
    );
    let listed = call(&app, &alice, "GET", "/api/vault-recoveries", None)
        .await
        .json();
    assert_eq!(listed[0]["status"], "expired");
    assert_eq!(listed[0]["mine"], true);
    // Taken back, a new one may be asked for.
    let cancelled = call(
        &app,
        &alice,
        "DELETE",
        &format!("/api/vault-recoveries/{id}"),
        None,
    )
    .await;
    assert_eq!(cancelled.status, StatusCode::NO_CONTENT);
    let listed = call(&app, &alice, "GET", "/api/vault-recoveries", None)
        .await
        .json();
    assert_eq!(listed, json!([]));
}

/// A key file with the master key versions `1..=versions`, as after
/// rotations.
fn keyring(versions: i32) -> String {
    (1..=versions)
        .map(|v| generate_key_line(v).as_str().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// #96: the master keys are kept for the newest recovery key, and its
/// private key alone brings back the key file.
#[sqlx::test(migrations = "../../migrations")]
async fn the_master_key_file_comes_back_with_the_recovery_key(pool: PgPool) {
    let file = keyring(2);
    let vault = Vault::new(Box::new(FileKeyring::parse(&file).unwrap()) as Box<dyn KeyProvider>);
    let app = app(
        AppState::new(
            pool.clone(),
            Some(Arc::new(FakeDirectory)),
            settings(),
            vault,
        ),
        None,
    );
    let alice = token(&app, "alice").await;
    let first = new_key(&app, &alice, 1).await;
    let keys = call(&app, &alice, "GET", "/api/recovery-keys", None)
        .await
        .json();
    assert_eq!(keys["keys"][0]["master_key"], true);

    let recovered = kept::recover(&pool, &private(1)).await.unwrap();
    let original = FileKeyring::parse(&file).unwrap();
    let back = FileKeyring::parse(&recovered).unwrap();
    assert_eq!(back.all(), [("file", 1), ("file", 2)]);
    for version in [1, 2] {
        assert_eq!(
            back.master_key("file", version).unwrap(),
            original.master_key("file", version).unwrap()
        );
    }
    let stranger = kept::recover(&pool, &private(3)).await.unwrap_err();
    assert!(
        stranger.to_string().contains("no recovery key"),
        "{stranger}"
    );

    // A version added to the file is kept at the next start, once.
    let rotated =
        Vault::new(Box::new(FileKeyring::parse(&keyring(3)).unwrap()) as Box<dyn KeyProvider>);
    assert_eq!(kept::keep(&pool, &rotated).await.unwrap(), 1);
    assert_eq!(kept::keep(&pool, &rotated).await.unwrap(), 0);

    // A new recovery key holds the master keys too; the old one keeps them
    // until it is deleted.
    new_key(&app, &alice, 2).await;
    assert!(kept::recover(&pool, &private(2)).await.is_ok());
    let deleted = call(
        &app,
        &alice,
        "DELETE",
        &format!("/api/recovery-keys/{first}"),
        None,
    )
    .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert!(kept::recover(&pool, &private(1)).await.is_err());
}
