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
