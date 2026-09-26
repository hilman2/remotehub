//! Site connectors (ADR 0008): managing them, and the tunnel end to end with
//! a connector running in this process against a real server.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use remotehub_connector::access::{Access, Changer, Gate};
use remotehub_connector::agent::{self, AgentSettings, Site};
use remotehub_connector::journal::{Event, Journal};
use remotehub_connector::protocol;
use remotehub_server::AppState;
use remotehub_server::connectors::{Forward, Requester, STREAM_TIMEOUT};
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

/// A connector agent running in this process, until dropped.
pub struct Agent {
    pub site: Site,
    /// Its data directory, as the command line would change it.
    pub dir: tempfile::TempDir,
    _task: AbortOnDrop,
}

/// Starts the connector agent against the server at `address`, restricted
/// to `allow`, with the customer's access as given.
pub fn start_connector(
    address: std::net::SocketAddr,
    token: &str,
    allow: &str,
    access: Access,
) -> Agent {
    let settings = AgentSettings {
        url: format!("http://{address}").parse().unwrap(),
        token: SecretString::from(token.to_owned()),
        tls: agent::tls_config(None).unwrap(),
        allow: allow
            .split(',')
            .filter(|n| !n.is_empty())
            .map(|n| n.parse().unwrap())
            .collect(),
    };
    let dir = tempfile::tempdir().unwrap();
    let journal = Arc::new(Journal::new(dir.path()));
    remotehub_connector::access::change(dir.path(), &journal, access, Changer::CommandLine)
        .unwrap();
    let site = Site {
        gate: Gate::new(dir.path(), journal.clone()).unwrap(),
        journal,
        connections: Arc::default(),
    };
    let task = AbortOnDrop(tokio::spawn(agent::run(settings, site.clone())));
    Agent {
        site,
        dir,
        _task: task,
    }
}

/// Runs a connector whose customer keeps access open, until the result is
/// dropped; returns once the server sees it.
pub async fn run_connector(
    state: &AppState,
    address: std::net::SocketAddr,
    id: Uuid,
    token: &str,
    allow: &str,
) -> Agent {
    let agent = start_connector(address, token, allow, Access::Open { until: None });
    wait_for(
        || state.connectors.is_online(id),
        "the connector to come online",
    )
    .await;
    agent
}

/// alice, connecting to the device named router.
fn alice() -> Requester {
    Requester {
        user: "alice".into(),
        device: "router".into(),
    }
}

/// Waits up to 5 s for `condition`.
pub async fn wait_for(condition: impl Fn() -> bool, what: &str) {
    for _ in 0..100 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("waited in vain for {what}");
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
                 "streams_carried": 0, "access": null, "open_until": null, "last_seen_at": null }])
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
                "port": 22, "auth_mode": "ask", "credential_id": null,
                "connector_mode": "connector", "connector_id": connector })
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
        let mode = if connector.is_null() {
            "inherit"
        } else {
            "connector"
        };
        json!({ "folder_id": folder, "name": "router", "protocol": "ssh", "host": "10.1.1.1",
                "port": 22, "auth_mode": "ask", "credential_id": null,
                "connector_mode": mode, "connector_id": connector })
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
        alice(),
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
        alice(),
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
        .stream(id, &device, None, STREAM_TIMEOUT)
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
    let _agent = start_connector(address, "rhc_wrong", "", Access::Open { until: None });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!state.connectors.is_online(id));
    assert!(state.connectors.access(id).is_none());

    // Streams: only with the token, and only one that was asked for.
    let version = protocol::VERSION.to_string();
    for (bearer, status) in [
        (None, 401),
        (Some("rhc_wrong"), 401),
        (Some(secret.as_str()), 404),
    ] {
        let path = format!("/api/connectors/streams/{}", Uuid::nil());
        let refused = refusal(address, &path, bearer, Some(&version)).await;
        assert_eq!(refused.status().as_u16(), status, "{bearer:?}");
    }
}

