//! Site connectors (ADR 0008): managing them, and the tunnel end to end with
//! a connector running in this process against a real server.

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use axum::Router;
use remotehub_server::AppState;
use remotehub_server::connector_agent::{self, AgentSettings};
use remotehub_server::connectors::{Forward, STREAM_TIMEOUT};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

use crate::common::{authed, get, send, sign_in_request};
use crate::terminal::{create, serve, setup};

/// A connector created through the API: its id and token.
pub async fn new_connector(app: &Router, token: &str, name: &str) -> (Uuid, String) {
    let response = send(
        app,
        authed(
            "POST",
            "/api/connectors",
            Some(json!({ "name": name })),
            token,
        ),
    )
    .await;
    assert_eq!(response.status, 201, "{}", response.json());
    let body = response.json();
    (
        body["id"].as_str().unwrap().parse().unwrap(),
        body["token"].as_str().unwrap().to_owned(),
    )
}

/// Runs the connector agent against the server at `address`, restricted to
/// `allow`, until the task is dropped; returns once the server sees it.
pub async fn run_connector(
    state: &AppState,
    address: std::net::SocketAddr,
    id: Uuid,
    token: &str,
    allow: &str,
) -> AbortOnDrop {
    let settings = AgentSettings {
        url: format!("http://{address}").parse().unwrap(),
        token: SecretString::from(token.to_owned()),
        tls: connector_agent::tls_config(None).unwrap(),
        allow: allow
            .split(',')
            .filter(|n| !n.is_empty())
            .map(|n| n.parse().unwrap())
            .collect(),
    };
    let task = AbortOnDrop(tokio::spawn(connector_agent::run(settings)));
    for _ in 0..100 {
        if state.connectors.is_online(id) {
            return task;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the connector did not come online");
}

/// The connections the only connector carries, as the API reports them,
/// once they are `expected` or 2 s have passed: a stream ends a moment after
/// its connection closed.
pub async fn carried(app: &Router, token: &str, expected: u64) -> u64 {
    let mut now = 0;
    for _ in 0..40 {
        let listed = send(app, get("/api/connectors", Some(token))).await.json();
        now = listed[0]["streams"].as_u64().unwrap();
        if now == expected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    now
}

pub struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A device that echoes what it gets, on 127.0.0.1.
async fn echo() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buffer = [0u8; 1024];
                while let Ok(n) = stream.read(&mut buffer).await {
                    if n == 0 || stream.write_all(&buffer[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    address
}

/// What comes back for `message` through a forward, until the other side
/// closes or 5 s pass.
async fn round_trip(forward: &Forward, message: &[u8]) -> Vec<u8> {
    let mut engine = TcpStream::connect(forward.address).await.unwrap();
    engine.write_all(message).await.unwrap();
    let mut received = vec![0u8; message.len()];
    let mut filled = 0;
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        while filled < received.len() {
            match engine.read(&mut received[filled..]).await {
                Ok(0) | Err(_) => break,
                Ok(n) => filled += n,
            }
        }
    })
    .await;
    received.truncate(filled);
    received
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn administrators_manage_connectors_and_devices_name_them(pool: PgPool) {
    let (_state, app, token, folder) = setup(pool).await;
    let (id, secret) = new_connector(&app, &token, "Hamburg office").await;
    assert!(secret.starts_with("rhc_") && secret.len() > 40, "{secret}");

    // Others see names and state, never tokens, and create nothing.
    let bob = send(&app, sign_in_request("bob", "right"))
        .await
        .session_token()
        .unwrap();
    let listed = send(&app, get("/api/connectors", Some(&bob))).await.json();
    assert_eq!(
        listed,
        json!([{ "id": id, "name": "Hamburg office", "online": false, "streams": 0,
                 "streams_carried": 0, "last_seen_at": null }])
    );
    let refused = send(
        &app,
        authed(
            "POST",
            "/api/connectors",
            Some(json!({ "name": "x" })),
            &bob,
        ),
    )
    .await;
    assert_eq!(refused.status, 403);
    let taken = send(
        &app,
        authed(
            "POST",
            "/api/connectors",
            Some(json!({ "name": "Hamburg office" })),
            &token,
        ),
    )
    .await;
    assert_eq!(taken.json()["code"], "name_taken");

    // A device names an existing connector, and keeps it from being deleted.
    let device = |connector: Value| {
        json!({ "folder_id": folder, "name": "router", "protocol": "ssh", "host": "10.1.1.1",
                "port": 22, "auth_mode": "ask", "credential_id": null, "connector_id": connector })
    };
    let unknown = send(
        &app,
        authed(
            "POST",
            "/api/devices",
            Some(device(json!(Uuid::nil()))),
            &token,
        ),
    )
    .await;
    assert_eq!(unknown.json()["params"]["field"], "connector_id");
    let router = create(&app, &token, "/api/devices", device(json!(id))).await;
    let tree = send(&app, get("/api/tree", Some(&token))).await.json();
    assert_eq!(tree["devices"][0]["connector_id"], json!(id));
    let in_use = send(
        &app,
        authed("DELETE", &format!("/api/connectors/{id}"), None, &token),
    )
    .await;
    assert_eq!(in_use.json()["code"], "connector_in_use");
    let deleted = send(
        &app,
        authed("DELETE", &format!("/api/devices/{router}"), None, &token),
    )
    .await;
    assert_eq!(deleted.status, 204);
    let deleted = send(
        &app,
        authed("DELETE", &format!("/api/connectors/{id}"), None, &token),
    )
    .await;
    assert_eq!(deleted.status, 204);

    let log = send(&app, get("/api/audit", Some(&token))).await.json();
    let log = log.to_string();
    assert!(log.contains("connector.created") && log.contains("connector.deleted"));
    assert!(!log.contains(&secret));
}

/// Another connector is another target: the pins go, and a linked
/// credential needs `connect` like for a new host.
#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn moving_a_device_to_a_connector_forgets_its_pins(pool: PgPool) {
    let (_state, app, token, folder) = setup(pool.clone()).await;
    let (id, _) = new_connector(&app, &token, "Hamburg office").await;
    let body = |connector: Value| {
        json!({ "folder_id": folder, "name": "router", "protocol": "ssh", "host": "10.1.1.1",
                "port": 22, "auth_mode": "ask", "credential_id": null, "connector_id": connector })
    };
    let router = create(&app, &token, "/api/devices", body(Value::Null)).await;
    sqlx::query("UPDATE devices SET host_key = 'ssh-ed25519 AAAA', host_key_pinned_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    let moved = send(
        &app,
        authed(
            "PUT",
            &format!("/api/devices/{router}"),
            Some(body(json!(id))),
            &token,
        ),
    )
    .await;
    assert_eq!(moved.status, 204, "{}", moved.json());
    let pinned: Option<String> = sqlx::query_scalar("SELECT host_key FROM devices")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(pinned, None);
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_forward_reaches_the_device_through_the_connector(pool: PgPool) {
    let (state, app, token, _folder) = setup(pool).await;
    let (id, secret) = new_connector(&app, &token, "lab").await;
    let address = serve(state.clone()).await;
    let device = echo().await;
    let loopback = IpAddr::from(Ipv4Addr::LOCALHOST);

    // Offline: nothing is carried.
    let forward = Forward::open(
        state.connectors.clone(),
        id,
        device.clone(),
        loopback,
        vec![loopback],
    )
    .await
    .unwrap();
    assert_eq!(round_trip(&forward, b"hello").await, b"");

    let _connector = run_connector(&state, address, id, &secret, "").await;
    assert_eq!(round_trip(&forward, b"hello").await, b"hello");
    let mut held = TcpStream::connect(forward.address).await.unwrap();
    held.write_all(b"x").await.unwrap();
    held.read_exact(&mut [0u8; 1]).await.unwrap();
    assert_eq!(carried(&app, &token, 1).await, 1);
    drop(held);
    assert_eq!(carried(&app, &token, 0).await, 0);
    // Every connection gets its own stream.
    let big = vec![7u8; 300_000];
    assert_eq!(round_trip(&forward, &big).await.len(), big.len());

    // A forward is only for the engine it was opened for.
    let other = Forward::open(
        state.connectors.clone(),
        id,
        device,
        loopback,
        vec![IpAddr::from(Ipv4Addr::new(10, 9, 9, 9))],
    )
    .await
    .unwrap();
    assert_eq!(round_trip(&other, b"hello").await, b"");
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_connector_reaches_only_what_it_allows(pool: PgPool) {
    let (state, app, token, _folder) = setup(pool).await;
    let (id, secret) = new_connector(&app, &token, "lab").await;
    let address = serve(state.clone()).await;
    let device = echo().await;
    let _connector = run_connector(&state, address, id, &secret, "10.0.0.0/8").await;
    let refused = state
        .connectors
        .stream(id, &device, STREAM_TIMEOUT)
        .await
        .err()
        .map(|e| e.to_string());
    assert_eq!(
        refused.as_deref(),
        Some("the connector could not reach the target: not allowed")
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_wrong_token_opens_nothing(pool: PgPool) {
    let (state, app, token, _folder) = setup(pool).await;
    let (id, secret) = new_connector(&app, &token, "lab").await;
    let address = serve(state.clone()).await;
    let settings = AgentSettings {
        url: format!("http://{address}").parse().unwrap(),
        token: SecretString::from("rhc_wrong".to_owned()),
        tls: connector_agent::tls_config(None).unwrap(),
        allow: Vec::new(),
    };
    let _task = AbortOnDrop(tokio::spawn(connector_agent::run(settings)));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!state.connectors.is_online(id));

    // Streams: only with the token, and only one that was asked for.
    let stream = |bearer: Option<&str>| {
        let mut request = format!("ws://{address}/api/connectors/streams/{}", Uuid::nil())
            .into_client_request()
            .unwrap();
        if let Some(bearer) = bearer {
            request
                .headers_mut()
                .insert("authorization", format!("Bearer {bearer}").parse().unwrap());
        }
        tokio_tungstenite::connect_async(request)
    };
    for (bearer, status) in [
        (None, 401),
        (Some("rhc_wrong"), 401),
        (Some(secret.as_str()), 404),
    ] {
        match stream(bearer).await {
            Err(tungstenite::Error::Http(response)) => {
                assert_eq!(response.status().as_u16(), status, "{bearer:?}");
            }
            other => panic!("{bearer:?}: {:?}", other.map(|_| ())),
        }
    }
}
