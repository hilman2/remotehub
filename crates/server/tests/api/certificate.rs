//! The certificate of Caddy of the ops package (#146): only administrators
//! change it, a certificate that does not fit the host never reaches Caddy,
//! and Caddy serves what is set, from its own CA or of your own. The lab's
//! Caddy (`REMOTEHUB_TEST_CADDY_SOCKET`) runs the ops package's Caddyfile for
//! remotehub.test and imports what the tests write to /run/caddy-sites.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use remotehub_server::caddy::{Caddy, CaddyConfig, Tls};
use remotehub_server::{AppState, app};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use zeroize::Zeroizing;

use crate::common::{self, FakeDirectory, authed, get, send, sign_in_request, state};

/// A self-signed certificate for `name`, valid from yesterday for 90 days,
/// and its key, as PEM.
fn made(name: &str) -> (String, String) {
    made_for(name, -1, 90)
}

/// As `made`, valid from `from` to `until` days from now.
fn made_for(name: &str, from: i64, until: i64) -> (String, String) {
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(vec![name.to_owned()]).unwrap();
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now + time::Duration::days(from);
    params.not_after = now + time::Duration::days(until);
    (params.self_signed(&key).unwrap().pem(), key.serialize_pem())
}

fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

async fn alice(app: &Router) -> String {
    send(app, sign_in_request("alice", "right"))
        .await
        .session_token()
        .unwrap()
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn without_caddy_there_is_nothing_to_set_but_a_misfit_is_refused(pool: PgPool) {
    let app = app(state(pool), None);
    let bob = send(&app, sign_in_request("bob", "right"))
        .await
        .session_token()
        .unwrap();
    for method in ["GET", "PUT", "DELETE"] {
        let body = (method == "PUT").then(|| json!({}));
        let response = send(
            &app,
            authed(method, "/api/settings/certificate", body, &bob),
        )
        .await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{method}");
    }

    let alice = alice(&app).await;
    let status = send(
        &app,
        authed("GET", "/api/settings/certificate", None, &alice),
    )
    .await;
    assert_eq!(
        status.json(),
        json!({ "host": "remotehub.test", "runs": false })
    );

    let (fitting, key) = made("remotehub.test");
    let (other, other_key) = made("other.example.com");
    let (expired, expired_key) = made_for("remotehub.test", -90, -1);
    for (certificate, key, reason) in [
        (&other, &other_key, "wrong_name"),
        (&fitting, &other_key, "key_mismatch"),
        (&expired, &expired_key, "expired"),
    ] {
        let misfit = send(
            &app,
            authed(
                "PUT",
                "/api/settings/certificate",
                Some(json!({ "certificate": certificate, "key": key })),
                &alice,
            ),
        )
        .await;
        assert_eq!(misfit.code(), "certificate_refused", "{reason}");
        assert_eq!(misfit.json()["params"]["reason"], reason);
    }

    let fits = send(
        &app,
        authed(
            "PUT",
            "/api/settings/certificate",
            Some(json!({ "certificate": fitting, "key": key })),
            &alice,
        ),
    )
    .await;
    assert_eq!(fits.code(), "caddy_unavailable");
    let reset = send(
        &app,
        authed("DELETE", "/api/settings/certificate", None, &alice),
    )
    .await;
    assert_eq!(reset.code(), "caddy_unavailable");

    // No CA of its own to hand out, for anyone.
    assert_eq!(
        send(&app, get("/ca.crt", None)).await.status,
        StatusCode::NOT_FOUND
    );
}

const SITES: &str = "/run/caddy-sites";

fn lab_caddy(caddyfile: PathBuf) -> CaddyConfig {
    CaddyConfig {
        socket: std::env::var("REMOTEHUB_TEST_CADDY_SOCKET")
            .expect("REMOTEHUB_TEST_CADDY_SOCKET names the test lab's Caddy")
            .into(),
        sites: SITES.into(),
        caddyfile,
        address: "caddy:443".to_owned(),
    }
}

fn ops_caddyfile() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/ops/caddy/Caddyfile")
}

