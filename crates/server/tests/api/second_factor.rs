//! A second factor for directory sign-ins (#107): alice administers, bob
//! and olaf (in RH Operators) are directory users.

use axum::Router;
use axum::http::StatusCode;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::pkcs8::EncodePublicKey;
use remotehub_server::app;
use remotehub_server::break_glass::code_at;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::common::{
    BOB_SID, OPS_SID, ORIGIN, Response, authed, get, json as request, send, state,
};

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// The app's code `offset` seconds from now.
fn code(secret: &str, offset: i64) -> String {
    code_at(secret, now().saturating_add_signed(offset)).unwrap()
}

async fn sign_in(app: &Router, user: &str, extra: Value) -> Response {
    let mut body = json!({ "username": user, "password": "right" });
    body.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    send(app, request("POST", "/api/session", body, Some(ORIGIN))).await
}

async fn token(app: &Router, user: &str, extra: Value) -> String {
    let response = sign_in(app, user, extra).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json());
    response.session_token().unwrap()
}

async fn call(app: &Router, token: &str, method: &str, uri: &str, body: Option<Value>) -> Response {
    send(app, authed(method, uri, body, token)).await
}

/// Sets up an app for the signed-in user; returns its secret.
async fn enroll(app: &Router, token: &str) -> String {
    let offer = call(app, token, "POST", "/api/account/second-factor/offer", None).await;
    assert_eq!(offer.status, StatusCode::OK, "{}", offer.json());
    let secret = offer.json()["secret"].as_str().unwrap().to_owned();
    assert!(offer.json()["uri"].as_str().unwrap().contains(&secret));
    // The previous step: the code of now stays for the next sign-in.
    let body = json!({ "secret": secret, "code": code(&secret, -30) });
    let response = call(app, token, "PUT", "/api/account/second-factor", Some(body)).await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{}",
        response.json()
    );
    secret
}

