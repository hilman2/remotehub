//! The browser extension (#201): connecting through a signed-in browser
//! with a code and PKCE, a token that opens only the extension's routes,
//! and passwords only for logins the user may reveal, only for their pages,
//! each audited.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::common::{
    BOB_SID, EXTENSION_ID, Response, authed, collection, get, send, sign_in_request,
};
use crate::terminal::{create, setup};

/// The extension's own origin, which its requests carry.
fn extension_origin() -> String {
    format!("chrome-extension://{EXTENSION_ID}")
}

/// A request as the extension sends it: bearer token, its version and
/// origin, no cookie.
fn from_extension(method: &str, uri: &str, body: Option<Value>, token: &str) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header("x-remotehub-extension", env!("CARGO_PKG_VERSION"))
        .header(header::ORIGIN, extension_origin());
    if body.is_some() {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    request
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap()
}

async fn sign_in(app: &Router, user: &str) -> String {
    send(app, sign_in_request(user, "right"))
        .await
        .session_token()
        .unwrap()
}

/// A verifier and its S256 challenge (RFC 7636).
fn pkce(seed: &str) -> (String, String) {
    let verifier = format!("{seed:-<43}");
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

async fn ask_code(app: &Router, browser: &str, extension_id: &str, challenge: &str) -> Response {
    send(
        app,
        authed(
            "POST",
            "/api/extension-codes",
            Some(json!({ "extension_id": extension_id, "challenge": challenge, "name": "Edge on Windows" })),
            browser,
        ),
    )
    .await
}

async fn trade(app: &Router, code: &str, verifier: &str) -> Response {
    let request = Request::post("/api/extension/token")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-remotehub-extension", env!("CARGO_PKG_VERSION"))
        .header(header::ORIGIN, extension_origin())
        .body(Body::from(
            json!({ "code": code, "verifier": verifier }).to_string(),
        ))
        .unwrap();
    send(app, request).await
}

/// Connects the extension for the user of the browser session `browser`
/// and returns its token.
async fn connect(app: &Router, browser: &str) -> String {
    let (verifier, challenge) = pkce("verifier");
    let code = ask_code(app, browser, EXTENSION_ID, &challenge).await;
    assert_eq!(code.status, StatusCode::CREATED, "{}", code.json());
    let traded = trade(app, code.json()["code"].as_str().unwrap(), &verifier).await;
    assert_eq!(traded.status, StatusCode::OK, "{}", traded.json());
    traded.json()["token"].as_str().unwrap().to_owned()
}

async fn audit(pool: &PgPool, action: &str) -> Vec<Value> {
    sqlx::query_scalar("SELECT details FROM audit_log WHERE action = $1 ORDER BY seq")
        .bind(action)
        .fetch_all(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_code_is_traded_once_and_only_with_its_verifier(pool: PgPool) {
    let (_, app, alice, _) = setup(pool.clone()).await;
    let (verifier, challenge) = pkce("right");

    // Only an extension remotehub knows gets a code: the redirect names it.
    let unknown = ask_code(&app, &alice, "ponmlkjihgfedcbaponmlkjihgfedcba", &challenge).await;
    assert_eq!(unknown.code(), "extension_unknown");
    let plain = ask_code(
        &app,
        &alice,
        EXTENSION_ID,
        "plain-verifier-instead-of-a-hash",
    )
    .await;
    assert_eq!(plain.json()["params"]["field"], "challenge");
    let foreign = send(
        &app,
        Request::post("/api/extension-codes")
            .header(header::COOKIE, format!("__Host-remotehub-session={alice}"))
            .header(header::ORIGIN, "https://evil.example")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "extension_id": EXTENSION_ID, "challenge": challenge, "name": "x" })
                    .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(foreign.code(), "forbidden_origin");

    // A wrong verifier uses the code up: the right one comes too late.
    let code = ask_code(&app, &alice, EXTENSION_ID, &challenge)
        .await
        .json()["code"]
        .as_str()
        .unwrap()
        .to_owned();
    let (wrong, _) = pkce("wrong");
    assert_eq!(
        trade(&app, &code, &wrong).await.json()["params"]["field"],
        "verifier"
    );
    assert_eq!(
        trade(&app, &code, &verifier).await.json()["params"]["field"],
        "code"
    );

    // A code waits a minute at most.
    let code = ask_code(&app, &alice, EXTENSION_ID, &challenge)
        .await
        .json()["code"]
        .as_str()
        .unwrap()
        .to_owned();
    sqlx::query("UPDATE extension_codes SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        trade(&app, &code, &verifier).await.json()["params"]["field"],
        "code"
    );

    // The right verifier in time, from the extension's own origin.
    let code = ask_code(&app, &alice, EXTENSION_ID, &challenge)
        .await
        .json()["code"]
        .as_str()
        .unwrap()
        .to_owned();
    let traded = trade(&app, &code, &verifier).await;
    assert_eq!(traded.status, StatusCode::OK, "{}", traded.json());
    assert_eq!(traded.json()["username"], "alice");
    assert_eq!(traded.json()["idle_seconds"], 30 * 60);
    assert_eq!(traded.headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        trade(&app, &code, &verifier).await.json()["params"]["field"],
        "code"
    );
    assert_eq!(
        audit(&pool, "extension.connected").await,
        [json!({ "name": "Edge on Windows", "extension_id": EXTENSION_ID })]
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_token_opens_the_extension_routes_and_nothing_else(pool: PgPool) {
    let (_, app, alice, _) = setup(pool).await;
    let token = connect(&app, &alice).await;
    let entries = |token: String| {
        let app = app.clone();
        async move {
            send(
                &app,
                from_extension("GET", "/api/extension/entries", None, &token),
            )
            .await
        }
    };
    assert_eq!(entries(token.clone()).await.status, StatusCode::OK);

    // As a cookie, the extension's token is no browser session.
    assert_eq!(
        send(&app, get("/api/tree", Some(&token))).await.code(),
        "unauthenticated"
    );
    assert_eq!(
        send(
            &app,
            authed("POST", "/api/extension-codes", Some(json!({})), &token)
        )
        .await
        .code(),
        "unauthenticated"
    );
    // A browser session opens none of the extension's routes, neither as
    // bearer token nor as cookie.
    assert_eq!(entries(alice.clone()).await.code(), "unauthenticated");
    assert_eq!(
        send(&app, get("/api/extension/entries", Some(&alice)))
            .await
            .code(),
        "extension_outdated"
    );
    let mut cookie = from_extension("GET", "/api/extension/entries", None, "");
    cookie.headers_mut().remove(header::AUTHORIZATION);
    cookie.headers_mut().insert(
        header::COOKIE,
        format!("__Host-remotehub-session={alice}").parse().unwrap(),
    );
    assert_eq!(send(&app, cookie).await.code(), "unauthenticated");

    // An extension older than the server answers asks for its update.
    let mut old = from_extension("GET", "/api/extension/entries", None, &token);
    old.headers_mut()
        .insert("x-remotehub-extension", "0.0.1".parse().unwrap());
    let old = send(&app, old).await;
    assert_eq!(old.status, StatusCode::UPGRADE_REQUIRED);
    assert_eq!(old.code(), "extension_outdated");
}

/// Alice's collection with logins, and bob's extension: he may reveal
/// `Portal` and `Code`, only see `Listed`, and not see `Hidden` at all.
struct Logins {
    app: Router,
    alice: String,
    bob: String,
    portal: String,
    listed: String,
    hidden: String,
    code: String,
}

async fn logins(pool: PgPool) -> Logins {
    let (_, app, alice, _) = setup(pool).await;
    let shared = collection(&app, &alice, None, "Shared").await;
    let login = |name: &'static str, extra: Value| {
        let (app, alice, shared) = (app.clone(), alice.clone(), shared.clone());
        async move {
            let mut body = json!({
                "collection_id": shared, "name": name, "username": format!("{name}-user"),
                "password": format!("{name}-Passw0rd!"),
            });
            body.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            create(&app, &alice, "/api/credentials", body).await
        }
    };
    let portal = login(
        "Portal",
        json!({ "url": "https://portal.example.com/sign-in" }),
    )
    .await;
    let listed = login("Listed", json!({ "url": "https://listed.example.com" })).await;
    let hidden = login("Hidden", json!({ "url": "https://hidden.example.com" })).await;
    let code = login(
        "Code",
        json!({ "url": "https://code.example.com", "totp": "JBSWY3DPEHPK3PXP" }),
    )
    .await;
    login("Without URL", json!({})).await;
    let binned = login("Binned", json!({ "url": "https://binned.example.com" })).await;
    let deleted = send(
        &app,
        authed(
            "DELETE",
            &format!("/api/credentials/{binned}"),
            None,
            &alice,
        ),
    )
    .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.json());
    for (id, role) in [
        (&portal, "reveal"),
        (&code, "reveal"),
        (&binned, "reveal"),
        (&listed, "list"),
    ] {
        let response = send(
            &app,
            authed(
                "POST",
                "/api/grants",
                Some(json!({
                    "object": { "kind": "credential", "id": id },
                    "principal_kind": "user", "principal_sid": BOB_SID,
                    "principal_name": "bob", "role": role,
                })),
                &alice,
            ),
        )
        .await;
        assert_eq!(
            response.status,
            StatusCode::NO_CONTENT,
            "{}",
            response.json()
        );
    }
    let bob = connect(&app, &sign_in(&app, "bob").await).await;
    Logins {
        app,
        alice,
        bob,
        portal,
        listed,
        hidden,
        code,
    }
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_extension_lists_the_logins_the_user_may_reveal_that_name_a_page(pool: PgPool) {
    let l = logins(pool).await;
    let listed = send(
        &l.app,
        from_extension("GET", "/api/extension/entries", None, &l.bob),
    )
    .await;
    assert_eq!(listed.headers[header::CACHE_CONTROL], "no-store");
    let names: Vec<String> = listed
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["Code", "Portal"]);
    let text = listed.json().to_string();
    assert!(!text.contains("Passw0rd"), "{text}");
    let portal = &listed.json()[1];
    assert_eq!(
        (&portal["username"], &portal["url"], &portal["has_totp"]),
        (
            &json!("Portal-user"),
            &json!("https://portal.example.com/sign-in"),
            &json!(false)
        )
    );

    // Alice may reveal everything and sees all logins with a URL outside
    // the bin; the extension's view of her own logins needs her own token.
    let alice = connect(&l.app, &l.alice).await;
    let all = send(
        &l.app,
        from_extension("GET", "/api/extension/entries", None, &alice),
    )
    .await;
    assert_eq!(all.json().as_array().unwrap().len(), 4, "{}", all.json());
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_password_is_filled_only_into_the_logins_pages_and_audited(pool: PgPool) {
    let l = logins(pool.clone()).await;
    let fill = |id: &str, origin: &str| {
        let request = from_extension(
            "POST",
            &format!("/api/extension/entries/{id}/fill"),
            Some(json!({ "origin": origin })),
            &l.bob,
        );
        let app = l.app.clone();
        async move { send(&app, request).await }
    };

    let filled = fill(&l.portal, "https://login.example.com").await;
    assert_eq!(filled.status, StatusCode::OK, "{}", filled.json());
    assert_eq!(
        filled.json(),
        json!({ "username": "Portal-user", "password": "Portal-Passw0rd!" })
    );
    assert_eq!(filled.headers[header::CACHE_CONTROL], "no-store");

    // A page of another site gets nothing, and nothing is revealed.
    for origin in [
        "https://example.com.evil.net",
        "http://portal.example.com",
        "null",
    ] {
        assert_eq!(
            fill(&l.portal, origin).await.code(),
            "wrong_site",
            "{origin}"
        );
    }
    assert_eq!(
        fill(&l.listed, "https://listed.example.com").await.code(),
        "forbidden"
    );
    assert_eq!(
        fill(&l.hidden, "https://hidden.example.com").await.code(),
        "not_found"
    );
    let binned: String =
        sqlx::query_scalar("SELECT id::text FROM credentials WHERE name = 'Binned'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        fill(&binned, "https://binned.example.com").await.code(),
        "not_found"
    );

    let copied = send(
        &l.app,
        from_extension(
            "POST",
            &format!("/api/extension/entries/{}/copy", l.portal),
            None,
            &l.bob,
        ),
    )
    .await;
    assert_eq!(copied.json(), json!({ "password": "Portal-Passw0rd!" }));

    assert_eq!(
        audit(&pool, "credential.revealed").await,
        [
            json!({ "purpose": "fill", "version": 1, "origin": "https://login.example.com", "client": "extension" }),
            json!({ "purpose": "copy", "version": 1, "client": "extension" }),
        ]
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_one_time_code_is_filled_into_the_logins_pages_or_copied(pool: PgPool) {
    let l = logins(pool.clone()).await;
    let code = |id: &str, body: Value| {
        let request = from_extension(
            "POST",
            &format!("/api/extension/entries/{id}/code"),
            Some(body),
            &l.bob,
        );
        let app = l.app.clone();
        async move { send(&app, request).await }
    };
    let filled = code(
        &l.code,
        json!({ "purpose": "fill", "origin": "https://code.example.com" }),
    )
    .await;
    assert_eq!(filled.status, StatusCode::OK, "{}", filled.json());
    let digits = filled.json()["code"].as_str().unwrap().to_owned();
    assert!(
        digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_digit()),
        "{digits}"
    );

    assert_eq!(
        code(
            &l.code,
            json!({ "purpose": "fill", "origin": "https://evil.example" })
        )
        .await
        .code(),
        "wrong_site"
    );
    assert_eq!(
        code(&l.code, json!({ "purpose": "fill" })).await.json()["params"]["field"],
        "origin"
    );
    assert_eq!(
        code(&l.code, json!({ "purpose": "show" })).await.json()["params"]["field"],
        "purpose"
    );
    assert_eq!(
        code(&l.code, json!({ "purpose": "copy" })).await.status,
        StatusCode::OK
    );
    assert_eq!(
        code(&l.portal, json!({ "purpose": "copy" })).await.code(),
        "not_found"
    );

    assert_eq!(
        audit(&pool, "credential.code_shown").await,
        [
            json!({ "purpose": "fill", "origin": "https://code.example.com", "client": "extension" }),
            json!({ "purpose": "copy", "client": "extension" }),
        ]
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_extension_ends_like_a_session_and_on_my_account(pool: PgPool) {
    let (_, app, alice, _) = setup(pool.clone()).await;
    let works = |token: String| {
        let app = app.clone();
        async move {
            send(
                &app,
                from_extension("GET", "/api/extension/entries", None, &token),
            )
            .await
            .status
                == StatusCode::OK
        }
    };

    // *My account* lists it and ends it; bob cannot end alice's.
    let first = connect(&app, &alice).await;
    let listed = send(&app, get("/api/account/extensions", Some(&alice)))
        .await
        .json();
    assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed[0]["name"], "Edge on Windows");
    let id = listed[0]["id"].as_str().unwrap().to_owned();
    let bob = sign_in(&app, "bob").await;
    let uri = format!("/api/account/extensions/{id}");
    assert_eq!(
        send(&app, authed("DELETE", &uri, None, &bob)).await.code(),
        "not_found"
    );
    assert!(works(first.clone()).await);
    let ended = send(&app, authed("DELETE", &uri, None, &alice)).await;
    assert_eq!(ended.status, StatusCode::NO_CONTENT);
    assert!(!works(first).await);

    // The extension signs out itself.
    let second = connect(&app, &alice).await;
    let out = send(
        &app,
        from_extension("DELETE", "/api/extension/session", None, &second),
    )
    .await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert!(!works(second).await);

    // After the idle time, as a browser session.
    let third = connect(&app, &alice).await;
    sqlx::query("UPDATE sessions SET last_seen_at = now() - interval '31 minutes' WHERE client = 'extension'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!works(third).await);

    // When an administrator ends the user's sessions.
    let fourth = connect(&app, &bob).await;
    let bob_id: String = sqlx::query_scalar("SELECT id::text FROM users WHERE username = 'bob'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let all = send(
        &app,
        authed(
            "DELETE",
            &format!("/api/users/{bob_id}/sessions"),
            None,
            &alice,
        ),
    )
    .await;
    assert_eq!(all.status, StatusCode::NO_CONTENT, "{}", all.json());
    assert!(!works(fourth).await);

    assert_eq!(
        audit(&pool, "extension.disconnected").await,
        [
            json!({ "by": "account", "name": "Edge on Windows" }),
            json!({ "by": "extension" }),
        ]
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_personal_vault_reaches_the_extension_sealed(pool: PgPool) {
    let (_, app, alice, _) = setup(pool).await;
    let token = connect(&app, &alice).await;
    let from_browser = send(&app, get("/api/personal/vault", Some(&alice))).await;
    let from_extension = send(
        &app,
        from_extension("GET", "/api/extension/personal", None, &token),
    )
    .await;
    assert_eq!(from_extension.status, StatusCode::OK);
    assert_eq!(from_extension.json(), from_browser.json());
    assert_eq!(from_extension.json()["scheme"], "e2e_user_v1");
}
