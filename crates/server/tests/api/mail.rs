//! The mail server on the settings page (#145): only administrators set it,
//! a password never goes over an unencrypted connection and never comes
//! back, and what remotehub sends arrives, in the asked language. The lab's
//! Mailpit (`REMOTEHUB_TEST_SMTP_HOST`) keeps every mail.

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use remotehub_server::api::courier;
use remotehub_server::app;
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

use crate::accounts;
use crate::common::{authed, send, sign_in_request, state};

fn smtp_host() -> String {
    std::env::var("REMOTEHUB_TEST_SMTP_HOST")
        .expect("REMOTEHUB_TEST_SMTP_HOST names the test lab's Mailpit")
}

/// The lab's Mailpit: SMTP on 1025 without TLS.
fn lab(overrides: Value) -> Value {
    let mut settings = json!({
        "host": smtp_host(),
        "port": 1025,
        "security": "none",
        "from_address": "remotehub@remotehub.test",
        "from_name": "remotehub lab",
    });
    for (key, value) in overrides.as_object().unwrap() {
        settings[key] = value.clone();
    }
    settings
}

/// A recipient no other test and no other run uses.
fn recipient(who: &str) -> String {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{who}-{nonce}@remotehub.test")
}

/// GET on Mailpit's API, with nothing but the standard library and tokio.
async fn mailpit(path: &str) -> Value {
    let mut stream = tokio::net::TcpStream::connect((smtp_host(), 8025))
        .await
        .unwrap();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: mail\r\nConnection: close\r\nAccept: application/json\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.unwrap();
    let answer = String::from_utf8(answer).unwrap();
    let (_, body) = answer.split_once("\r\n\r\n").unwrap();
    serde_json::from_str(body).unwrap_or(Value::Null)
}

