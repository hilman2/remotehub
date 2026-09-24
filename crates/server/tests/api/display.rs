//! The display WebSocket (RDP and VNC through guacd) end to end: a real
//! server, a WebSocket client like the browser, and (lab tests) guacd and the
//! test lab's desktop target.

use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::common::{ORIGIN, get, send};
use crate::terminal::{create, serve, setup};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn open(
    address: SocketAddr,
    device: &str,
    token: &str,
    endpoint: &str,
    origin: &str,
) -> Result<Socket, tungstenite::Error> {
    let mut request = format!("ws://{address}/api/devices/{device}/{endpoint}")
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

fn status(result: Result<Socket, tungstenite::Error>) -> u16 {
    match result {
        Err(tungstenite::Error::Http(response)) => response.status().as_u16(),
        Err(other) => panic!("{other}"),
        Ok(_) => 101,
    }
}

async fn start(socket: &mut Socket, extra: Value) {
    let mut start = json!({ "type": "start", "width": 1024, "height": 768, "dpi": 96, "timezone": "Europe/Berlin" });
    start
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    socket
        .send(Message::Text(start.to_string().into()))
        .await
        .unwrap();
}

/// The next text frame.
async fn text(socket: &mut Socket) -> String {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(20), socket.next())
            .await
            .expect("a frame within 20 s")
            .expect("the socket is open")
            .unwrap();
        if let Message::Text(text) = message {
            return text.to_string();
        }
    }
}

/// Guacamole instructions until `needle` shows up.
async fn instructions_until(socket: &mut Socket, needle: &str) -> String {
    let mut seen = String::new();
    while !seen.contains(needle) {
        let message = tokio::time::timeout(Duration::from_secs(20), socket.next())
            .await
            .unwrap_or_else(|_| panic!("{needle:?} not seen in: {seen:.500}"))
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            seen.push_str(&text);
        }
    }
    seen
}

/// The lab certificate's fingerprint (deploy/testlab/desktop/README.md).
const LAB_CERTIFICATE: &str = "C1:E8:6D:13:4E:8D:B7:A5:D2:72:01:8F:93:8F:C4:44:EC:E4:C0:D5:97:C8:00:EF:25:24:BB:76:22:0B:DF:CD";

/// The server's first frame, JSON.
async fn connected(socket: &mut Socket) -> Value {
    serde_json::from_str(&text(socket).await).unwrap()
}

fn desktop_host() -> String {
    std::env::var("REMOTEHUB_TEST_DESKTOP_HOST")
        .expect("REMOTEHUB_TEST_DESKTOP_HOST points to the test lab")
}

async fn device(app: &Router, token: &str, folder: &str, device: Value) -> String {
    let mut body = json!({ "folder_id": folder, "name": "desktop", "host": desktop_host() });
    body.as_object_mut()
        .unwrap()
        .extend(device.as_object().unwrap().clone());
    create(app, token, "/api/devices", body).await
}

