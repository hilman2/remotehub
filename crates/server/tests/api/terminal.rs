//! The terminal WebSocket end to end: a real server on a free port, a
//! WebSocket client like the browser, and (lab tests) the test lab's SSH
//! target.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use futures_util::{SinkExt, StreamExt};
use remotehub_gateway::ssh_ca::SshCa;
use remotehub_server::{AppState, app};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::common::{ORIGIN, authed, send, sign_in_request, state};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub async fn serve(state: AppState) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = app(state, None);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    address
}

async fn open(
    address: SocketAddr,
    device: &str,
    token: &str,
    origin: &str,
) -> Result<Socket, tungstenite::Error> {
    let mut request = format!("ws://{address}/api/devices/{device}/terminal")
        .into_client_request()
        .unwrap();
    let headers = request.headers_mut();
    headers.insert(
        "cookie",
        format!("__Host-remotehub-session={token}").parse().unwrap(),
    );
    headers.insert("origin", origin.parse().unwrap());
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(socket, _)| socket)
}

/// The next JSON event, skipping terminal output.
async fn event(socket: &mut Socket) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(20), socket.next())
            .await
            .expect("an event within 20 s")
            .expect("the socket is open")
            .unwrap();
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

/// The next event, which must be `connected`; anything else fails with the
/// whole event, error code included.
async fn expect_connected(socket: &mut Socket) -> Value {
    let event = event(socket).await;
    assert_eq!(event["type"], "connected", "{event}");
    event
}

/// Terminal output until `needle` shows up.
async fn output_until(socket: &mut Socket, needle: &str) -> String {
    let mut seen = String::new();
    while !seen.contains(needle) {
        let message = tokio::time::timeout(Duration::from_secs(20), socket.next())
            .await
            .unwrap_or_else(|_| panic!("{needle:?} not seen in: {seen}"))
            .unwrap()
            .unwrap();
        if let Message::Binary(data) = message {
            seen.push_str(&String::from_utf8_lossy(&data));
        }
    }
    seen
}

async fn start(socket: &mut Socket, extra: Value) {
    let mut start = json!({ "type": "start", "cols": 100, "rows": 30 });
    start
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    socket
        .send(Message::Text(start.to_string().into()))
        .await
        .unwrap();
}

pub async fn create(app: &Router, token: &str, uri: &str, body: Value) -> String {
    let response = send(app, authed("POST", uri, Some(body), token)).await;
    assert_eq!(response.status, 201, "{uri}: {}", response.json());
    response.json()["id"].as_str().unwrap().to_owned()
}

/// Alice's session and a folder for the devices.
pub async fn setup(pool: PgPool) -> (AppState, Router, String, String) {
    setup_with(state(pool)).await
}

async fn setup_with(state: AppState) -> (AppState, Router, String, String) {
    let app = app(state.clone(), None);
    let token = send(&app, sign_in_request("alice", "right"))
        .await
        .session_token()
        .unwrap();
    let folder = create(
        &app,
        &token,
        "/api/folders",
        json!({ "parent_id": null, "name": "Lab" }),
    )
    .await;
    (state, app, token, folder)
}

fn ssh_host() -> String {
    std::env::var("REMOTEHUB_TEST_SSH_HOST")
        .expect("REMOTEHUB_TEST_SSH_HOST points to the test lab")
}

