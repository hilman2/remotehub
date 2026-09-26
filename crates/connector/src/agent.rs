//! The connector's way to remotehub: while the customer keeps access open
//! (#165), a control WebSocket to remotehub, and a connection to a device in
//! this network whenever remotehub asks, each with a WebSocket of its own
//! for its bytes. Open or closed, it reports the access to remotehub.
//!
//! Configuration from the environment (each also as `NAME_FILE`):
//! - `REMOTEHUB_URL`: remotehub's public address, e.g.
//!   `https://remotehub.example.com`
//! - `REMOTEHUB_CONNECTOR_TOKEN`: the token shown when the connector was
//!   created in remotehub
//! - `REMOTEHUB_CONNECTOR_CA_FILE`: PEM file with the CA of remotehub's
//!   certificate, if the system does not trust it
//! - `REMOTEHUB_CONNECTOR_ALLOW`: address ranges the connector may connect
//!   to, separated by commas; all if unset

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::{CertificateDer, ServerName, pem::PemObject};
use secrecy::{ExposeSecret, SecretString};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode, Uri};
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::access::{self, Gate, Snapshot, later};
use crate::inventory::{self, Address};
use crate::journal::{Event, Journal};
use crate::network::Network;
use crate::protocol::{self, Control, CustomerRequest, OpenDevice, OpenGroup, Report, State};
use crate::requests::{self, Requests};
use crate::settings::{ConfigError, invalid, read_setting};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PING: Duration = Duration::from_secs(30);
/// Waits between attempts to reach remotehub, growing up to the last one.
const BACKOFF: [u64; 5] = [1, 2, 5, 10, 30];
const CHUNK: usize = 64 * 1024;

#[derive(Clone)]
pub struct AgentSettings {
    /// `https://remotehub.example.com` (or `http://` in tests).
    pub url: Uri,
    pub token: SecretString,
    pub tls: Arc<ClientConfig>,
    /// Where the connector may connect to; empty allows everything.
    pub allow: Vec<Network>,
    /// How often it reports without a change: [`protocol::STATE_EVERY`],
    /// shorter in tests.
    pub report_every: Duration,
}

