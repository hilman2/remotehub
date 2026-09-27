//! The site connector for Windows, served by remotehub (#188): without
//! sign-in, and before setup, since the customer's server has no session.

use std::sync::Arc;

use axum::http::{StatusCode, header};
use remotehub_server::downloads::{Downloads, PROGRAM};
use remotehub_server::{AppState, app};
use sqlx::PgPool;

use crate::common::{FakeDirectory, get, send, settings, vault};

#[sqlx::test(migrations = "../../migrations")]
async fn the_program_and_its_hash_are_served_to_anyone(pool: PgPool) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(PROGRAM), b"MZ program").unwrap();
    let mut with = settings();
    with.downloads = Some(Arc::new(Downloads::load(dir.path()).unwrap().unwrap()));
    // No setup done: the wizard would hold every API request back.
    let served = app(
        AppState::new(pool.clone(), Some(Arc::new(FakeDirectory)), with, vault()),
        None,
    );

    let program = send(&served, get("/downloads/remotehub-connector.exe", None)).await;
    assert_eq!(program.status, StatusCode::OK);
    assert_eq!(program.body, b"MZ program");
    assert!(
        program.headers[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );
    let sums = send(&served, get("/downloads/SHA256SUMS", None)).await;
    assert_eq!(sums.status, StatusCode::OK);
    let text = String::from_utf8(sums.body).unwrap();
    assert!(text.ends_with("  remotehub-connector.exe\n"), "{text}");
    for other in ["/downloads/other.exe", "/downloads/..%2Fca.crt"] {
        assert_eq!(
            send(&served, get(other, None)).await.status,
            StatusCode::NOT_FOUND,
            "{other}"
        );
    }

    // Without a program, there is nothing to serve.
    let bare = app(
        AppState::new(pool, Some(Arc::new(FakeDirectory)), settings(), vault()),
        None,
    );
    assert_eq!(
        send(&bare, get("/downloads/remotehub-connector.exe", None))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_extension_is_served_for_hand_and_policy_and_may_connect(pool: PgPool) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("remotehub-extension.zip"), b"PK zip").unwrap();
    std::fs::write(dir.path().join("remotehub-extension.crx"), b"Cr24 crx").unwrap();
    std::fs::write(
        dir.path().join("remotehub-extension.json"),
        r#"{"id": "ponmlkjihgfedcbaponmlkjihgfedcba", "version": "0.3.0"}"#,
    )
    .unwrap();
    let mut with = settings();
    with.extension_ids.clear();
    with.downloads = Some(Arc::new(Downloads::load(dir.path()).unwrap().unwrap()));
    // The served extension may connect, without being named in the settings.
    assert!(with.allows_extension("ponmlkjihgfedcbaponmlkjihgfedcba"));
    assert!(!with.allows_extension("abcdefghijklmnopabcdefghijklmnop"));
    let served = app(
        AppState::new(pool, Some(Arc::new(FakeDirectory)), with, vault()),
        None,
    );

    let zip = send(&served, get("/downloads/remotehub-extension.zip", None)).await;
    assert_eq!(zip.status, StatusCode::OK);
    assert_eq!(zip.body, b"PK zip");
    assert_eq!(zip.headers[header::CONTENT_TYPE], "application/zip");
    let crx = send(&served, get("/downloads/remotehub-extension.crx", None)).await;
    assert_eq!(
        crx.headers[header::CONTENT_TYPE],
        "application/x-chrome-extension"
    );
    let sums =
        String::from_utf8(send(&served, get("/downloads/SHA256SUMS", None)).await.body).unwrap();
    assert!(sums.contains("  remotehub-extension.zip\n"), "{sums}");
    assert!(sums.contains("  remotehub-extension.crx\n"), "{sums}");
    assert!(!sums.contains("json"), "{sums}");

    // The manifest the policy points to names the CRX under the public origin.
    let manifest = send(&served, get("/downloads/remotehub-extension.xml", None)).await;
    assert_eq!(manifest.status, StatusCode::OK);
    let xml = String::from_utf8(manifest.body).unwrap();
    assert!(
        xml.contains("appid='ponmlkjihgfedcbaponmlkjihgfedcba'"),
        "{xml}"
    );
    assert!(
        xml.contains(
            "codebase='https://remotehub.test/downloads/remotehub-extension.crx' version='0.3.0'"
        ),
        "{xml}"
    );
    assert_eq!(
        send(&served, get("/downloads/remotehub-extension.json", None))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}