#[sqlx::test(migrations = "../../migrations")]
async fn each_protocol_has_its_endpoint_and_only_the_own_origin_opens_it(pool: PgPool) {
    let (state, app, token, folder) = setup(pool).await;
    let ask = json!({ "auth_mode": "ask", "credential_id": null });
    let mut rdp = json!({ "name": "rdp", "protocol": "rdp", "port": 3389 });
    rdp.as_object_mut()
        .unwrap()
        .extend(ask.as_object().unwrap().clone());
    let rdp = device(&app, &token, &folder, rdp).await;
    let mut ssh = json!({ "name": "ssh", "protocol": "ssh", "port": 22 });
    ssh.as_object_mut()
        .unwrap()
        .extend(ask.as_object().unwrap().clone());
    let ssh = device(&app, &token, &folder, ssh).await;
    let address = serve(state).await;

    assert_eq!(
        status(open(address, &rdp, &token, "display", "https://evil.example").await),
        403
    );
    assert_eq!(
        status(open(address, &rdp, "no-session", "display", ORIGIN).await),
        401
    );
    assert_eq!(
        status(open(address, &ssh, &token, "display", ORIGIN).await),
        400
    );
    assert_eq!(
        status(open(address, &rdp, &token, "terminal", ORIGIN).await),
        400
    );
    let unknown = "00000000-0000-0000-0000-000000000000";
    assert_eq!(
        status(open(address, unknown, &token, "display", ORIGIN).await),
        404
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_ssh_key_does_not_sign_in_to_rdp(pool: PgPool) {
    let (state, app, token, folder) = setup(pool).await;
    let key = std::fs::read_to_string(
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("../../deploy/testlab/ssh/tester_ed25519"),
    )
    .unwrap();
    let credential = create(
        &app,
        &token,
        "/api/credentials",
        json!({ "folder_id": folder, "name": "key", "kind": "ssh_key", "username": "tester", "private_key": key }),
    )
    .await;
    let rdp = device(
        &app,
        &token,
        &folder,
        json!({ "protocol": "rdp", "port": 3389, "auth_mode": "stored", "credential_id": credential }),
    )
    .await;
    let address = serve(state).await;

    let mut socket = open(address, &rdp, &token, "display", ORIGIN)
        .await
        .unwrap();
    start(&mut socket, json!({})).await;
    let error: Value = serde_json::from_str(&text(&mut socket).await).unwrap();
    assert_eq!(error["code"], "invalid_request", "{error}");
    assert_eq!(error["params"]["field"], "credential_id");
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn a_stored_credential_opens_an_rdp_desktop(pool: PgPool) {
    let (state, app, token, folder) = setup(pool).await;
    let credential = create(
        &app,
        &token,
        "/api/credentials",
        json!({ "folder_id": folder, "name": "tester", "username": "tester", "password": "Tester-Passw0rd!" }),
    )
    .await;
    let rdp = device(
        &app,
        &token,
        &folder,
        json!({ "protocol": "rdp", "port": 3389, "auth_mode": "stored", "credential_id": credential }),
    )
    .await;
    let address = serve(state).await;

    let mut socket = open(address, &rdp, &token, "display", ORIGIN)
        .await
        .unwrap();
    start(&mut socket, json!({})).await;
    let connected: Value = serde_json::from_str(&text(&mut socket).await).unwrap();
    assert_eq!(connected["type"], "connected", "{connected}");
    assert_eq!(connected["certificate_fingerprint"], LAB_CERTIFICATE);
    assert_eq!(connected["pinned"], true);
    let seen = instructions_until(&mut socket, "3.img,").await;
    assert!(!seen.contains("Tester-Passw0rd!"));

    // Input reaches the desktop; changing the connection's parameters does not.
    socket
        .send(Message::Text(
            "5.mouse,3.100,3.100,1.0;4.argv,1.0,10.text/plain,8.password;".into(),
        ))
        .await
        .unwrap();
    socket.send(Message::Close(None)).await.unwrap();
    drop(socket);

    tokio::time::sleep(Duration::from_millis(500)).await;
    let log = send(&app, get("/api/audit", Some(&token))).await.json();
    let closed = log
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["action"] == "connection.closed")
        .unwrap_or_else(|| panic!("no connection.closed in {log}"));
    assert_eq!(closed["details"]["protocol"], "rdp");
    assert_eq!(closed["details"]["dropped_instructions"], 1, "{closed}");
    assert!(log.to_string().contains("connection.opened"));
    assert!(!log.to_string().contains("Tester-Passw0rd!"));
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn vnc_asks_for_the_password_and_reports_a_wrong_one(pool: PgPool) {
    let (state, app, token, folder) = setup(pool).await;
    let vnc = device(
        &app,
        &token,
        &folder,
        json!({ "protocol": "vnc", "port": 5900, "auth_mode": "ask", "credential_id": null }),
    )
    .await;
    let address = serve(state).await;

    let mut socket = open(address, &vnc, &token, "display", ORIGIN)
        .await
        .unwrap();
    start(&mut socket, json!({ "password": "Vnc-Pw1!" })).await;
    assert_eq!(connected(&mut socket).await["type"], "connected");
    instructions_until(&mut socket, "3.img,").await;
    drop(socket);

    // guacd reports the refusal as an instruction; the client shows it.
    let mut socket = open(address, &vnc, &token, "display", ORIGIN)
        .await
        .unwrap();
    start(&mut socket, json!({ "password": "wrong" })).await;
    assert_eq!(connected(&mut socket).await["type"], "connected");
    let seen = instructions_until(&mut socket, "5.error,").await;
    assert!(!seen.contains("3.img,"), "{seen:.300}");
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn a_changed_rdp_certificate_stops_the_connection_until_the_pin_is_forgotten(pool: PgPool) {
    let (state, app, token, folder) = setup(pool.clone()).await;
    let credential = create(
        &app,
        &token,
        "/api/credentials",
        json!({ "folder_id": folder, "name": "tester", "username": "tester", "password": "Tester-Passw0rd!" }),
    )
    .await;
    let rdp = device(
        &app,
        &token,
        &folder,
        json!({ "protocol": "rdp", "port": 3389, "auth_mode": "stored", "credential_id": credential }),
    )
    .await;
    let other = LAB_CERTIFICATE.replace("C1:E8", "00:00");
    sqlx::query("UPDATE devices SET certificate_fingerprint = $1, certificate_pinned_at = now()")
        .bind(&other)
        .execute(&pool)
        .await
        .unwrap();
    let address = serve(state).await;

    let mut socket = open(address, &rdp, &token, "display", ORIGIN)
        .await
        .unwrap();
    start(&mut socket, json!({})).await;
    let error = connected(&mut socket).await;
    assert_eq!(error["code"], "certificate_changed", "{error}");
    assert_eq!(error["params"]["expected"], other.as_str());
    assert_eq!(error["params"]["presented"], LAB_CERTIFICATE);

    // Someone with edit forgets the pin; the next connection pins again.
    let forgotten = send(
        &app,
        crate::common::authed(
            "DELETE",
            &format!("/api/devices/{rdp}/host-key"),
            None,
            &token,
        ),
    )
    .await;
    assert_eq!(forgotten.status, 204);
    let mut socket = open(address, &rdp, &token, "display", ORIGIN)
        .await
        .unwrap();
    start(&mut socket, json!({})).await;
    let connected = connected(&mut socket).await;
    assert_eq!(connected["pinned"], true, "{connected}");
    drop(socket);

    let tree = send(&app, get("/api/tree", Some(&token))).await.json();
    assert_eq!(
        tree["devices"][0]["certificate_fingerprint"],
        LAB_CERTIFICATE
    );
    let log = send(&app, get("/api/audit", Some(&token)))
        .await
        .json()
        .to_string();
    for action in [
        "device.certificate_reset",
        "device.certificate_pinned",
        "connection.failed",
    ] {
        assert!(log.contains(action), "{action} missing");
    }
}