fn problem(response: &Response) -> (StatusCode, String) {
    (response.status, response.code())
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_user_with_an_app_signs_in_only_with_its_code(pool: PgPool) {
    let app = app(state(pool), None);
    let bob = token(&app, "bob", json!({})).await;
    let secret = enroll(&app, &bob).await;
    let status = call(&app, &bob, "GET", "/api/account/second-factor", None).await;
    assert_eq!(
        status.json(),
        json!({
            "available": true, "enrolled": true, "required": false, "app": true, "keys": [],
        })
    );

    let asked = sign_in(&app, "bob", json!({})).await;
    assert_eq!(
        problem(&asked),
        (StatusCode::FORBIDDEN, "second_factor_required".into())
    );
    assert!(asked.session_token().is_none());
    let wrong = sign_in(&app, "bob", json!({ "code": code(&secret, 300) })).await;
    assert_eq!(
        problem(&wrong),
        (StatusCode::UNAUTHORIZED, "second_factor_invalid".into())
    );
    assert!(wrong.session_token().is_none());

    let right = code(&secret, 0);
    token(&app, "bob", json!({ "code": right })).await;
    // Each code works once.
    let again = sign_in(&app, "bob", json!({ "code": right })).await;
    assert_eq!(
        problem(&again),
        (StatusCode::UNAUTHORIZED, "second_factor_invalid".into())
    );
    let alice = token(&app, "alice", json!({})).await;
    let log = call(&app, &alice, "GET", "/api/audit", None).await.json();
    let entries: Vec<(&str, &Value)> = log
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["actor_name"] == "bob")
        .map(|e| (e["action"].as_str().unwrap(), &e["details"]))
        .collect();
    assert_eq!(
        entries[0],
        (
            "session.sign_in_failed",
            &json!({ "reason": "second_factor_invalid" })
        )
    );
    assert_eq!(entries[1].0, "session.sign_in");
    assert_eq!(entries[1].1["second_factor"], true);
    assert!(
        entries
            .iter()
            .any(|(action, _)| *action == "second_factor.enrolled")
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn wrong_codes_count_as_failed_attempts(pool: PgPool) {
    let app = app(state(pool), None);
    let bob = token(&app, "bob", json!({})).await;
    let secret = enroll(&app, &bob).await;
    for _ in 0..5 {
        let wrong = sign_in(&app, "bob", json!({ "code": code(&secret, 300) })).await;
        assert_eq!(wrong.code(), "second_factor_invalid");
    }
    // The right password does not start the count anew.
    let blocked = sign_in(&app, "bob", json!({ "code": code(&secret, 0) })).await;
    assert_eq!(
        problem(&blocked),
        (StatusCode::TOO_MANY_REQUESTS, "too_many_attempts".into())
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_rule_asks_for_an_app_before_the_next_session(pool: PgPool) {
    let app = app(state(pool), None);
    let alice = token(&app, "alice", json!({})).await;
    let rule = json!({ "principal_kind": "group", "principal_name": "RH Operators" });
    let uri = format!("/api/second-factor-principals/{OPS_SID}");
    let response = call(&app, &alice, "PUT", &uri, Some(rule)).await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{}",
        response.json()
    );

    let offered = sign_in(&app, "olaf", json!({})).await;
    assert_eq!(
        problem(&offered),
        (StatusCode::FORBIDDEN, "second_factor_setup_required".into())
    );
    assert!(offered.session_token().is_none());
    let secret = offered.json()["params"]["secret"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        offered.json()["params"]["uri"]
            .as_str()
            .unwrap()
            .contains(&secret)
    );
    let wrong = sign_in(
        &app,
        "olaf",
        json!({ "totp_secret": secret, "code": code(&secret, 300) }),
    )
    .await;
    assert_eq!(
        problem(&wrong),
        (StatusCode::UNAUTHORIZED, "second_factor_invalid".into())
    );
    let olaf = token(
        &app,
        "olaf",
        json!({ "totp_secret": secret, "code": code(&secret, -30) }),
    )
    .await;

    // Set up, it is asked for, and the rule keeps it.
    let asked = sign_in(&app, "olaf", json!({})).await;
    assert_eq!(asked.code(), "second_factor_required");
    let status = call(&app, &olaf, "GET", "/api/account/second-factor", None).await;
    assert_eq!(status.json()["required"], true);
    let body = json!({ "code": code(&secret, 0) });
    let kept = call(
        &app,
        &olaf,
        "DELETE",
        "/api/account/second-factor",
        Some(body),
    )
    .await;
    assert_eq!(kept.status, StatusCode::FORBIDDEN);

    // An administrator removes it: the next sign-in sets up a new one.
    let users = call(&app, &alice, "GET", "/api/users", None).await.json();
    let id = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "olaf")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        users
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["username"] == "olaf" && u["second_factor"] == true)
    );
    assert!(
        users
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["username"] == "alice" && u["second_factor"] == false)
    );
    let uri = format!("/api/users/{id}/second-factor");
    assert_eq!(
        call(&app, &olaf, "DELETE", &uri, None).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, &alice, "DELETE", &uri, None).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, &alice, "DELETE", &uri, None).await.status,
        StatusCode::NOT_FOUND
    );
    let offered = sign_in(&app, "olaf", json!({})).await;
    assert_eq!(offered.code(), "second_factor_setup_required");

    // Without the rule, nobody is asked.
    let uri = format!("/api/second-factor-principals/{OPS_SID}");
    assert_eq!(
        call(&app, &alice, "DELETE", &uri, None).await.status,
        StatusCode::NO_CONTENT
    );
    token(&app, "olaf", json!({})).await;
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_rule_reaches_members_of_own_groups(pool: PgPool) {
    let app = app(state(pool), None);
    let alice = token(&app, "alice", json!({})).await;
    let group = call(
        &app,
        &alice,
        "POST",
        "/api/groups",
        Some(json!({ "name": "Contractors" })),
    )
    .await;
    let group = group.json()["id"].as_str().unwrap().to_owned();
    let member = json!({ "principal_kind": "user", "principal_name": "Bob" });
    call(
        &app,
        &alice,
        "PUT",
        &format!("/api/groups/{group}/members/{BOB_SID}"),
        Some(member),
    )
    .await;
    let rule = json!({ "principal_kind": "group", "principal_name": "Contractors" });
    let uri = format!("/api/second-factor-principals/group:{group}");
    let response = call(&app, &alice, "PUT", &uri, Some(rule)).await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{}",
        response.json()
    );
    assert_eq!(
        sign_in(&app, "bob", json!({})).await.code(),
        "second_factor_setup_required"
    );
    let rules = call(&app, &alice, "GET", "/api/second-factor-principals", None)
        .await
        .json();
    assert_eq!(rules[0]["principal_sid"], format!("group:{group}"));
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn removing_the_app_takes_one_of_its_codes(pool: PgPool) {
    let app = app(state(pool), None);
    let bob = token(&app, "bob", json!({})).await;
    let secret = enroll(&app, &bob).await;
    let wrong = json!({ "code": code(&secret, 300) });
    let refused = call(
        &app,
        &bob,
        "DELETE",
        "/api/account/second-factor",
        Some(wrong),
    )
    .await;
    assert_eq!(
        problem(&refused),
        (StatusCode::UNAUTHORIZED, "second_factor_invalid".into())
    );
    let right = json!({ "code": code(&secret, 0) });
    let removed = call(
        &app,
        &bob,
        "DELETE",
        "/api/account/second-factor",
        Some(right),
    )
    .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    token(&app, "bob", json!({})).await;
    let status = send(&app, get("/api/account/second-factor", Some(&bob))).await;
    assert_eq!(status.json()["enrolled"], false);
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_wrong_secret_or_code_sets_up_nothing(pool: PgPool) {
    let app = app(state(pool), None);
    let bob = token(&app, "bob", json!({})).await;
    for (secret, field) in [
        ("JBSWY3DPEHPK3PXP", "totp_secret"),
        ("not base32", "totp_secret"),
    ] {
        let body = json!({ "secret": secret, "code": "123456" });
        let response = call(&app, &bob, "PUT", "/api/account/second-factor", Some(body)).await;
        assert_eq!(
            (response.status, response.json()["params"]["field"].as_str()),
            (StatusCode::BAD_REQUEST, Some(field)),
            "{secret}"
        );
    }
    let offer = call(&app, &bob, "POST", "/api/account/second-factor/offer", None).await;
    let secret = offer.json()["secret"].as_str().unwrap().to_owned();
    let body = json!({ "secret": secret, "code": code(&secret, 300) });
    let response = call(&app, &bob, "PUT", "/api/account/second-factor", Some(body)).await;
    assert_eq!(response.code(), "second_factor_invalid");
    token(&app, "bob", json!({})).await;
}

/// A security key as a browser's WebAuthn would use it (#129), with the
/// relying party of the tests' origin.
struct Key {
    signing: SigningKey,
    id: Vec<u8>,
    counter: u32,
    counts: bool,
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

impl Key {
    fn new(seed: u8) -> Self {
        Key {
            signing: SigningKey::from_slice(&[seed; 32]).unwrap(),
            id: vec![seed; 20],
            counter: 0,
            counts: false,
        }
    }

    fn data(&self, flags: u8) -> Vec<u8> {
        let mut data = Sha256::digest(b"remotehub.test").to_vec();
        data.push(flags);
        data.extend(self.counter.to_be_bytes());
        data
    }

    fn client_data(kind: &str, challenge: &str) -> Vec<u8> {
        json!({ "type": kind, "challenge": challenge, "origin": ORIGIN })
            .to_string()
            .into_bytes()
    }

    /// The answer to `navigator.credentials.create()` with the server's options.
    fn register(&self, options: &Value) -> Value {
        let challenge = options["publicKey"]["challenge"].as_str().unwrap();
        let mut data = self.data(0x41);
        data.extend([0u8; 16]);
        data.extend((self.id.len() as u16).to_be_bytes());
        data.extend(&self.id);
        let spki = self.signing.verifying_key().to_public_key_der().unwrap();
        json!({
            "rawId": b64(&self.id),
            "response": {
                "clientDataJSON": b64(&Self::client_data("webauthn.create", challenge)),
                "authenticatorData": b64(&data),
                "publicKey": b64(spki.as_bytes()),
                "publicKeyAlgorithm": -7,
            },
        })
    }

    /// The answer to `navigator.credentials.get()`.
    fn sign(&mut self, options: &Value) -> Value {
        // Many passkeys keep no counter and always send 0: then only the
        // single use of the challenge stops a replay.
        if self.counts {
            self.counter += 1;
        }
        let challenge = options["publicKey"]["challenge"].as_str().unwrap();
        let data = self.data(0x01);
        let client = Self::client_data("webauthn.get", challenge);
        let signed = [data.as_slice(), &Sha256::digest(&client)].concat();
        let signature: Signature = self.signing.sign(&signed);
        json!({
            "rawId": b64(&self.id),
            "response": {
                "clientDataJSON": b64(&client),
                "authenticatorData": b64(&data),
                "signature": b64(signature.to_der().as_bytes()),
            },
        })
    }
}

/// Signs `user` in up to the second factor; returns the key's challenge.
async fn asked_for_key(app: &Router, user: &str) -> (Value, String) {
    let asked = sign_in(app, user, json!({})).await;
    assert_eq!(asked.code(), "second_factor_required");
    let params = asked.json()["params"].clone();
    let id = params["key_challenge_id"].as_str().unwrap().to_owned();
    (params, id)
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_security_key_signs_in_once_per_challenge(pool: PgPool) {
    let app = app(state(pool), None);
    let alice = token(&app, "alice", json!({})).await;
    let bob = token(&app, "bob", json!({})).await;
    let mut key = Key::new(3);

    let offer = call(&app, &bob, "POST", "/api/account/security-keys/offer", None).await;
    assert_eq!(offer.status, StatusCode::OK, "{}", offer.json());
    let offer = offer.json();
    assert_eq!(offer["options"]["publicKey"]["rp"]["id"], "remotehub.test");
    let body = json!({
        "challenge_id": offer["challenge_id"], "name": "Desk key",
        "credential": key.register(&offer["options"]),
    });
    let added = call(
        &app,
        &bob,
        "POST",
        "/api/account/security-keys",
        Some(body.clone()),
    )
    .await;
    assert_eq!(added.status, StatusCode::NO_CONTENT, "{}", added.json());
    // Its challenge is used up, for any key.
    let other = json!({
        "challenge_id": offer["challenge_id"], "name": "Spare key",
        "credential": Key::new(5).register(&offer["options"]),
    });
    let again = call(
        &app,
        &bob,
        "POST",
        "/api/account/security-keys",
        Some(other),
    )
    .await;
    assert_eq!(again.code(), "second_factor_invalid");
    let status = call(&app, &bob, "GET", "/api/account/second-factor", None)
        .await
        .json();
    assert_eq!(
        (status["enrolled"].clone(), status["app"].clone()),
        (json!(true), json!(false))
    );
    assert_eq!(status["keys"][0]["name"], "Desk key");

    // The sign-in asks for the key, and takes its signature once.
    let (params, challenge) = asked_for_key(&app, "bob").await;
    assert_eq!(params["app"], false);
    assert_eq!(
        params["key_options"]["publicKey"]["allowCredentials"][0]["id"],
        b64(&key.id)
    );
    let answer =
        json!({ "challenge_id": challenge, "credential": key.sign(&params["key_options"]) });
    token(&app, "bob", json!({ "security_key": answer.clone() })).await;
    let replayed = sign_in(&app, "bob", json!({ "security_key": answer })).await;
    assert_eq!(replayed.code(), "second_factor_invalid");

    // Another key, with the right credential ID, does not sign for bob.
    let (params, challenge) = asked_for_key(&app, "bob").await;
    let mut stranger = Key {
        id: key.id.clone(),
        ..Key::new(4)
    };
    let forged =
        json!({ "challenge_id": challenge, "credential": stranger.sign(&params["key_options"]) });
    let refused = sign_in(&app, "bob", json!({ "security_key": forged })).await;
    assert_eq!(refused.code(), "second_factor_invalid");

    // With a rule, the last factor stays; an administrator's reset takes it.
    let rule = json!({ "principal_kind": "user", "principal_name": "Bob Helpdesk" });
    let uri = format!("/api/second-factor-principals/{BOB_SID}");
    assert_eq!(
        call(&app, &alice, "PUT", &uri, Some(rule)).await.status,
        StatusCode::NO_CONTENT
    );
    let id = status["keys"][0]["id"].as_str().unwrap();
    let kept = call(
        &app,
        &bob,
        "DELETE",
        &format!("/api/account/security-keys/{id}"),
        None,
    )
    .await;
    assert_eq!(kept.status, StatusCode::FORBIDDEN);
    let users = call(&app, &alice, "GET", "/api/users", None).await.json();
    let bob_id = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "bob")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let reset = format!("/api/users/{bob_id}/second-factor");
    assert_eq!(
        call(&app, &alice, "DELETE", &reset, None).await.status,
        StatusCode::NO_CONTENT
    );
    let status = call(&app, &bob, "GET", "/api/account/second-factor", None)
        .await
        .json();
    assert_eq!(status["keys"], json!([]));
}