async fn stored_device(
    app: &Router,
    token: &str,
    folder: &str,
    label: &str,
    password: &str,
) -> String {
    let credential = create(
        app,
        token,
        "/api/credentials",
        json!({ "folder_id": folder, "name": format!("tester ({label})"), "username": "tester", "password": password }),
    )
    .await;
    create(
        app,
        token,
        "/api/devices",
        json!({
            "folder_id": folder, "name": format!("target ({label})"), "protocol": "ssh", "host": ssh_host(),
            "port": 22, "auth_mode": "stored", "credential_id": credential,
        }),
    )
    .await
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_foreign_origin_or_no_session_cannot_open_a_terminal(pool: PgPool) {
    let (state, app, token, folder) = setup(pool).await;
    let device = create(
        &app,
        &token,
        "/api/devices",
        json!({
            "folder_id": folder, "name": "x", "protocol": "ssh", "host": "x.example.com",
            "port": 22, "auth_mode": "ask", "credential_id": null,
        }),
    )
    .await;
    let address = serve(state).await;

    // Cross-site WebSocket hijacking: another website opens the terminal with the user's cookie.
    match open(address, &device, &token, "https://evil.example").await {
        Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 403),
        other => panic!("expected 403, got {:?}", other.map(|_| ())),
    }
    match open(address, &device, "no-session", ORIGIN).await {
        Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 401),
        other => panic!("expected 401, got {:?}", other.map(|_| ())),
    }
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn a_stored_credential_opens_a_shell_and_pins_the_host_key(pool: PgPool) {
    let (state, app, token, folder) = setup(pool.clone()).await;
    let device = stored_device(&app, &token, &folder, "right", "Tester-Passw0rd!").await;
    let address = serve(state).await;

    let mut socket = open(address, &device, &token, ORIGIN).await.unwrap();
    start(&mut socket, json!({})).await;
    let connected = event(&mut socket).await;
    assert_eq!(connected["type"], "connected", "{connected}");
    assert_eq!(connected["pinned"], true);
    let fingerprint = connected["host_key_fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(fingerprint.starts_with("SHA256:"));

    socket
        .send(Message::Binary(
            b"echo \"I am $(whoami)\"\n".to_vec().into(),
        ))
        .await
        .unwrap();
    output_until(&mut socket, "I am tester").await;
    socket
        .send(Message::Binary(b"exit 0\n".to_vec().into()))
        .await
        .unwrap();
    assert_eq!(
        event(&mut socket).await,
        json!({ "type": "closed", "exit_status": 0 })
    );

    // The key is pinned; the next connection does not pin again.
    let tree = send(&app, crate::common::get("/api/tree", Some(&token)))
        .await
        .json();
    assert_eq!(tree["devices"][0]["host_key_fingerprint"], fingerprint);
    let mut again = open(address, &device, &token, ORIGIN).await.unwrap();
    start(&mut again, json!({})).await;
    assert_eq!(expect_connected(&mut again).await["pinned"], false);
    drop(again);

    tokio::time::sleep(Duration::from_millis(300)).await;
    let log = send(&app, crate::common::get("/api/audit", Some(&token)))
        .await
        .json();
    let actions: Vec<&str> = log
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    for action in [
        "connection.opened",
        "device.host_key_pinned",
        "connection.closed",
    ] {
        assert!(actions.contains(&action), "{action} missing in {actions:?}");
    }
    assert!(!log.to_string().contains("Tester-Passw0rd!"));
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn a_changed_host_key_stops_the_connection(pool: PgPool) {
    let (state, app, token, folder) = setup(pool.clone()).await;
    let device = stored_device(&app, &token, &folder, "right", "Tester-Passw0rd!").await;
    let other =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIErqoGI5zlU7vi5Y/fdFH/EJV35jU1dDC5j8WCWzOszR other";
    sqlx::query("UPDATE devices SET host_key = $1, host_key_pinned_at = now()")
        .bind(other)
        .execute(&pool)
        .await
        .unwrap();
    let address = serve(state).await;

    let mut socket = open(address, &device, &token, ORIGIN).await.unwrap();
    start(&mut socket, json!({})).await;
    let error = event(&mut socket).await;
    assert_eq!(error["code"], "host_key_changed", "{error}");
    assert!(
        error["params"]["expected"]
            .as_str()
            .unwrap()
            .starts_with("SHA256:")
    );

    // After forgetting the pin, the next connection pins the real key.
    let reset = send(
        &app,
        authed(
            "DELETE",
            &format!("/api/devices/{device}/host-key"),
            None,
            &token,
        ),
    )
    .await;
    assert_eq!(reset.status, 204);
    let mut socket = open(address, &device, &token, ORIGIN).await.unwrap();
    start(&mut socket, json!({})).await;
    assert_eq!(event(&mut socket).await["pinned"], true);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn asked_credentials_work_once_and_wrong_ones_are_reported(pool: PgPool) {
    let (state, app, token, folder) = setup(pool).await;
    let asking = create(
        &app,
        &token,
        "/api/devices",
        json!({
            "folder_id": folder, "name": "asking", "protocol": "ssh", "host": ssh_host(),
            "port": 22, "auth_mode": "ask", "credential_id": null,
        }),
    )
    .await;
    let wrong = stored_device(&app, &token, &folder, "wrong", "wrong").await;
    let address = serve(state).await;

    let mut socket = open(address, &asking, &token, ORIGIN).await.unwrap();
    start(&mut socket, json!({})).await;
    assert_eq!(event(&mut socket).await["code"], "invalid_request");

    let mut socket = open(address, &asking, &token, ORIGIN).await.unwrap();
    start(
        &mut socket,
        json!({ "username": "tester", "password": "Tester-Passw0rd!" }),
    )
    .await;
    expect_connected(&mut socket).await;

    let mut socket = open(address, &wrong, &token, ORIGIN).await.unwrap();
    start(&mut socket, json!({})).await;
    assert_eq!(event(&mut socket).await["code"], "target_auth_failed");
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn the_own_account_connects_with_the_sign_in_password(pool: PgPool) {
    let state = state(pool);
    let app = app(state.clone(), None);
    // "tester" exists in the fake directory with the SSH target's password.
    let sign_in = send(&app, sign_in_request("tester", "Tester-Passw0rd!")).await;
    let token = sign_in.session_token().unwrap();
    let key_cookie = sign_in
        .headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("__Host-remotehub-login-key="))
        .map(|v| v.split(';').next().unwrap().to_owned())
        .unwrap();
    let alice = send(&app, sign_in_request("alice", "right"))
        .await
        .session_token()
        .unwrap();
    let folder = create(
        &app,
        &alice,
        "/api/folders",
        json!({ "parent_id": null, "name": "Own" }),
    )
    .await;
    let device = create(
        &app,
        &alice,
        "/api/devices",
        json!({
            "folder_id": folder, "name": "own", "protocol": "ssh", "host": ssh_host(),
            "port": 22, "auth_mode": "own", "credential_id": null,
        }),
    )
    .await;
    let grant = send(
        &app,
        authed(
            "POST",
            "/api/grants",
            Some(json!({
                "object": { "kind": "device", "id": device },
                "principal_kind": "user", "principal_sid": "S-1-5-21-1-2-3-1108",
                "principal_name": "Tester", "role": "connect",
            })),
            &alice,
        ),
    )
    .await;
    assert_eq!(grant.status, 204);
    let address = serve(state).await;

    // Without the key cookie the password cannot be opened.
    let mut socket = open(address, &device, &token, ORIGIN).await.unwrap();
    start(&mut socket, json!({})).await;
    assert_eq!(event(&mut socket).await["code"], "own_account_unavailable");

    let mut request = format!("ws://{address}/api/devices/{device}/terminal")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "cookie",
        format!("__Host-remotehub-session={token}; {key_cookie}")
            .parse()
            .unwrap(),
    );
    request
        .headers_mut()
        .insert("origin", ORIGIN.parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    start(&mut socket, json!({})).await;
    expect_connected(&mut socket).await;
    socket
        .send(Message::Binary(b"whoami\n".to_vec().into()))
        .await
        .unwrap();
    output_until(&mut socket, "tester").await;
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn a_stored_key_with_certificate_opens_a_shell(pool: PgPool) {
    let (state, app, token, folder) = setup(pool).await;
    let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../deploy/testlab/ssh");
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
    let credential = create(
        &app,
        &token,
        "/api/credentials",
        json!({
            "folder_id": folder, "name": "certified", "kind": "ssh_key", "username": "tester",
            "private_key": read("tester_ed25519_cert"), "certificate": read("tester_ed25519_cert-cert.pub"),
        }),
    )
    .await;
    let device = create(
        &app,
        &token,
        "/api/devices",
        json!({
            "folder_id": folder, "name": "by certificate", "protocol": "ssh", "host": ssh_host(),
            "port": 22, "auth_mode": "stored", "credential_id": credential,
        }),
    )
    .await;
    let address = serve(state).await;

    let mut socket = open(address, &device, &token, ORIGIN).await.unwrap();
    start(&mut socket, json!({})).await;
    expect_connected(&mut socket).await;
    socket
        .send(Message::Binary(
            b"echo \"cert: $(whoami)\"\n".to_vec().into(),
        ))
        .await
        .unwrap();
    output_until(&mut socket, "cert: tester").await;
}

/// A server whose SSH CA is `ca`, with alice signed in, a folder, and a
/// device on the lab's SSH target that signs in with a certificate.
async fn certificate_device(
    pool: PgPool,
    ca: Option<SshCa>,
) -> (SocketAddr, Router, String, String) {
    let mut settings = crate::common::settings();
    settings.ssh_ca = ca.map(Arc::new);
    let state = AppState::new(
        pool,
        Some(Arc::new(crate::common::FakeDirectory)),
        settings,
        crate::common::vault(),
    );
    let (state, app, token, folder) = setup_with(state).await;
    let device = create(
        &app,
        &token,
        "/api/devices",
        json!({
            "folder_id": folder, "name": "as alice", "protocol": "ssh", "host": ssh_host(),
            "port": 22, "auth_mode": "certificate", "credential_id": null,
        }),
    )
    .await;
    (serve(state).await, app, token, device)
}

/// The lab's key for remotehub's CA (deploy/testlab/ssh/remotehub_ca).
fn lab_ca() -> SshCa {
    let key = std::fs::read_to_string(
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("../../deploy/testlab/ssh/remotehub_ca"),
    )
    .unwrap();
    SshCa::from_openssh(&key).unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn a_certificate_from_the_ca_signs_in_as_the_user(pool: PgPool) {
    let (address, _, token, device) = certificate_device(pool, Some(lab_ca())).await;
    let mut socket = open(address, &device, &token, ORIGIN).await.unwrap();
    start(&mut socket, json!({})).await;
    expect_connected(&mut socket).await;
    socket
        .send(Message::Binary(b"echo \"ca: $(whoami)\"\n".to_vec().into()))
        .await
        .unwrap();
    // alice has neither password nor key on the target.
    output_until(&mut socket, "ca: alice").await;
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn a_certificate_from_another_ca_is_refused(pool: PgPool) {
    let other = SshCa::from_openssh(&SshCa::generate()).unwrap();
    let (address, _, token, device) = certificate_device(pool, Some(other)).await;
    let mut socket = open(address, &device, &token, ORIGIN).await.unwrap();
    start(&mut socket, json!({})).await;
    assert_eq!(event(&mut socket).await["code"], "target_auth_failed");
}

#[sqlx::test(migrations = "../../migrations")]
async fn without_a_ca_there_is_no_certificate_and_no_public_key(pool: PgPool) {
    let (address, app, token, device) = certificate_device(pool, None).await;
    let mut socket = open(address, &device, &token, ORIGIN).await.unwrap();
    start(&mut socket, json!({})).await;
    assert_eq!(event(&mut socket).await["code"], "ssh_ca_unavailable");
    let missing = send(&app, crate::common::get("/api/ssh-ca.pub", None)).await;
    assert_eq!(missing.status, 404);
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_public_key_is_there_for_the_targets_without_signing_in(pool: PgPool) {
    let ca = lab_ca();
    let expected = format!("{}\n", ca.public_key());
    let (_, app, _, _) = certificate_device(pool, Some(ca)).await;
    let response = send(&app, crate::common::get("/api/ssh-ca.pub", None)).await;
    assert_eq!(response.status, 200);
    assert_eq!(String::from_utf8(response.body).unwrap(), expected);
    // The same key as the lab target trusts, apart from the comment.
    let lab = std::fs::read_to_string(
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("../../deploy/testlab/ssh/remotehub_ca.pub"),
    )
    .unwrap();
    let key_of = |line: &str| {
        line.split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert_eq!(key_of(&expected), key_of(&lab));
}

#[sqlx::test(migrations = "../../migrations")]
async fn only_ssh_devices_sign_in_with_a_certificate(pool: PgPool) {
    let (_, app, token, _) = certificate_device(pool, None).await;
    let tree = send(&app, crate::common::get("/api/tree", Some(&token)))
        .await
        .json();
    let folder = tree["folders"][0]["id"].as_str().unwrap().to_owned();
    let response = send(
        &app,
        authed(
            "POST",
            "/api/devices",
            Some(json!({
                "folder_id": folder, "name": "rdp", "protocol": "rdp", "host": "desktop",
                "port": 3389, "auth_mode": "certificate", "credential_id": null,
            })),
            &token,
        ),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["params"]["field"], "auth_mode");
}

async fn laps_device(app: &Router, token: &str, folder: &str, host: &str) -> String {
    create(
        app,
        token,
        "/api/devices",
        json!({
            "folder_id": folder, "name": format!("laps {host}"), "protocol": "ssh", "host": host,
            "port": 22, "auth_mode": "laps", "credential_id": null,
        }),
    )
    .await
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn the_laps_password_signs_in_as_the_local_account(pool: PgPool) {
    let (state, app, token, folder) = setup(pool).await;
    let device = laps_device(&app, &token, &folder, &ssh_host()).await;
    let address = serve(state).await;
    let mut socket = open(address, &device, &token, ORIGIN).await.unwrap();
    start(&mut socket, json!({})).await;
    expect_connected(&mut socket).await;
    socket
        .send(Message::Binary(
            b"echo \"laps: $(whoami)\"\n".to_vec().into(),
        ))
        .await
        .unwrap();
    output_until(&mut socket, "laps: tester").await;
    drop(socket);

    let log = send(&app, crate::common::get("/api/audit", Some(&token)))
        .await
        .json();
    let opened = log
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["action"] == "connection.opened")
        .unwrap();
    assert_eq!(opened["details"]["auth_mode"], "laps");
    assert_eq!(opened["details"]["username"], "tester");
    assert!(!log.to_string().contains("Tester-Passw0rd!"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn without_a_laps_password_there_is_no_connection(pool: PgPool) {
    let (state, app, token, folder) = setup(pool).await;
    let mut devices = Vec::new();
    for host in ["nolaps", "nowhere.example.com", "offline"] {
        devices.push(laps_device(&app, &token, &folder, host).await);
    }
    let address = serve(state).await;
    for (device, code) in devices.iter().zip([
        "laps_unavailable",
        "laps_unavailable",
        "directory_unavailable",
    ]) {
        let mut socket = open(address, device, &token, ORIGIN).await.unwrap();
        start(&mut socket, json!({})).await;
        assert_eq!(event(&mut socket).await["code"], code);
    }
    // VNC and web interfaces have passwords of their own, which LAPS does
    // not keep.
    for (protocol, port) in [("vnc", 5900), ("https", 443)] {
        let response = send(
            &app,
            authed(
                "POST",
                "/api/devices",
                Some(json!({
                    "folder_id": folder, "name": protocol, "protocol": protocol, "host": "x",
                    "port": port, "auth_mode": "laps", "credential_id": null,
                })),
                &token,
            ),
        )
        .await;
        assert_eq!(
            response.json()["params"]["field"],
            "auth_mode",
            "{protocol}"
        );
    }
}
