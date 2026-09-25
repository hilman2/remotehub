//! The personal vault stores what the browser encrypted, for its owner only.

use axum::Router;
use axum::http::StatusCode;
use remotehub_server::app;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{Response, authed, get, send, sign_in_request, state};

async fn token(app: &Router, user: &str) -> String {
    send(app, sign_in_request(user, "right"))
        .await
        .session_token()
        .unwrap()
}

async fn call(app: &Router, token: &str, method: &str, uri: &str, body: Option<Value>) -> Response {
    send(app, authed(method, uri, body, token)).await
}

/// 32 bytes of wrapped key and 12 of nonce, in base64.
const WRAPPED: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const NONCE: &str = "AAECAwQFBgcICQoL";
const CIPHERTEXT: &str = "c2VhbGVkIGJ5IHRoZSBicm93c2Vy";
const ENTRY: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

async fn set_up(app: &Router, token: &str) {
    for kind in ["passphrase", "recovery"] {
        let response = call(
            app,
            token,
            "POST",
            "/api/personal/unlocks",
            Some(json!({ "kind": kind, "params": { "salt": "c2FsdA==" }, "wrapped_key": WRAPPED })),
        )
        .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.json());
    }
}

/// What the owner picked after searching (#81): sealed like an entry, kept
/// per owner, gone with the vault, and no entry in the audit log per pick.
#[sqlx::test(migrations = "../../migrations")]
async fn search_picks_are_sealed_per_owner_and_not_audited(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let alice = token(&app, "alice").await;
    let bob = token(&app, "bob").await;
    let vault = |token: String| {
        let app = app.clone();
        async move {
            send(&app, get("/api/personal/vault", Some(&token)))
                .await
                .json()
        }
    };
    assert_eq!(vault(bob.clone()).await["search"], Value::Null);

    let audited = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit_log")
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    let before = audited().await;
    let sealed = json!({ "nonce": NONCE, "ciphertext": CIPHERTEXT });
    for _ in 0..2 {
        let saved = call(
            &app,
            &bob,
            "PUT",
            "/api/personal/search",
            Some(sealed.clone()),
        )
        .await;
        assert_eq!(saved.status, StatusCode::NO_CONTENT);
    }
    assert_eq!(audited().await, before);
    assert_eq!(vault(bob.clone()).await["search"], sealed);
    assert_eq!(vault(alice.clone()).await["search"], Value::Null);

    let short = json!({ "nonce": "AAEC", "ciphertext": CIPHERTEXT });
    let refused = call(&app, &bob, "PUT", "/api/personal/search", Some(short)).await;
    assert_eq!(refused.json()["params"]["field"], "nonce");

    let reset = call(&app, &bob, "DELETE", "/api/personal/vault", None).await;
    assert_eq!(reset.status, StatusCode::NO_CONTENT);
    assert_eq!(vault(bob).await["search"], Value::Null);
}

