//! The display WebSocket (RDP, VNC and HTTPS through guacd) end to end: a
//! real server, a WebSocket client like the browser, and (lab tests) guacd,
//! the browser service and the test lab's desktop and web targets.

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

/// Guacamole instructions until `needle` shows up. Fails at once when guacd
/// disconnects first, instead of waiting out the timeout.
async fn instructions_until(socket: &mut Socket, needle: &str) -> String {
    let mut seen = String::new();
    while !seen.contains(needle) {
        assert!(
            !seen.contains("10.disconnect;"),
            "{needle:?} not seen before guacd disconnected: {seen:.500}"
        );
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

/// Leaves the session as a browser does, and waits until the server has
/// closed its side, and with it the guacd connection. A test that only
/// dropped the socket would end the server with it before it said goodbye to
/// guacd, and xrdp would keep the session (#75).
async fn leave(mut socket: Socket) {
    let _ = socket.send(Message::Close(None)).await;
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Close(_) = message {
                break;
            }
        }
    })
    .await;
}

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
    leave(socket).await;
    // The server records the end after it has closed the socket.
    let mut log = Value::Null;
    for _ in 0..50 {
        log = send(&app, get("/api/audit", Some(&token))).await.json();
        if log.to_string().contains("connection.closed") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
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
    leave(socket).await;

    // guacd reports the refusal as an instruction; the client shows it.
    let mut socket = open(address, &vnc, &token, "display", ORIGIN)
        .await
        .unwrap();
    start(&mut socket, json!({ "password": "wrong" })).await;
    assert_eq!(connected(&mut socket).await["type"], "connected");
    let seen = instructions_until(&mut socket, "5.error,").await;
    assert!(!seen.contains("3.img,"), "{seen:.300}");
    leave(socket).await;
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
    leave(socket).await;

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

/// A desktop of the test lab, open and drawn: RDP with a stored credential,
/// VNC with the password as asked.
async fn open_desktop(pool: PgPool, protocol: &str) -> Socket {
    let (state, app, token, folder) = setup(pool).await;
    let (id, start_with) = if protocol == "rdp" {
        let credential = create(
            &app,
            &token,
            "/api/credentials",
            json!({ "folder_id": folder, "name": "tester", "username": "tester", "password": "Tester-Passw0rd!" }),
        )
        .await;
        let rdp = json!({ "protocol": "rdp", "port": 3389, "auth_mode": "stored", "credential_id": credential });
        (device(&app, &token, &folder, rdp).await, json!({}))
    } else {
        let vnc =
            json!({ "protocol": "vnc", "port": 5900, "auth_mode": "ask", "credential_id": null });
        (
            device(&app, &token, &folder, vnc).await,
            json!({ "password": "Vnc-Pw1!" }),
        )
    };
    let address = serve(state).await;
    let mut socket = open(address, &id, &token, "display", ORIGIN).await.unwrap();
    start(&mut socket, start_with).await;
    assert_eq!(connected(&mut socket).await["type"], "connected");
    instructions_until(&mut socket, "3.img,").await;
    socket
}

/// Puts `text` on the session's clipboard and waits for the lab desktop's
/// answer (`echo:<text>`, deploy/testlab/desktop/clipboard-echo) to come back
/// as a clipboard stream.
async fn clipboard_round_trip(socket: &mut Socket, text: &str) {
    use base64::Engine;
    let base64 = base64::engine::general_purpose::STANDARD;
    let blob = base64.encode(text);
    socket
        .send(Message::Text(
            format!(
                "9.clipboard,1.0,10.text/plain;4.blob,1.0,{}.{blob};3.end,1.0;",
                blob.len()
            )
            .into(),
        ))
        .await
        .unwrap();
    let expected = format!("echo:{text}");
    let mut received = Vec::new();
    let mut parser = remotehub_gateway::guacamole::Parser::default();
    // Clipboard streams by index, with the blobs so far.
    let mut streams = std::collections::HashMap::<String, Vec<u8>>::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !received.contains(&expected) {
        let message = tokio::time::timeout_at(deadline, socket.next())
            .await
            .unwrap_or_else(|_| panic!("{expected:?} not among the clipboards {received:?}"))
            .unwrap()
            .unwrap();
        let Message::Text(frame) = message else {
            continue;
        };
        parser.push(frame.as_bytes());
        while let Some(instruction) = parser.next_instruction().unwrap() {
            let args = &instruction.args;
            match instruction.opcode.as_str() {
                "clipboard" => {
                    streams.insert(args[0].clone(), Vec::new());
                }
                "blob" if streams.contains_key(&args[0]) => {
                    let data = base64.decode(&args[1]).unwrap();
                    streams.get_mut(&args[0]).unwrap().extend(data);
                }
                "end" => {
                    if let Some(data) = streams.remove(&args[0]) {
                        received.push(String::from_utf8_lossy(&data).into_owned());
                    }
                }
                _ => {}
            }
        }
    }
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn text_crosses_the_rdp_clipboard_both_ways(pool: PgPool) {
    let mut socket = open_desktop(pool, "rdp").await;
    // Not beyond U+FFFF: guacd 1.6.0 writes the RDP clipboard's UTF-16
    // without surrogate pairs (GUAC_WRITE_UTF16 in src/common/iconv.c), so
    // U+1F600 arrives as U+F600.
    clipboard_round_trip(&mut socket, "über RDP €").await;
    leave(socket).await;
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn text_crosses_the_vnc_clipboard_both_ways(pool: PgPool) {
    let mut socket = open_desktop(pool, "vnc").await;
    // VNC carries Latin-1 only; the VNC display is shared with other tests,
    // so the text is this test's own.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    clipboard_round_trip(&mut socket, &format!("vnc {}", now.as_nanos())).await;
    leave(socket).await;
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn an_rdp_desktop_follows_the_browser_window(pool: PgPool) {
    let mut socket = open_desktop(pool, "rdp").await;
    // Opened at 1024×768 (`start`); the browser's window changes.
    socket
        .send(Message::Text("4.size,4.1280,3.720;".into()))
        .await
        .unwrap();
    instructions_until(&mut socket, "4.size,1.0,4.1280,3.720;").await;
    leave(socket).await;
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn an_rdp_desktop_opens_with_its_laps_password(pool: PgPool) {
    let (state, app, token, folder) = setup(pool).await;
    // The fake directory knows the lab desktop's LAPS password by its name.
    let rdp = device(
        &app,
        &token,
        &folder,
        json!({ "host": "desktop-target", "protocol": "rdp", "port": 3389, "auth_mode": "laps", "credential_id": null }),
    )
    .await;
    let address = serve(state).await;
    let mut socket = open(address, &rdp, &token, "display", ORIGIN)
        .await
        .unwrap();
    start(&mut socket, json!({})).await;
    assert_eq!(connected(&mut socket).await["type"], "connected");
    let seen = instructions_until(&mut socket, "3.img,").await;
    assert!(!seen.contains("5.error,"), "{seen:.300}");
    leave(socket).await;
}

// ── HTTPS: the browser service and the lab's web target ─────────────────────

fn web_host() -> String {
    std::env::var("REMOTEHUB_TEST_WEB_HOST")
        .expect("REMOTEHUB_TEST_WEB_HOST points to the test lab")
}

fn browser_service() -> String {
    std::env::var("REMOTEHUB_TEST_BROWSER").expect("REMOTEHUB_TEST_BROWSER points to the test lab")
}

/// The web target's record of sign-ins (deploy/testlab/web), over its plain
/// HTTP port.
async fn sign_ins() -> Value {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = TcpStream::connect((web_host(), 8080)).await.unwrap();
    stream
        .write_all(b"GET /last HTTP/1.0\r\nHost: web-target\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    serde_json::from_str(body).unwrap()
}

/// The next sign-in after `count` sign-ins.
async fn sign_in_after(count: &Value) -> Value {
    for _ in 0..100 {
        let now = sign_ins().await;
        if now["count"].as_u64() > count.as_u64() {
            return now;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no sign-in after {count}");
}

async fn web_device(app: &Router, token: &str, folder: &str, device: Value) -> String {
    let mut body = json!({ "folder_id": folder, "name": "appliance", "protocol": "https",
                           "host": web_host(), "port": 443 });
    body.as_object_mut()
        .unwrap()
        .extend(device.as_object().unwrap().clone());
    create(app, token, "/api/devices", body).await
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn an_https_device_opens_signed_in_with_its_stored_credential(pool: PgPool) {
    let (state, app, token, folder) = setup(pool).await;
    let credential = create(
        &app,
        &token,
        "/api/credentials",
        json!({ "folder_id": folder, "name": "tester", "username": "tester", "password": "Tester-Passw0rd!" }),
    )
    .await;
    let web = web_device(
        &app,
        &token,
        &folder,
        json!({ "auth_mode": "stored", "credential_id": credential }),
    )
    .await;
    let address = serve(state).await;
    let before = sign_ins().await;

    let mut socket = open(address, &web, &token, "display", ORIGIN)
        .await
        .unwrap();
    start(&mut socket, json!({})).await;
    let connected = connected(&mut socket).await;
    assert_eq!(connected["type"], "connected", "{connected}");
    assert_eq!(connected["pinned"], true, "{connected}");
    let seen = instructions_until(&mut socket, "3.img,").await;
    assert!(!seen.contains("Tester-Passw0rd!"));
    let signed_in = sign_in_after(&before["count"]).await;
    assert_eq!(signed_in["username"], "tester", "{signed_in}");
    assert_eq!(signed_in["ok"], true, "{signed_in}");
    // The sign-in page also loads an image from another port of the target,
    // which counts as another device: the browser's proxy refuses it.
    assert_eq!(signed_in["escapes"], before["escapes"], "{signed_in}");

    leave(socket).await;
    let mut log = Value::Null;
    for _ in 0..50 {
        log = send(&app, get("/api/audit", Some(&token))).await.json();
        if log.to_string().contains("connection.closed") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let closed = log
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["action"] == "connection.closed")
        .unwrap_or_else(|| panic!("no connection.closed in {log}"));
    assert_eq!(closed["details"]["protocol"], "https");
    assert_eq!(closed["details"]["signed_in"], true, "{closed}");
    assert!(!log.to_string().contains("Tester-Passw0rd!"));
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn asked_credentials_reach_the_web_interface_as_typed(pool: PgPool) {
    let (state, app, token, folder) = setup(pool).await;
    let web = web_device(
        &app,
        &token,
        &folder,
        json!({ "auth_mode": "ask", "credential_id": null }),
    )
    .await;
    let address = serve(state).await;
    let before = sign_ins().await["count"].clone();

    let mut socket = open(address, &web, &token, "display", ORIGIN)
        .await
        .unwrap();
    start(
        &mut socket,
        json!({ "username": "tester", "password": "not-the-password" }),
    )
    .await;
    assert_eq!(connected(&mut socket).await["type"], "connected");
    let signed_in = sign_in_after(&before).await;
    assert_eq!(signed_in["username"], "tester", "{signed_in}");
    assert_eq!(signed_in["ok"], false, "{signed_in}");
    leave(socket).await;
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn a_changed_https_certificate_stops_the_connection(pool: PgPool) {
    let (state, app, token, folder) = setup(pool.clone()).await;
    let web = web_device(
        &app,
        &token,
        &folder,
        json!({ "auth_mode": "ask", "credential_id": null }),
    )
    .await;
    let presented =
        remotehub_gateway::tls::https_certificate(&web_host(), 443, Duration::from_secs(5))
            .await
            .unwrap()
            .fingerprint;
    let other = format!("00:00{}", &presented[5..]);
    sqlx::query("UPDATE devices SET certificate_fingerprint = $1, certificate_pinned_at = now()")
        .bind(&other)
        .execute(&pool)
        .await
        .unwrap();
    let address = serve(state).await;
    let before = sign_ins().await["count"].clone();

    let mut socket = open(address, &web, &token, "display", ORIGIN)
        .await
        .unwrap();
    start(
        &mut socket,
        json!({ "username": "tester", "password": "Tester-Passw0rd!" }),
    )
    .await;
    let error = connected(&mut socket).await;
    assert_eq!(error["code"], "certificate_changed", "{error}");
    assert_eq!(error["params"]["presented"], presented.as_str());
    assert_eq!(sign_ins().await["count"], before);
}

/// Chromium itself holds to the pinned key: with another one it shows its
/// error page, and the agent types nothing.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs the test lab"]
async fn the_browser_signs_in_only_where_the_pinned_key_is(_pool: PgPool) {
    use remotehub_browser::client::{self, Request};
    use remotehub_browser::protocol::Reply;

    let host = web_host();
    let right = remotehub_gateway::tls::https_certificate(&host, 443, Duration::from_secs(5))
        .await
        .unwrap()
        .spki;
    let before = sign_ins().await["count"].clone();
    // Open until the form has been sent: the browser ends with its session.
    let mut sessions = Vec::new();
    for (spki, expected) in [
        (
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            Reply::NotFilled {
                reason: "page_error".into(),
            },
        ),
        (right.as_str(), Reply::Filled),
    ] {
        let mut session = client::open(
            &browser_service(),
            &Request {
                host: &host,
                port: 443,
                spki,
                width: 1024,
                height: 768,
                timezone: None,
                login: Some(("tester", "Tester-Passw0rd!")),
            },
            Duration::from_secs(20),
        )
        .await
        .unwrap();
        let answer = tokio::time::timeout(Duration::from_secs(40), session.next())
            .await
            .expect("an answer within 40 s");
        assert_eq!(answer, Some(expected), "{spki}");
        sessions.push(session);
    }
    let after = sign_in_after(&before).await;
    assert_eq!(
        after["count"].as_u64(),
        before.as_u64().map(|n| n + 1),
        "one sign-in, with the right key: {after}"
    );
}