/// The certificate status, once `ready` says it is what the test waits for.
async fn status_when(app: &Router, token: &str, ready: impl Fn(&Value) -> bool) -> Value {
    let mut status = Value::Null;
    for _ in 0..120 {
        status = send(app, authed("GET", "/api/settings/certificate", None, token))
            .await
            .json();
        if ready(&status) {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("the certificate status did not come: {status}");
}

/// Fails unless `issuer`'s key signed `certificate`.
fn signed_by(certificate: &[u8], issuer: &[u8]) {
    let (_, certificate) = x509_parser::parse_x509_certificate(certificate).unwrap();
    let (_, issuer) = x509_parser::parse_x509_certificate(issuer).unwrap();
    certificate
        .verify_signature(Some(issuer.public_key()))
        .unwrap_or_else(|e| {
            panic!(
                "{} not signed by {}: {e}",
                certificate.subject(),
                issuer.subject()
            )
        });
}

/// Stops the lab's Caddy through its admin API; its container starts it
/// again, which reads the Caddyfile and what it imports anew. Returns once
/// the old process is gone.
async fn restart_caddy() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let socket = std::env::var("REMOTEHUB_TEST_CADDY_SOCKET").unwrap();
    let mut stream = tokio::net::UnixStream::connect(&socket).await.unwrap();
    stream
        .write_all(b"POST /stop HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer).await;
    for _ in 0..200 {
        if tokio::net::UnixStream::connect(&socket).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the lab's Caddy did not stop");
}

fn sites_file(name: &str) -> Option<String> {
    std::fs::read_to_string(Path::new(SITES).join(name)).ok()
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
#[ignore = "needs the test lab"]
async fn caddy_serves_its_own_ca_a_certificate_of_your_own_and_back(pool: PgPool) {
    let mut settings = common::settings();
    let caddy = Arc::new(Caddy::new(lab_caddy(ops_caddyfile()), settings.host()));
    settings.caddy = Some(caddy.clone());
    let app = app(
        AppState::new(
            pool.clone(),
            Some(Arc::new(FakeDirectory)),
            settings,
            common::vault(),
        ),
        None,
    );
    let alice = alice(&app).await;

    // As at startup: the snippet first, which the lab's Caddy waits for.
    // Then automatic once Caddy answers, in case a run before left it
    // with a certificate of its own.
    caddy.prepare(&Tls::Automatic).unwrap();
    let mut answered = false;
    for _ in 0..60 {
        if caddy.serve(&Tls::Automatic).await.is_ok() {
            answered = true;
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert!(answered, "the lab's Caddy did not answer");
    let local = status_when(&app, &alice, |s| s["source"] == "remotehub").await;
    assert_eq!(local["runs"], true);
    assert!(local.get("own").is_none(), "{local}");
    let root = local["root_fingerprint"].as_str().unwrap().to_owned();

    // The root handed out is the one that signed what Caddy serves.
    let pem = send(&app, get("/ca.crt", None)).await;
    assert_eq!(pem.status, StatusCode::OK);
    let der = CertificateDer::from_pem_slice(&pem.body).unwrap();
    assert_eq!(fingerprint(&der), root);
    let cer = send(&app, get("/ca.cer", None)).await;
    assert_eq!(fingerprint(&cer.body), root);
    let served = caddy.current().await.unwrap();
    signed_by(&served[0], &served[1]);
    signed_by(&served[1], &der);

    let (certificate, key) = made("remotehub.test");
    let uploaded = send(
        &app,
        authed(
            "PUT",
            "/api/settings/certificate",
            Some(json!({ "certificate": certificate, "key": key })),
            &alice,
        ),
    )
    .await;
    assert_eq!(uploaded.status, StatusCode::OK, "{}", uploaded.json());
    let own = uploaded.json()["info"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    let serving = status_when(&app, &alice, |s| s["source"] == "own").await;
    assert_eq!(serving["served"]["fingerprint"], own.as_str());
    assert_eq!(serving["own"]["info"]["fingerprint"], own.as_str());

    // Its successor lives in the same files, so Caddy's configuration
    // stays the same: Caddy must load it all the same.
    let (successor, successor_key) = made("remotehub.test");
    let uploaded = send(
        &app,
        authed(
            "PUT",
            "/api/settings/certificate",
            Some(json!({ "certificate": successor, "key": successor_key })),
            &alice,
        ),
    )
    .await;
    assert_eq!(uploaded.status, StatusCode::OK, "{}", uploaded.json());
    let own = uploaded.json()["info"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    status_when(&app, &alice, |s| s["served"]["fingerprint"] == own.as_str()).await;
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'tls_certificate.uploaded'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audited, 2);

    // A restart of Caddy reads the files remotehub wrote.
    restart_caddy().await;
    status_when(&app, &alice, |s| s["served"]["fingerprint"] == own.as_str()).await;

    // Caddy refuses what does not load: its configuration and remotehub's
    // files stay as they were.
    let broken = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(broken.path(), "{\n\tno_such_option\n}\n").unwrap();
    let refusing = Caddy::new(lab_caddy(broken.path().to_owned()), "remotehub.test".into());
    let before = (sites_file("tls.caddy"), sites_file("certificate.pem"));
    let (other, other_key) = made("remotehub.test");
    let refused = refusing
        .serve(&Tls::Own {
            chain: other,
            key: Zeroizing::new(other_key),
        })
        .await;
    assert!(
        matches!(
            refused,
            Err(remotehub_server::caddy::CaddyError::Refused(_))
        ),
        "{refused:?}"
    );
    assert_eq!(
        (sites_file("tls.caddy"), sites_file("certificate.pem")),
        before
    );
    let still = status_when(&app, &alice, |s| s["source"].is_string()).await;
    assert_eq!(still["served"]["fingerprint"], own.as_str());

    let reset = send(
        &app,
        authed("DELETE", "/api/settings/certificate", None, &alice),
    )
    .await;
    assert_eq!(reset.status, StatusCode::NO_CONTENT);
    let back = status_when(&app, &alice, |s| s["source"] == "remotehub").await;
    assert!(back.get("own").is_none(), "{back}");
    assert_eq!(sites_file("key.pem"), None);
    let sealed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM secret_fields WHERE field = 'tls_key'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(sealed, 0);
}