/// A connector of another release learns which protocol version remotehub
/// speaks, on the control socket and on streams alike.
#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_connector_of_another_protocol_version_is_refused(pool: PgPool) {
    let (state, app, token, _folder) = setup(pool).await;
    let (id, secret) = new_connector(&app, &token, "lab").await;
    let address = serve(state.clone()).await;
    let stream = format!("/api/connectors/streams/{}", Uuid::nil());
    for path in ["/api/connectors/control", stream.as_str()] {
        for theirs in [None, Some("0"), Some("99")] {
            let refused = refusal(address, path, Some(&secret), theirs).await;
            assert_eq!(refused.status().as_u16(), 426, "{path} {theirs:?}");
            assert_eq!(
                refused.headers()[protocol::HEADER],
                protocol::VERSION.to_string().as_str()
            );
        }
    }
    assert!(!state.connectors.is_online(id));
}

/// The answer to a WebSocket request that remotehub refuses, signed in with
/// `bearer` and claiming the protocol version `theirs`.
async fn refusal(
    address: std::net::SocketAddr,
    path: &str,
    bearer: Option<&str>,
    theirs: Option<&str>,
) -> tungstenite::http::Response<Option<Vec<u8>>> {
    let mut request = format!("ws://{address}{path}")
        .into_client_request()
        .unwrap();
    if let Some(bearer) = bearer {
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {bearer}").parse().unwrap());
    }
    if let Some(theirs) = theirs {
        request
            .headers_mut()
            .insert(protocol::HEADER, theirs.parse().unwrap());
    }
    match tokio_tungstenite::connect_async(request).await {
        Err(tungstenite::Error::Http(response)) => *response,
        other => panic!("{path}: {:?}", other.map(|_| ())),
    }
}

// ── The customer's access (#165) ──────────────────────────────────────────

fn reported_open(state: &AppState, id: Uuid) -> Option<bool> {
    state.connectors.access(id).map(|access| access.open)
}

/// A closed connector keeps no control socket, but tells remotehub that the
/// customer closed it. Opening brings it online; the audit log records each
/// change.
#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_customer_opens_and_closes_access(pool: PgPool) {
    let (state, app, token, _folder) = setup(pool).await;
    let (id, secret) = new_connector(&app, &token, "lab").await;
    let address = serve(state.clone()).await;
    let agent = start_connector(address, &secret, "", Access::Closed);
    wait_for(
        || reported_open(&state, id) == Some(false),
        "a closed report",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!state.connectors.is_online(id));
    let listed = send(&app, get("/api/connectors", Some(&token)))
        .await
        .json();
    assert_eq!(listed[0]["access"], "closed");

    let until = time::OffsetDateTime::now_utc() + Duration::from_secs(3600);
    let carol = Changer::Web {
        user: "carol".into(),
    };
    agent
        .site
        .gate
        .set(Access::Open { until: Some(until) }, carol.clone())
        .unwrap();
    wait_for(
        || state.connectors.is_online(id),
        "the connector to come online",
    )
    .await;
    wait_for(|| reported_open(&state, id) == Some(true), "an open report").await;
    let listed = send(&app, get("/api/connectors", Some(&token)))
        .await
        .json();
    assert_eq!(listed[0]["access"], "open");
    let reported: time::OffsetDateTime = time::OffsetDateTime::parse(
        listed[0]["open_until"].as_str().unwrap(),
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    assert_eq!(reported.unix_timestamp(), until.unix_timestamp());

    agent.site.gate.set(Access::Closed, carol).unwrap();
    wait_for(|| !state.connectors.is_online(id), "the connector to go").await;
    wait_for(
        || reported_open(&state, id) == Some(false),
        "a closed report",
    )
    .await;

    let log = send(&app, get("/api/audit", Some(&token))).await.json();
    let log = log.to_string();
    assert!(
        log.contains("connector.opened") && log.contains("connector.closed"),
        "{log}"
    );
}