#[sqlx::test(migrations = "../../migrations")]
async fn only_the_owner_gets_their_entries_back_as_stored(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let alice = token(&app, "alice").await;
    let bob = token(&app, "bob").await;

    // No entry before there is a way to unlock the vault.
    let uri = format!("/api/personal/entries/{ENTRY}");
    let entry = json!({ "nonce": NONCE, "ciphertext": CIPHERTEXT });
    let early = call(&app, &bob, "PUT", &uri, Some(entry.clone())).await;
    assert_eq!(early.json()["params"]["field"], "vault");

    set_up(&app, &bob).await;
    assert_eq!(
        call(&app, &bob, "PUT", &uri, Some(entry.clone()))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    let vault = send(&app, get("/api/personal/vault", Some(&bob)))
        .await
        .json();
    assert_eq!(vault["scheme"], "e2e_user_v1");
    assert_eq!(
        vault["entries"],
        json!([{ "id": ENTRY, "nonce": NONCE, "ciphertext": CIPHERTEXT }])
    );
    assert_eq!(vault["unlocks"].as_array().unwrap().len(), 2);
    assert_eq!(vault["unlocks"][0]["wrapped_key"], WRAPPED);

    // alice, administrator or not, sees nothing of it and cannot touch it.
    let hers = send(&app, get("/api/personal/vault", Some(&alice)))
        .await
        .json();
    assert_eq!(hers["entries"], json!([]));
    assert_eq!(hers["unlocks"], json!([]));
    set_up(&app, &alice).await;
    // "overwritten by alice!"
    let other = json!({ "nonce": NONCE, "ciphertext": "b3ZlcndyaXR0ZW4gYnkgYWxpY2Uh" });
    assert_eq!(
        call(&app, &alice, "PUT", &uri, Some(other)).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, &alice, "DELETE", &uri, None).await.status,
        StatusCode::NOT_FOUND
    );
    let vault = send(&app, get("/api/personal/vault", Some(&bob)))
        .await
        .json();
    assert_eq!(vault["entries"][0]["ciphertext"], CIPHERTEXT);

    // The database holds exactly the bytes the browser sent.
    let stored: Vec<u8> = sqlx::query_scalar("SELECT ciphertext FROM personal_entries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, b"sealed by the browser");

    assert_eq!(
        call(&app, &bob, "DELETE", &uri, None).await.status,
        StatusCode::NO_CONTENT
    );
    let log = send(&app, get("/api/audit", Some(&alice)))
        .await
        .json()
        .to_string();
    for action in [
        "personal.unlock_added",
        "personal.entry_saved",
        "personal.entry_deleted",
    ] {
        assert!(log.contains(action), "{action}");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_last_way_to_unlock_stays(pool: PgPool) {
    let app = app(state(pool), None);
    let bob = token(&app, "bob").await;
    set_up(&app, &bob).await;
    // A new passphrase replaces the old one.
    set_up(&app, &bob).await;
    let vault = send(&app, get("/api/personal/vault", Some(&bob)))
        .await
        .json();
    let ids: Vec<String> = vault["unlocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids.len(), 2);
    let remove = |id: &str| format!("/api/personal/unlocks/{id}");
    assert_eq!(
        call(&app, &bob, "DELETE", &remove(&ids[0]), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, &bob, "DELETE", &remove(&ids[1]), None)
            .await
            .code(),
        "last_unlock"
    );
    // Starting over removes everything.
    assert_eq!(
        call(&app, &bob, "DELETE", "/api/personal/vault", None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    let vault = send(&app, get("/api/personal/vault", Some(&bob)))
        .await
        .json();
    assert_eq!(vault["unlocks"], json!([]));
}

#[sqlx::test(migrations = "../../migrations")]
async fn only_well_formed_blobs_are_stored(pool: PgPool) {
    let app = app(state(pool), None);
    let bob = token(&app, "bob").await;
    set_up(&app, &bob).await;
    let uri = format!("/api/personal/entries/{ENTRY}");
    for (body, field) in [
        (
            json!({ "nonce": "AAEC", "ciphertext": CIPHERTEXT }),
            "nonce",
        ),
        (
            json!({ "nonce": NONCE, "ciphertext": "AAEC" }),
            "ciphertext",
        ),
    ] {
        assert_eq!(
            call(&app, &bob, "PUT", &uri, Some(body)).await.json()["params"]["field"],
            field
        );
    }
    for (body, field) in [
        (
            json!({ "kind": "sms", "params": {}, "wrapped_key": WRAPPED }),
            "kind",
        ),
        (
            json!({ "kind": "passkey", "params": [], "wrapped_key": WRAPPED }),
            "params",
        ),
        (
            json!({ "kind": "passkey", "params": {}, "wrapped_key": "AAEC" }),
            "wrapped_key",
        ),
    ] {
        let response = call(&app, &bob, "POST", "/api/personal/unlocks", Some(body)).await;
        assert_eq!(response.json()["params"]["field"], field);
    }
    assert_eq!(
        send(&app, get("/api/personal/vault", None)).await.status,
        StatusCode::UNAUTHORIZED
    );
}
