//! Break-glass accounts: CLI functions and the sign-in endpoint.

use crate::common::{ORIGIN, get, json, send, state, vault};
use axum::http::StatusCode;
use remotehub_server::break_glass::{self, BreakGlassError};
use remotehub_server::{AppState, app};
use secrecy::SecretString;
use serde_json::json;
use sqlx::PgPool;

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn code(secret: &str, at: u64) -> String {
    break_glass::code_at(secret, at).unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn password_and_totp_sign_in_once_per_code(pool: PgPool) {
    let vault = vault();
    let issued = break_glass::create(&pool, &vault, "emergency")
        .await
        .unwrap();
    let password = SecretString::from(issued.password.as_str().to_owned());
    let t = now();

    let sign_in = |code: String, at: u64| {
        let (pool, vault, password) = (&pool, &vault, &password);
        async move {
            break_glass::authenticate_at(pool, vault, "Emergency", password, &code, at)
                .await
                .unwrap()
        }
    };

    let account = sign_in(code(&issued.totp_secret, t), t).await.unwrap();
    assert_eq!(account.username, "emergency");
    // The same code (and any older one) does not work a second time.
    assert!(sign_in(code(&issued.totp_secret, t), t).await.is_none());
    assert!(
        sign_in(code(&issued.totp_secret, t - 30), t)
            .await
            .is_none()
    );
    // The next one does.
    assert!(
        sign_in(code(&issued.totp_secret, t + 30), t + 30)
            .await
            .is_some()
    );

    let wrong = SecretString::from("wrong".to_owned());
    let next = code(&issued.totp_secret, t + 60);
    assert!(
        break_glass::authenticate_at(&pool, &vault, "emergency", &wrong, &next, t + 60)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        break_glass::authenticate_at(&pool, &vault, "nobody", &password, &next, t + 60)
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn names_are_unique_and_reset_replaces_everything(pool: PgPool) {
    let vault = vault();
    let first = break_glass::create(&pool, &vault, "emergency")
        .await
        .unwrap();
    assert!(matches!(
        break_glass::create(&pool, &vault, "EMERGENCY").await,
        Err(BreakGlassError::Exists(_))
    ));

    let second = break_glass::reset(&pool, &vault, "emergency")
        .await
        .unwrap();
    assert_ne!(first.password.as_str(), second.password.as_str());
    assert_ne!(first.totp_secret.as_str(), second.totp_secret.as_str());
    let t = now();
    let old = SecretString::from(first.password.as_str().to_owned());
    let new = SecretString::from(second.password.as_str().to_owned());
    assert!(
        break_glass::authenticate_at(
            &pool,
            &vault,
            "emergency",
            &old,
            &code(&first.totp_secret, t),
            t
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(
        break_glass::authenticate_at(
            &pool,
            &vault,
            "emergency",
            &new,
            &code(&second.totp_secret, t),
            t
        )
        .await
        .unwrap()
        .is_some()
    );

    let names: Vec<String> = break_glass::list(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|a| a.username)
        .collect();
    assert_eq!(names, ["emergency"]);
    break_glass::delete(&pool, "emergency").await.unwrap();
    assert!(break_glass::list(&pool).await.unwrap().is_empty());
    assert!(matches!(
        break_glass::reset(&pool, &vault, "emergency").await,
        Err(BreakGlassError::Unknown(_))
    ));
    // The name is free again.
    break_glass::create(&pool, &vault, "emergency")
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_endpoint_signs_in_an_administrator_and_audits_it(pool: PgPool) {
    let state: AppState = state(pool.clone());
    let issued = break_glass::create(&pool, &state.vault, "emergency")
        .await
        .unwrap();
    let app = app(state, None);
    let body = |code: String| json!({ "username": "emergency", "password": issued.password.as_str(), "code": code });

    let wrong = send(
        &app,
        json(
            "POST",
            "/api/session/break-glass",
            body("000000".into()),
            Some(ORIGIN),
        ),
    )
    .await;
    assert_eq!(
        (wrong.status, wrong.code().as_str()),
        (StatusCode::UNAUTHORIZED, "invalid_credentials")
    );

    let response = send(
        &app,
        json(
            "POST",
            "/api/session/break-glass",
            body(code(&issued.totp_secret, now())),
            Some(ORIGIN),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.json(),
        json!({ "username": "emergency", "display_name": "emergency", "kind": "break_glass", "admin": true })
    );
    let token = response.session_token().unwrap();

    let log = send(&app, get("/api/audit", Some(&token))).await.json();
    assert_eq!(log[0]["action"], "session.sign_in");
    assert_eq!(
        log[0]["details"],
        json!({ "kind": "break_glass", "break_glass": true })
    );
    assert_eq!(log[1]["action"], "session.sign_in_failed");
    assert_eq!(log[1]["details"]["break_glass"], true);
    assert!(!log.to_string().contains(issued.password.as_str()));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_reset_ends_open_sessions(pool: PgPool) {
    let state: AppState = state(pool.clone());
    let issued = break_glass::create(&pool, &state.vault, "emergency")
        .await
        .unwrap();
    let app = app(state.clone(), None);
    let body = json!({
        "username": "emergency",
        "password": issued.password.as_str(),
        "code": code(&issued.totp_secret, now())
    });
    let token = send(
        &app,
        json("POST", "/api/session/break-glass", body, Some(ORIGIN)),
    )
    .await
    .session_token()
    .unwrap();
    assert_eq!(
        send(&app, get("/api/session", Some(&token))).await.status,
        StatusCode::OK
    );

    break_glass::reset(&pool, &state.vault, "emergency")
        .await
        .unwrap();
    assert_eq!(
        send(&app, get("/api/session", Some(&token))).await.status,
        StatusCode::UNAUTHORIZED
    );
}