/// Closing ends running connections at once, and so does the time running
/// out. The customer's journal holds each connection with the remotehub
/// user it was for.
#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn closing_ends_running_connections(pool: PgPool) {
    let (state, app, token, _folder) = setup(pool).await;
    let (id, secret) = new_connector(&app, &token, "lab").await;
    let address = serve(state.clone()).await;
    let device = echo().await;
    let loopback = IpAddr::from(Ipv4Addr::LOCALHOST);
    let forward = Forward::open(
        state.connectors.clone(),
        id,
        device,
        alice(),
        loopback,
        vec![loopback],
    )
    .await
    .unwrap();
    let agent = run_connector(&state, address, id, &secret, "").await;

    /// A connection through the connector that has carried a byte each way.
    async fn held(forward: &Forward) -> TcpStream {
        let mut held = TcpStream::connect(forward.address).await.unwrap();
        held.write_all(b"x").await.unwrap();
        held.read_exact(&mut [0u8; 1]).await.unwrap();
        held
    }
    /// Whether the connection ends within `seconds`.
    async fn ends_within(held: &mut TcpStream, seconds: u64) -> bool {
        let mut buffer = [0u8; 16];
        let read = held.read(&mut buffer);
        matches!(
            tokio::time::timeout(Duration::from_secs(seconds), read).await,
            Ok(Ok(0) | Err(_))
        )
    }

    let mut first = held(&forward).await;
    agent
        .site
        .gate
        .set(
            Access::Closed,
            Changer::Web {
                user: "carol".into(),
            },
        )
        .unwrap();
    assert!(
        ends_within(&mut first, 3).await,
        "closing left the connection open"
    );
    let ended = agent
        .site
        .journal
        .recent(10)
        .into_iter()
        .find_map(|e| match e.event {
            Event::ConnectionEnded {
                user,
                device,
                sent,
                received,
                ..
            } => Some((user, device, sent, received)),
            _ => None,
        });
    // Named as in remotehub (#177), with the user it was for.
    assert_eq!(
        ended,
        Some((Some("alice".to_owned()), Some("router".to_owned()), 1, 1))
    );

    let until = time::OffsetDateTime::now_utc() + Duration::from_secs(2);
    agent
        .site
        .gate
        .set(Access::Open { until: Some(until) }, Changer::CommandLine)
        .unwrap();
    wait_for(
        || state.connectors.is_online(id),
        "the connector to come online",
    )
    .await;
    let mut second = held(&forward).await;
    assert!(
        ends_within(&mut second, 5).await,
        "the time ran out, the connection did not"
    );
    wait_for(
        || reported_open(&state, id) == Some(false),
        "a closed report",
    )
    .await;
    assert!(
        agent
            .site
            .journal
            .recent(10)
            .iter()
            .any(|e| e.event == Event::Expired)
    );
}

/// The command line changes the file; the running connector follows.
#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_command_line_opens_a_running_connector(pool: PgPool) {
    let (state, app, token, _folder) = setup(pool).await;
    let (id, secret) = new_connector(&app, &token, "lab").await;
    let address = serve(state.clone()).await;
    let agent = start_connector(address, &secret, "", Access::Closed);
    wait_for(
        || reported_open(&state, id) == Some(false),
        "a closed report",
    )
    .await;
    remotehub_connector::access::change(
        agent.dir.path(),
        &Journal::new(agent.dir.path()),
        Access::Open { until: None },
        Changer::CommandLine,
    )
    .unwrap();
    wait_for(
        || state.connectors.is_online(id),
        "the connector to come online",
    )
    .await;
}

/// A report reaches the audit log and the UI only as a point in time.
#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_report_holds_a_point_in_time_or_none(pool: PgPool) {
    let (state, app, token, _folder) = setup(pool).await;
    let (id, secret) = new_connector(&app, &token, "lab").await;
    let report = |until: Value| {
        axum::http::Request::post("/api/connectors/state")
            .header("authorization", format!("Bearer {secret}"))
            .header(protocol::HEADER, protocol::VERSION)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({ "open": true, "until": until }).to_string(),
            ))
            .unwrap()
    };
    let refused = send(&app, report(json!("<b>soon</b>"))).await;
    assert_eq!(refused.status, 400);
    assert_eq!(refused.json()["params"]["field"], "until");
    assert!(state.connectors.access(id).is_none());

    let taken = send(&app, report(json!("2026-10-01T18:00:00+02:00"))).await;
    assert_eq!(taken.status, 204);
    assert_eq!(
        state.connectors.access(id).unwrap().until.as_deref(),
        Some("2026-10-01T16:00:00Z")
    );
}