impl AgentSettings {
    pub fn from_env(lookup: &impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let setting = |name: &'static str| read_setting(lookup, name);
        let raw_url = setting("REMOTEHUB_URL")?.ok_or(ConfigError::Missing("REMOTEHUB_URL"))?;
        let url: Uri = raw_url
            .parse()
            .ok()
            .filter(|url: &Uri| {
                matches!(url.scheme_str(), Some("https" | "http")) && url.host().is_some()
            })
            .ok_or_else(|| invalid("REMOTEHUB_URL", &raw_url))?;
        let token = setting("REMOTEHUB_CONNECTOR_TOKEN")?
            .ok_or(ConfigError::Missing("REMOTEHUB_CONNECTOR_TOKEN"))?;
        let ca_file = setting("REMOTEHUB_CONNECTOR_CA_FILE")?;
        let tls = tls_config(ca_file.as_deref().map(Path::new))
            .map_err(|reason| invalid("REMOTEHUB_CONNECTOR_CA_FILE", &reason))?;
        let allow = setting("REMOTEHUB_CONNECTOR_ALLOW")?
            .map(|list| {
                list.split([',', ' '])
                    .filter(|entry| !entry.is_empty())
                    .map(|entry| {
                        entry
                            .parse()
                            .map_err(|()| invalid("REMOTEHUB_CONNECTOR_ALLOW", entry))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        Ok(AgentSettings {
            url,
            token: SecretString::from(token),
            tls,
            allow,
            report_every: protocol::STATE_EVERY,
        })
    }
}

/// The system's CAs, and the one in `ca_file` if given.
pub fn tls_config(ca_file: Option<&Path>) -> Result<Arc<ClientConfig>, String> {
    let mut roots = RootCertStore::empty();
    for certificate in rustls_native_certs::load_native_certs().certs {
        let _ = roots.add(certificate);
    }
    if let Some(file) = ca_file {
        for certificate in CertificateDer::pem_file_iter(file).map_err(|e| e.to_string())? {
            roots
                .add(certificate.map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        }
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Ok(Arc::new(
        ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

/// What the connector keeps while it runs, shared with the web interface.
#[derive(Clone)]
pub struct Site {
    pub gate: Arc<Gate>,
    pub journal: Arc<Journal>,
    pub connections: Arc<Connections>,
    /// remotehub's requests for access and the customer's answers (#181).
    pub requests: Arc<Requests>,
}

/// A connection the connector carries now.
#[derive(Clone)]
pub struct Running {
    pub target: String,
    pub user: Option<String>,
    /// The device's name in remotehub, as remotehub reports it.
    pub device: Option<String>,
    pub since: OffsetDateTime,
    /// Bytes to the device so far.
    pub sent: Arc<AtomicU64>,
    /// Bytes from the device so far.
    pub received: Arc<AtomicU64>,
    /// Where it went, for checking it again when the access changes.
    reached: Reached,
    /// Ends this connection alone, when its device closes (#180).
    ends: CancellationToken,
}

/// Where a connection went: the host as asked, the address it resolved to,
/// and the port.
#[derive(Clone)]
struct Reached {
    host: String,
    ip: IpAddr,
    port: u16,
}

/// The connections the connector carries now, for the web interface.
#[derive(Default)]
pub struct Connections(Mutex<HashMap<Uuid, Running>>);

impl Connections {
    /// Oldest first.
    pub fn list(&self) -> Vec<Running> {
        let mut running: Vec<Running> = self
            .0
            .lock()
            .expect("no panics while locked")
            .values()
            .cloned()
            .collect();
        running.sort_by_key(|r| r.since);
        running
    }
}

/// Runs until the process ends, or until the future is dropped: then the
/// control socket and every connection close with it.
pub async fn run(settings: AgentSettings, site: Site) {
    tokio::join!(
        supervise(&settings, &site),
        report(&settings, &site),
        site.gate.clone().follow(),
    );
}

/// Connected to remotehub while anything is open, the network or a device,
/// and not at all while everything is closed.
async fn supervise(settings: &AgentSettings, site: &Site) {
    let mut access = site.gate.subscribe();
    loop {
        until_open(&mut access).await;
        let ends = CancellationToken::new();
        // Closing ends the connections, which hold child tokens; so does
        // dropping this future.
        let _ends = ends.clone().drop_guard();
        tokio::select! {
            () = connected(settings, site, &ends) => {}
            () = while_open(&mut access, site) => {}
        }
        tracing::info!("access closed: disconnected from remotehub");
    }
}

fn open_now(access: &watch::Receiver<Snapshot>) -> bool {
    access.borrow().any_open(OffsetDateTime::now_utc())
}

async fn until_open(access: &mut watch::Receiver<Snapshot>) {
    while !open_now(access) {
        if access.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Returns when nothing is open any more. Before, on every change and every
/// time something runs out, ends the connections the access no longer lets
/// through: a device can close while the network stays closed and another
/// device stays open.
async fn while_open(access: &mut watch::Receiver<Snapshot>, site: &Site) {
    loop {
        let snapshot = access.borrow_and_update().clone();
        let now = OffsetDateTime::now_utc();
        if !snapshot.any_open(now) {
            return;
        }
        for running in site.connections.list() {
            let Reached { host, ip, port } = &running.reached;
            if !permitted(&snapshot, host, *ip, *port).await {
                tracing::info!(target = %running.target, "no longer open: disconnecting");
                running.ends.cancel();
            }
        }
        let remaining = snapshot
            .next_end(now)
            .map(|at| Duration::try_from(at - OffsetDateTime::now_utc()).unwrap_or(Duration::ZERO));
        tokio::select! {
            changed = access.changed() => {
                if changed.is_err() {
                    std::future::pending::<()>().await;
                }
            }
            () = sleep_or_forever(remaining) => {}
        }
    }
}

/// Whether the access lets a connection to `host`, resolved to `ip`, on
/// `port` through: the whole network is open, or an open device covers the
/// address and the port. A device's host name is resolved here, in the
/// customer's network.
async fn permitted(snapshot: &Snapshot, host: &str, ip: IpAddr, port: u16) -> bool {
    open_until(snapshot, host, ip, port).await.is_some()
}

/// Until when the access lets such a connection through: the latest end of
/// the network and the devices that cover it, `Some(None)` without end, none
/// if it does not.
async fn open_until(
    snapshot: &Snapshot,
    host: &str,
    ip: IpAddr,
    port: u16,
) -> Option<Option<OffsetDateTime>> {
    let now = OffsetDateTime::now_utc();
    let mut until = snapshot.network_until(now);
    for device in snapshot.open_devices(now) {
        if !device.ports.iter().any(|ports| ports.contains(port)) {
            continue;
        }
        let resolved = match &device.address {
            Address::Host(name) => resolve(name).await,
            Address::Range(_) => Vec::new(),
        };
        if device.address.covers(host, ip, &resolved) {
            until = later(until, snapshot.device_until(device, now));
        }
    }
    // An approved request opens its targets, each on its one port (#181).
    for (_, grant) in snapshot.open_requests(now) {
        for device in grant.devices().filter(|d| d.ports[0].contains(port)) {
            let resolved = match &device.address {
                Address::Host(name) => resolve(name).await,
                Address::Range(_) => Vec::new(),
            };
            if device.address.covers(host, ip, &resolved) {
                until = later(until, Some(grant.stored.access.closes_at()));
            }
        }
    }
    until
}

async fn resolve(name: &str) -> Vec<IpAddr> {
    match tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::lookup_host((name, 0))).await {
        Ok(Ok(addresses)) => addresses.map(|a| a.ip()).collect(),
        _ => Vec::new(),
    }
}

async fn sleep_or_forever(duration: Option<Duration>) {
    match duration {
        Some(duration) => tokio::time::sleep(duration).await,
        None => std::future::pending().await,
    }
}

/// Keeps a control connection to remotehub, reconnecting whenever it
/// closes, until the future is dropped.
async fn connected(settings: &AgentSettings, site: &Site, ends: &CancellationToken) {
    let mut attempt = 0;
    loop {
        match control(settings, site, ends).await {
            Ok(()) => {
                tracing::warn!("remotehub closed the control connection");
                attempt = 0;
            }
            Err(error) => tracing::warn!(%error, "cannot reach remotehub"),
        }
        let wait = BACKOFF[attempt.min(BACKOFF.len() - 1)];
        attempt += 1;
        tokio::time::sleep(Duration::from_secs(wait)).await;
    }
}

/// One control connection, until it closes.
async fn control(
    settings: &AgentSettings,
    site: &Site,
    ends: &CancellationToken,
) -> Result<(), String> {
    let socket = open_socket(settings, "/api/connectors/control").await?;
    tracing::info!(url = %settings.url, "connected to remotehub");
    let (mut to_remotehub, mut from_remotehub) = socket.split();
    let (reports, mut reported) = mpsc::channel::<Report>(64);
    let mut ping = tokio::time::interval(PING);
    loop {
        tokio::select! {
            message = from_remotehub.next() => match message {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<Control>(&text) {
                    Ok(Control::Open { id, target, user, device }) => {
                        let request = Request { id, target, user, device };
                        tokio::spawn(stream(
                            settings.clone(),
                            site.clone(),
                            request,
                            reports.clone(),
                            ends.child_token(),
                        ));
                    }
                    Ok(Control::Check { id, target }) => {
                        let (allow, gate, reports) =
                            (settings.allow.clone(), site.gate.clone(), reports.clone());
                        tokio::spawn(async move {
                            let checked = match resolve_target(&allow, &gate.current(), &target).await {
                                Ok(resolved) => Report::Checked {
                                    id,
                                    open: true,
                                    until: resolved.until.and_then(|at| at.format(&Rfc3339).ok()),
                                },
                                Err(_) => Report::Checked { id, open: false, until: None },
                            };
                            let _ = reports.send(checked).await;
                        });
                    }
                    Err(error) => tracing::warn!(%error, "unknown message from remotehub"),
                },
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Err(error)) => return Err(error.to_string()),
                Some(Ok(_)) => {}
            },
            Some(report) = reported.recv() => {
                let text = serde_json::to_string(&report).expect("reports serialize");
                to_remotehub.send(Message::Text(text.into())).await.map_err(|e| e.to_string())?;
            }
            _ = ping.tick() => {
                to_remotehub.send(Message::Ping(Vec::new().into())).await.map_err(|e| e.to_string())?;
            }
        }
    }
}

/// What remotehub asked for in an `open`.
struct Request {
    id: Uuid,
    target: String,
    user: Option<String>,
    device: Option<String>,
}

/// Connects to the target and carries the connection over a new stream to
/// remotehub until either side ends it or `ends` is cancelled; reports on
/// the control socket if the target is out of reach.
async fn stream(
    settings: AgentSettings,
    site: Site,
    request: Request,
    reports: mpsc::Sender<Report>,
    ends: CancellationToken,
) {
    let Request {
        id,
        target,
        user,
        device,
    } = request;
    let name = device.as_deref().unwrap_or_default();
    let (connection, reached) = match reach(&settings.allow, &site.gate.current(), &target).await {
        Ok(reached) => reached,
        Err(Refusal { reason, not_open }) => {
            tracing::warn!(device = name, %target, reason, "cannot reach the target");
            site.journal.append(Event::ConnectionRefused {
                target: target.clone(),
                device: device.clone(),
                user,
                reason: reason.clone(),
            });
            let _ = reports
                .send(Report::Failed {
                    id,
                    reason,
                    not_open,
                })
                .await;
            return;
        }
    };
    let socket = match open_socket(&settings, &format!("/api/connectors/streams/{id}")).await {
        Ok(socket) => socket,
        Err(error) => {
            tracing::warn!(device = name, %target, %error, "cannot open the stream to remotehub");
            return;
        }
    };
    let running = Running {
        target: target.clone(),
        user: user.clone(),
        device: device.clone(),
        since: OffsetDateTime::now_utc(),
        sent: Arc::default(),
        received: Arc::default(),
        reached,
        ends: ends.clone(),
    };
    site.connections
        .0
        .lock()
        .expect("no panics while locked")
        .insert(id, running.clone());
    site.journal.append(Event::ConnectionStarted {
        id,
        target: target.clone(),
        device: device.clone(),
        user: user.clone(),
    });
    tokio::select! {
        () = carry(connection, socket, &running) => {}
        () = ends.cancelled() => {}
    }
    site.connections
        .0
        .lock()
        .expect("no panics while locked")
        .remove(&id);
    site.journal.append(Event::ConnectionEnded {
        id,
        target,
        device,
        user,
        seconds: (OffsetDateTime::now_utc() - running.since)
            .whole_seconds()
            .unsigned_abs(),
        sent: running.sent.load(Ordering::Relaxed),
        received: running.received.load(Ordering::Relaxed),
    });
}

/// Why the connector does not connect; `not_open` if the customer has not
/// opened the target.
struct Refusal {
    reason: String,
    not_open: bool,
}

impl From<String> for Refusal {
    fn from(reason: String) -> Self {
        Refusal {
            reason,
            not_open: false,
        }
    }
}

/// Where the access lets a connection to a target through.
struct Resolved {
    host: String,
    port: u16,
    /// What the target resolves to, `allow` covers and the access lets
    /// through.
    addresses: Vec<SocketAddr>,
    /// The latest end of the access to them; none without end.
    until: Option<OffsetDateTime>,
}

/// The addresses `target` (`host:port`) resolves to that `allow` covers and
/// the access lets through, with the host and port as asked.
async fn resolve_target(
    allow: &[Network],
    access: &Snapshot,
    target: &str,
) -> Result<Resolved, Refusal> {
    let (host, port) = target
        .rsplit_once(':')
        .and_then(|(host, port)| Some((host.trim_matches(['[', ']']), port.parse().ok()?)))
        .ok_or_else(|| format!("not host:port: {target}"))?;
    let addresses: Vec<SocketAddr> =
        tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::lookup_host(target))
            .await
            .map_err(|_| "name lookup timed out".to_owned())?
            .map_err(|e| format!("name lookup: {e}"))?
            .collect();
    let allowed: Vec<SocketAddr> = addresses
        .into_iter()
        .filter(|address| allow.is_empty() || allow.iter().any(|n| n.contains(address.ip())))
        .collect();
    if allowed.is_empty() {
        return Err("not allowed".to_owned().into());
    }
    let mut open = Vec::new();
    let mut until = None;
    for address in allowed {
        if let Some(end) = open_until(access, host, address.ip(), port).await {
            open.push(address);
            until = later(until, Some(end));
        }
    }
    match until {
        Some(until) => Ok(Resolved {
            host: host.to_owned(),
            port,
            addresses: open,
            until,
        }),
        None => Err(Refusal {
            reason: "not opened".to_owned(),
            not_open: true,
        }),
    }
}

/// A TCP connection to `target` (`host:port`), to an address `allow` covers
/// and `access` lets through.
async fn reach(
    allow: &[Network],
    access: &Snapshot,
    target: &str,
) -> Result<(TcpStream, Reached), Refusal> {
    let Resolved {
        host,
        port,
        addresses,
        ..
    } = resolve_target(allow, access, target).await?;
    let connection = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addresses[..]))
        .await
        .map_err(|_| "timed out".to_owned())?
        .map_err(|e| e.to_string())?;
    let ip = connection.peer_addr().map_err(|e| e.to_string())?.ip();
    Ok((connection, Reached { host, ip, port }))
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// A connection to remotehub, over TLS for `https`.
async fn connect(settings: &AgentSettings) -> Result<Box<dyn Io>, String> {
    let tls = settings.url.scheme_str() == Some("https");
    let host = settings.url.host().unwrap_or_default().to_owned();
    let port = settings
        .url
        .port_u16()
        .unwrap_or(if tls { 443 } else { 80 });
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host.as_str(), port)))
        .await
        .map_err(|_| "timed out".to_owned())?
        .map_err(|e| e.to_string())?;
    if !tls {
        return Ok(Box::new(tcp));
    }
    let name = ServerName::try_from(host).map_err(|e| e.to_string())?;
    let connector = tokio_rustls::TlsConnector::from(settings.tls.clone());
    Ok(Box::new(
        connector
            .connect(name, tcp)
            .await
            .map_err(|e| e.to_string())?,
    ))
}

fn bearer(settings: &AgentSettings) -> Result<HeaderValue, String> {
    format!("Bearer {}", settings.token.expose_secret())
        .parse()
        .map_err(|_| "the token is not a valid header value".to_owned())
}

/// A WebSocket to `path` on remotehub, signed in with the token.
async fn open_socket(
    settings: &AgentSettings,
    path: &str,
) -> Result<WebSocketStream<Box<dyn Io>>, String> {
    let base = settings.url.to_string();
    let url = format!(
        "{}{path}",
        base.trim_end_matches('/')
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1)
    );
    let mut request = url.into_client_request().map_err(|e| e.to_string())?;
    let headers = request.headers_mut();
    headers.insert("authorization", bearer(settings)?);
    headers.insert(protocol::HEADER, protocol::VERSION.into());
    let io = connect(settings).await?;
    let (socket, _) = tokio_tungstenite::client_async(request, io)
        .await
        .map_err(|error| match &error {
            tungstenite::Error::Http(response) => {
                refusal(response.status(), response.headers().get(protocol::HEADER))
            }
            _ => error.to_string(),
        })?;
    Ok(socket)
}

/// Why remotehub refused a request, in words an administrator can act on.
/// `theirs` is the protocol version remotehub named in its answer.
fn refusal(status: StatusCode, theirs: Option<&HeaderValue>) -> String {
    match status {
        StatusCode::UPGRADE_REQUIRED => {
            let theirs = theirs
                .and_then(|value| value.to_str().ok())
                .unwrap_or("unknown");
            format!(
                "remotehub speaks connector protocol version {theirs}, this connector {}: \
                 run the connector of remotehub's release",
                protocol::VERSION
            )
        }
        StatusCode::UNAUTHORIZED => "remotehub does not know the token".to_owned(),
        status => format!("remotehub answered {status}"),
    }
}

/// Reports the access to remotehub on every change, every answer to a
/// request and every [`AgentSettings::report_every`], until the future is
/// dropped; takes the requests remotehub lists in return (#181).
async fn report(settings: &AgentSettings, site: &Site) {
    let mut access = site.gate.subscribe();
    loop {
        let mut state = state(&access.borrow_and_update(), OffsetDateTime::now_utc());
        state.answers = site.requests.answers();
        match post_state(settings, &state).await {
            Ok(requests) => {
                for request in site.requests.listed(requests) {
                    access::received(&site.journal, &request);
                }
            }
            Err(error) => tracing::warn!(%error, "cannot report the access to remotehub"),
        }
        tokio::select! {
            changed = access.changed() => {
                if changed.is_err() {
                    std::future::pending::<()>().await;
                }
            }
            () = site.requests.answered.notified() => {}
            () = tokio::time::sleep(settings.report_every) => {}
        }
    }
}

/// What the connector reports about `snapshot` at `now`: the whole network,
/// the open groups and devices of the list and the targets of approved
/// requests, at most [`protocol::MAX_LISTED`] of each.
fn state(snapshot: &Snapshot, now: OffsetDateTime) -> State {
    let text = |at: Option<OffsetDateTime>| at.and_then(|at| at.format(&Rfc3339).ok());
    let open = snapshot.network_open(now);
    let approved = snapshot
        .open_requests(now)
        .into_iter()
        .flat_map(|(_, grant)| {
            grant.targets.iter().map(|target| OpenDevice {
                name: target.name.clone(),
                address: target.host.clone(),
                ports: target.port.to_string(),
                until: text(grant.stored.access.closes_at()),
            })
        });
    State {
        open,
        until: snapshot.network_until(now).and_then(text),
        partly: !open && snapshot.any_open(now),
        groups: snapshot
            .open_groups(now)
            .into_iter()
            .take(protocol::MAX_LISTED)
            .map(|(name, until)| OpenGroup {
                name: name.to_owned(),
                until: text(until),
            })
            .collect(),
        devices: snapshot
            .open_devices(now)
            .into_iter()
            .map(|device| OpenDevice {
                name: device.name.clone(),
                address: device.address.to_string(),
                ports: inventory::ports_text(&device.ports),
                until: text(snapshot.device_until(device, now).flatten()),
            })
            .chain(approved)
            .take(protocol::MAX_LISTED)
            .collect(),
        answers: Vec::new(),
    }
}

/// Posts the state; returns the requests remotehub lists in its answer, or
/// why there are none to take.
async fn post_state(
    settings: &AgentSettings,
    state: &State,
) -> Result<Vec<CustomerRequest>, String> {
    let io = connect(settings).await?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(io))
        .await
        .map_err(|e| e.to_string())?;
    tokio::spawn(connection);
    let authority = settings
        .url
        .authority()
        .map(|a| a.as_str().to_owned())
        .unwrap_or_default();
    let request = hyper::Request::post(protocol::STATE_PATH)
        .header("host", authority)
        .header("authorization", bearer(settings)?)
        .header(protocol::HEADER, protocol::VERSION)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(
            serde_json::to_vec(state).expect("states serialize"),
        )))
        .map_err(|e| e.to_string())?;
    let response = tokio::time::timeout(CONNECT_TIMEOUT, sender.send_request(request))
        .await
        .map_err(|_| "timed out".to_owned())?
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(refusal(
            response.status(),
            response.headers().get(protocol::HEADER),
        ));
    }
    // One byte more than allowed tells a long answer from a full one.
    let limited = Limited::new(response.into_body(), protocol::MAX_PENDING_BYTES + 1);
    let body = tokio::time::timeout(CONNECT_TIMEOUT, limited.collect())
        .await
        .map_err(|_| "timed out".to_owned())?
        .map_err(|_| {
            format!(
                "the answer is longer than {} bytes",
                protocol::MAX_PENDING_BYTES
            )
        })?
        .to_bytes();
    requests::parse(&body).map_err(|error| {
        // Nothing of it is taken; the requests listed before stay.
        format!("dropped remotehub's answer: {error}")
    })
}

