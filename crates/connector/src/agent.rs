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
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use http_body_util::Full;
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

use crate::access::{Gate, Stored};
use crate::journal::{Event, Journal};
use crate::network::Network;
use crate::protocol::{self, Control, Report, State};
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
}

/// A connection the connector carries now.
#[derive(Clone)]
pub struct Running {
    pub target: String,
    pub user: Option<String>,
    pub since: OffsetDateTime,
    /// Bytes to the device so far.
    pub sent: Arc<AtomicU64>,
    /// Bytes from the device so far.
    pub received: Arc<AtomicU64>,
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
        report(&settings, &site.gate),
        site.gate.clone().follow(),
    );
}

/// Connected to remotehub while the access is open, and not at all while it
/// is closed.
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
            () = until_closed(&mut access) => {}
        }
        tracing::info!("access closed: disconnected from remotehub");
    }
}

fn open_now(access: &watch::Receiver<Stored>) -> bool {
    access.borrow().access.is_open(OffsetDateTime::now_utc())
}

async fn until_open(access: &mut watch::Receiver<Stored>) {
    while !open_now(access) {
        if access.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Returns when the access closes: by a change, or when its time runs out.
async fn until_closed(access: &mut watch::Receiver<Stored>) {
    loop {
        let closes_at = access.borrow_and_update().access.closes_at();
        if !open_now(access) {
            return;
        }
        let remaining = closes_at
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
                    Ok(Control::Open { id, target, user }) => {
                        let request = Request { id, target, user };
                        tokio::spawn(stream(
                            settings.clone(),
                            site.clone(),
                            request,
                            reports.clone(),
                            ends.child_token(),
                        ));
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
    let Request { id, target, user } = request;
    let device = match reach(&settings.allow, &target).await {
        Ok(device) => device,
        Err(reason) => {
            tracing::warn!(%target, reason, "cannot reach the target");
            site.journal.append(Event::ConnectionRefused {
                target: target.clone(),
                user,
                reason: reason.clone(),
            });
            let _ = reports.send(Report::Failed { id, reason }).await;
            return;
        }
    };
    let socket = match open_socket(&settings, &format!("/api/connectors/streams/{id}")).await {
        Ok(socket) => socket,
        Err(error) => {
            tracing::warn!(%target, %error, "cannot open the stream to remotehub");
            return;
        }
    };
    let running = Running {
        target: target.clone(),
        user: user.clone(),
        since: OffsetDateTime::now_utc(),
        sent: Arc::default(),
        received: Arc::default(),
    };
    site.connections
        .0
        .lock()
        .expect("no panics while locked")
        .insert(id, running.clone());
    site.journal.append(Event::ConnectionStarted {
        id,
        target: target.clone(),
        user: user.clone(),
    });
    tokio::select! {
        () = carry(device, socket, &running) => {}
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
        user,
        seconds: (OffsetDateTime::now_utc() - running.since)
            .whole_seconds()
            .unsigned_abs(),
        sent: running.sent.load(Ordering::Relaxed),
        received: running.received.load(Ordering::Relaxed),
    });
}

/// A TCP connection to `target` (`host:port`), to an address `allow` covers.
async fn reach(allow: &[Network], target: &str) -> Result<TcpStream, String> {
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
        return Err("not allowed".to_owned());
    }
    tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&allowed[..]))
        .await
        .map_err(|_| "timed out".to_owned())?
        .map_err(|e| e.to_string())
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

/// Reports the access to remotehub on every change and every
/// [`protocol::STATE_EVERY`], until the future is dropped.
async fn report(settings: &AgentSettings, gate: &Gate) {
    let mut access = gate.subscribe();
    loop {
        let stored = access.borrow_and_update().access;
        let open = stored.is_open(OffsetDateTime::now_utc());
        let state = State {
            open,
            until: stored
                .closes_at()
                .filter(|_| open)
                .and_then(|at| at.format(&Rfc3339).ok()),
        };
        if let Err(error) = post_state(settings, &state).await {
            tracing::warn!(%error, "cannot report the access to remotehub");
        }
        tokio::select! {
            changed = access.changed() => {
                if changed.is_err() {
                    std::future::pending::<()>().await;
                }
            }
            () = tokio::time::sleep(protocol::STATE_EVERY) => {}
        }
    }
}

async fn post_state(settings: &AgentSettings, state: &State) -> Result<(), String> {
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
    if response.status().is_success() {
        Ok(())
    } else {
        Err(refusal(
            response.status(),
            response.headers().get(protocol::HEADER),
        ))
    }
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

    #[tokio::test]
    async fn only_allowed_addresses_are_reached() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = listener.local_addr().unwrap().to_string();
        assert!(reach(&[], &target).await.is_ok());
        assert!(
            reach(&["127.0.0.0/8".parse().unwrap()], &target)
                .await
                .is_ok()
        );
        assert_eq!(
            reach(&["10.0.0.0/8".parse().unwrap()], &target)
                .await
                .err()
                .as_deref(),
            Some("not allowed")
        );
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