/// Subject and text of the mails to `to`, waiting a little for them.
async fn inbox(to: &str) -> Vec<(String, String)> {
    for _ in 0..50 {
        let found = mailpit(&format!("/api/v1/search?query=to:{to}")).await;
        let messages = found["messages"].as_array().cloned().unwrap_or_default();
        if !messages.is_empty() {
            let mut mails = Vec::new();
            for message in messages {
                let id = message["ID"].as_str().unwrap();
                let full = mailpit(&format!("/api/v1/message/{id}")).await;
                mails.push((
                    full["Subject"].as_str().unwrap_or_default().to_owned(),
                    full["Text"].as_str().unwrap_or_default().to_owned(),
                ));
            }
            return mails;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Vec::new()
}

fn field(response: &crate::common::Response) -> String {
    response.json()["params"]["field"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

async fn alice(app: &Router) -> String {
    send(app, sign_in_request("alice", "right"))
        .await
        .session_token()
        .unwrap()
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn only_administrators_set_the_mail_server_and_the_form_is_checked(pool: PgPool) {
    let app = app(state(pool), None);
    let bob = send(&app, sign_in_request("bob", "right"))
        .await
        .session_token()
        .unwrap();
    let settings =
        json!({ "host": "mail", "security": "starttls", "from_address": "a@example.com" });
    for (method, uri) in [
        ("GET", "/api/settings/mail"),
        ("PUT", "/api/settings/mail"),
        ("POST", "/api/settings/mail/test"),
        ("DELETE", "/api/settings/mail"),
    ] {
        let body = (method == "PUT" || method == "POST").then(|| settings.clone());
        let response = send(&app, authed(method, uri, body, &bob)).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{method} {uri}");
    }

    let alice = alice(&app).await;
    for (change, wrong) in [
        (json!({ "host": "" }), "host"),
        (json!({ "security": "ssl" }), "security"),
        (json!({ "port": 0 }), "port"),
        // A password never goes over an unencrypted connection.
        (
            json!({ "security": "none", "username": "u", "password": "p" }),
            "password",
        ),
        (json!({ "password": "p" }), "username"),
        (json!({ "from_address": "nobody" }), "from_address"),
        (json!({ "ca_pem": "x" }), "ca_pem"),
    ] {
        let mut body = settings.clone();
        for (key, value) in change.as_object().unwrap() {
            body[key] = value.clone();
        }
        let response = send(
            &app,
            authed("PUT", "/api/settings/mail", Some(body), &alice),
        )
        .await;
        assert_eq!(response.code(), "invalid_request", "{change}");
        assert_eq!(field(&response), wrong, "{change}");
    }
    let stored = send(&app, authed("GET", "/api/settings/mail", None, &alice)).await;
    assert_eq!(stored.json(), json!({ "server": null }));
    let none = send(&app, authed("DELETE", "/api/settings/mail", None, &alice)).await;
    assert_eq!(none.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_password_is_sealed_kept_with_its_user_and_never_shown(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let alice = alice(&app).await;
    let sealed = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM secret_fields WHERE field = 'smtp_password'",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let put = |body: Value| {
        send(
            &app,
            authed("PUT", "/api/settings/mail", Some(body), &alice),
        )
    };
    let server = json!({
        "host": "smtp.example.com", "security": "starttls", "username": "relay",
        "password": "Smtp-Passw0rd!", "from_address": "remotehub@example.com",
    });
    assert_eq!(put(server.clone()).await.status, StatusCode::NO_CONTENT);
    assert_eq!(sealed().await, 1);

    let stored = send(&app, authed("GET", "/api/settings/mail", None, &alice)).await;
    let text = String::from_utf8(stored.body.clone()).unwrap();
    assert!(
        !text.contains("Smtp-Passw0rd!") && !text.contains("password"),
        "{text}"
    );
    assert_eq!(stored.json()["server"]["port"], 587);
    assert_eq!(stored.json()["server"]["from_name"], "remotehub");

    // Without a new password, the stored one stays with its user.
    let mut again = server.clone();
    again["password"] = Value::Null;
    again["from_name"] = json!("IT");
    assert_eq!(put(again.clone()).await.status, StatusCode::NO_CONTENT);
    assert_eq!(sealed().await, 1);
    // Another user does not get the old password.
    again["username"] = json!("other");
    assert_eq!(put(again).await.status, StatusCode::NO_CONTENT);
    assert_eq!(sealed().await, 0);

    let fields: Vec<Value> = sqlx::query_scalar(
        "SELECT details -> 'fields' FROM audit_log WHERE action = 'mail.changed' ORDER BY seq",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(fields[1], json!(["from_name"]));
    assert_eq!(fields[2], json!(["username"]));

    let removed = send(&app, authed("DELETE", "/api/settings/mail", None, &alice)).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let stored = send(&app, authed("GET", "/api/settings/mail", None, &alice)).await;
    assert_eq!(stored.json(), json!({ "server": null }));
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
#[ignore = "needs the test lab"]
async fn a_test_mail_arrives_in_the_chosen_language_or_names_its_failure(pool: PgPool) {
    let app = app(state(pool), None);
    let alice = alice(&app).await;
    let to = recipient("test");
    let body = |overrides: Value, language: &str| {
        let mut body = lab(overrides);
        body["to"] = json!(to);
        body["language"] = json!(language);
        body
    };
    let sent = send(
        &app,
        authed(
            "POST",
            "/api/settings/mail/test",
            Some(body(json!({}), "de")),
            &alice,
        ),
    )
    .await;
    assert_eq!(sent.json(), json!({}), "no failure");
    let mails = inbox(&to).await;
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].0, "remotehub-Testmail");
    assert!(
        mails[0].1.contains("Der Mailserver funktioniert."),
        "{}",
        mails[0].1
    );

    let refused = send(
        &app,
        authed(
            "POST",
            "/api/settings/mail/test",
            Some(body(json!({ "port": 1 }), "en")),
            &alice,
        ),
    )
    .await;
    assert_eq!(
        refused.json()["failure"]["step"],
        "connect",
        "{}",
        refused.json()
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
#[ignore = "needs the test lab"]
async fn invitations_and_kratos_codes_go_out_by_mail(pool: PgPool) {
    // remotehub with the stand-in for Kratos, whose invitations all have the
    // code 123456, and the lab's Mailpit as its mail server.
    let (app, _) = accounts::setup(pool.clone()).await;
    let admin = send(&app, accounts::sign_in("admin"))
        .await
        .session_token()
        .unwrap();
    let saved = send(
        &app,
        authed("PUT", "/api/settings/mail", Some(lab(json!({}))), &admin),
    )
    .await;
    assert_eq!(saved.status, StatusCode::NO_CONTENT, "{}", saved.json());

    let invited = recipient("invited");
    let response = send(
        &app,
        authed(
            "POST",
            "/api/users/invite",
            Some(json!({ "email": invited, "name": "Ines", "send_mail": true, "language": "en" })),
            &admin,
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json());
    assert_eq!(response.json()["mailed"], true);
    let mails = inbox(&invited).await;
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].0, "An account on remotehub");
    assert!(mails[0].1.contains("Hello Ines"), "{}", mails[0].1);
    assert!(mails[0].1.contains("Code: 123456"));
    assert!(mails[0].1.contains("/sign-in/recovery?flow=f"));

    // Kratos' code for a forgotten password, through the courier's port:
    // only with the token, and in the language the browser asked in.
    let state = remotehub_server::AppState::new(
        pool.clone(),
        None,
        crate::common::settings(),
        crate::common::vault(),
    );
    let courier = courier::router(state, SecretString::from("courier-token"));
    let forgot = recipient("forgot");
    let delivery = json!({
        "recipient": forgot,
        "template_type": "recovery_code_valid",
        "template_data": { "recovery_code": "654321" },
        "request_headers": { "Accept-Language": ["de-DE,de;q=0.9"] },
    });
    let post = |token: &str| {
        Request::post("/courier")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(delivery.to_string()))
            .unwrap()
    };
    let refused = courier.clone().oneshot(post("wrong")).await.unwrap();
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    let delivered = courier.oneshot(post("courier-token")).await.unwrap();
    assert_eq!(delivered.status(), StatusCode::NO_CONTENT);
    let mails = inbox(&forgot).await;
    assert_eq!(mails.len(), 1, "only the mail with the token");
    assert_eq!(mails[0].0, "Code für remotehub");
    assert!(mails[0].1.contains("654321"));
}