/// Copies bytes both ways between the device and remotehub until both sides
/// have finished, counting them in `running`; the server's
/// `connectors::carry` is the other end.
async fn carry(device: TcpStream, socket: WebSocketStream<Box<dyn Io>>, running: &Running) {
    let (mut to_remotehub, mut from_remotehub) = socket.split();
    let (mut read, mut write) = device.into_split();
    let up = async {
        let mut buffer = vec![0u8; CHUNK];
        loop {
            match read.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    running.received.fetch_add(n as u64, Ordering::Relaxed);
                    let data = buffer[..n].to_vec();
                    if to_remotehub
                        .send(Message::Binary(data.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
        let _ = to_remotehub.send(Message::Close(None)).await;
    };
    let down = async {
        while let Some(Ok(message)) = from_remotehub.next().await {
            match message {
                Message::Binary(data) => {
                    running.sent.fetch_add(data.len() as u64, Ordering::Relaxed);
                    if write.write_all(&data).await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        let _ = write.shutdown().await;
    };
    tokio::join!(up, down);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::{Access, AccessFile, Stored};
    use crate::inventory::{Device, Ports};

    #[tokio::test]
    async fn only_allowed_addresses_are_reached() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = listener.local_addr().unwrap().to_string();
        let open = Snapshot {
            access: AccessFile {
                network: Stored {
                    access: Access::Open { until: None },
                    ..Stored::default()
                },
                ..AccessFile::default()
            },
            ..Snapshot::default()
        };
        let refused = |result: Result<_, Refusal>| result.err().map(|r| r.reason);
        assert!(reach(&[], &open, &target).await.is_ok());
        assert!(
            reach(&["127.0.0.0/8".parse().unwrap()], &open, &target)
                .await
                .is_ok()
        );
        assert_eq!(
            refused(reach(&["10.0.0.0/8".parse().unwrap()], &open, &target).await).as_deref(),
            Some("not allowed")
        );
    }

    /// With the network closed, only a port of an open device goes through,
    /// whether the device is open itself or through its group (#180).
    #[tokio::test]
    async fn a_closed_network_lets_through_only_open_devices() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let target = format!("127.0.0.1:{port}");
        let device = |name: &str, address: &str, ports: &str, groups: &[&str]| Device {
            name: name.into(),
            address: address.parse().unwrap(),
            ports: Ports::parse_list(ports).unwrap(),
            groups: groups.iter().map(|g| (*g).to_owned()).collect(),
        };
        let open = Stored {
            access: Access::Open { until: None },
            ..Stored::default()
        };
        let mut snapshot = Snapshot::default();
        snapshot.inventory.groups = vec!["ERP".into()];
        snapshot.inventory.devices = vec![
            device("sql", "127.0.0.1", &port.to_string(), &["ERP"]),
            device("other port", "127.0.0.1", "1", &[]),
            device("other host", "10.0.0.1", &port.to_string(), &[]),
        ];
        let not_open = |result: Result<_, Refusal>| result.err().is_some_and(|r| r.not_open);

        assert!(not_open(reach(&[], &snapshot, &target).await));
        snapshot
            .access
            .devices
            .insert("other port".into(), open.clone());
        snapshot
            .access
            .devices
            .insert("other host".into(), open.clone());
        assert!(not_open(reach(&[], &snapshot, &target).await));
        snapshot.access.groups.insert("ERP".into(), open.clone());
        assert!(reach(&[], &snapshot, &target).await.is_ok());
        snapshot.access.groups.clear();
        snapshot.access.devices.insert("sql".into(), open);
        assert!(reach(&[], &snapshot, &target).await.is_ok());
        // The outer limit still holds.
        assert!(!not_open(
            reach(&["10.0.0.0/8".parse().unwrap()], &snapshot, &target).await
        ));
    }

    /// An approved request opens its targets on their ports and nothing
    /// else, until its end (#181).
    #[tokio::test]
    async fn an_approved_request_opens_only_its_targets() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let other = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let other_port = other.local_addr().unwrap().port();
        let now = OffsetDateTime::now_utc();
        let grant = |until: OffsetDateTime| crate::access::Grant {
            requester: "alice".into(),
            reason: "ERP update".into(),
            targets: vec![protocol::RequestTarget {
                name: "sql".into(),
                host: "127.0.0.1".into(),
                port,
            }],
            stored: Stored {
                access: Access::Open { until: Some(until) },
                ..Stored::default()
            },
        };
        let mut snapshot = Snapshot::default();
        snapshot
            .access
            .requests
            .insert(Uuid::nil(), grant(now + time::Duration::hours(1)));
        assert!(
            reach(&[], &snapshot, &format!("127.0.0.1:{port}"))
                .await
                .is_ok()
        );
        let refused = reach(&[], &snapshot, &format!("127.0.0.1:{other_port}")).await;
        assert!(refused.err().is_some_and(|r| r.not_open));
        assert_eq!(state(&snapshot, now).devices[0].name, "sql");
        assert!(state(&snapshot, now).partly);

        snapshot
            .access
            .requests
            .insert(Uuid::nil(), grant(now - time::Duration::minutes(1)));
        assert!(!snapshot.any_open(now));
        let refused = reach(&[], &snapshot, &format!("127.0.0.1:{port}")).await;
        assert!(refused.err().is_some_and(|r| r.not_open));
    }

    /// The report and the answer to a check name the latest end of what
    /// opens a device: the device itself or one of its groups.
    #[tokio::test]
    async fn a_device_is_open_until_the_latest_end() {
        let now = OffsetDateTime::now_utc().replace_nanosecond(0).unwrap();
        let until = |hours: i64| Stored {
            access: Access::Open {
                until: Some(now + time::Duration::hours(hours)),
            },
            ..Stored::default()
        };
        let mut snapshot = Snapshot::default();
        snapshot.inventory.groups = vec!["ERP".into(), "closed".into()];
        snapshot.inventory.devices = vec![Device {
            name: "sql".into(),
            address: "127.0.0.1".parse().unwrap(),
            ports: Ports::parse_list("1433").unwrap(),
            groups: vec!["ERP".into(), "closed".into()],
        }];
        snapshot.access.devices.insert("sql".into(), until(1));
        snapshot.access.groups.insert("ERP".into(), until(3));
        let three = (now + time::Duration::hours(3)).format(&Rfc3339).unwrap();

        let reported = state(&snapshot, now);
        assert!(!reported.open && reported.partly);
        assert_eq!(
            reported.groups,
            vec![OpenGroup {
                name: "ERP".into(),
                until: Some(three.clone()),
            }]
        );
        assert_eq!(
            reported.devices,
            vec![OpenDevice {
                name: "sql".into(),
                address: "127.0.0.1".into(),
                ports: "1433".into(),
                until: Some(three.clone()),
            }]
        );
        let resolved = resolve_target(&[], &snapshot, "127.0.0.1:1433").await;
        assert_eq!(
            resolved.ok().and_then(|r| r.until),
            Some(now + time::Duration::hours(3))
        );

        // Without end wins over any point in time.
        snapshot.access.devices.insert(
            "sql".into(),
            Stored {
                access: Access::Open { until: None },
                ..Stored::default()
            },
        );
        assert_eq!(state(&snapshot, now).devices[0].until, None);
        let resolved = resolve_target(&[], &snapshot, "127.0.0.1:1433").await;
        assert!(resolved.is_ok_and(|r| r.until.is_none()));
    }

    #[test]
    fn a_refusal_says_which_protocol_remotehub_wants() {
        let reason = refusal(
            StatusCode::UPGRADE_REQUIRED,
            Some(&HeaderValue::from_static("2")),
        );
        assert!(
            reason.starts_with("remotehub speaks connector protocol version 2, this connector 1"),
            "{reason}"
        );
    }
}
