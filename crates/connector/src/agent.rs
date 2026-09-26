//! The connector: keeps a control WebSocket open to remotehub, and connects
//! to devices in its network when remotehub asks. Each such connection gets
//! a WebSocket of its own to remotehub for its bytes.
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

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::{CertificateDer, ServerName, pem::PemObject};
use secrecy::{ExposeSecret, SecretString};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{StatusCode, Uri};
use tokio_tungstenite::tungstenite::{self, Message};
use uuid::Uuid;

use crate::network::Network;
use crate::protocol::{self, Control, Report};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PING: Duration = Duration::from_secs(30);
/// Waits between attempts to reach remotehub, growing up to the last one.
const BACKOFF: [u64; 5] = [1, 2, 5, 10, 30];
const CHUNK: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0} is not set (also possible as {0}_FILE)")]
    Missing(&'static str),
    #[error("{name}_FILE: cannot read {path}: {reason}")]
    File {
        name: &'static str,
        path: String,
        reason: String,
    },
    #[error("{name}: invalid value {value:?}")]
    Invalid { name: &'static str, value: String },
}

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
    pub fn from_env() -> Result<Self, ConfigError> {
        let lookup = |name: &str| std::env::var(name).ok();
        let setting = |name: &'static str| read_setting(&lookup, name);
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

/// `NAME_FILE` (trimmed file content) takes precedence over `NAME`; empty
/// values count as unset. The same rule as the server's settings.
fn read_setting(
    lookup: &impl Fn(&str) -> Option<String>,
    name: &'static str,
) -> Result<Option<String>, ConfigError> {
    if let Some(path) = lookup(&format!("{name}_FILE")).filter(|p| !p.is_empty()) {
        let content = std::fs::read_to_string(&path).map_err(|e| ConfigError::File {
            name,
            path: path.clone(),
            reason: e.to_string(),
        })?;
        return Ok(Some(content.trim().to_owned()).filter(|v| !v.is_empty()));
    }
    Ok(lookup(name).filter(|v| !v.is_empty()))
}

fn invalid(name: &'static str, value: &str) -> ConfigError {
    ConfigError::Invalid {
        name,
        value: value.to_owned(),
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

/// Runs until the process ends: reconnects whenever the control socket
/// closes.
pub async fn run(settings: AgentSettings) {
    let mut attempt = 0;
    loop {
        match control(&settings).await {
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
async fn control(settings: &AgentSettings) -> Result<(), String> {
    let socket = open_socket(settings, "/api/connectors/control").await?;
    tracing::info!(url = %settings.url, "connected to remotehub");
    let (mut to_remotehub, mut from_remotehub) = socket.split();
    let (reports, mut reported) = mpsc::channel::<Report>(64);
    let mut ping = tokio::time::interval(PING);
    loop {
        tokio::select! {
            message = from_remotehub.next() => match message {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<Control>(&text) {
                    Ok(Control::Open { id, target }) => {
                        tokio::spawn(stream(settings.clone(), id, target, reports.clone()));
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

/// Connects to `target` and carries the connection over a new stream to
/// remotehub; reports on the control socket if the target is out of reach.
async fn stream(settings: AgentSettings, id: Uuid, target: String, reports: mpsc::Sender<Report>) {
    let device = match reach(&settings.allow, &target).await {
        Ok(device) => device,
        Err(reason) => {
            tracing::warn!(%target, reason, "cannot reach the target");
            let _ = reports.send(Report::Failed { id, reason }).await;
            return;
        }
    };
    match open_socket(&settings, &format!("/api/connectors/streams/{id}")).await {
        Ok(socket) => carry(device, socket).await,
        Err(error) => tracing::warn!(%target, %error, "cannot open the stream to remotehub"),
    }
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

/// A WebSocket to `path` on remotehub, signed in with the token.
async fn open_socket(
    settings: &AgentSettings,
    path: &str,
) -> Result<WebSocketStream<Box<dyn Io>>, String> {
    let tls = settings.url.scheme_str() == Some("https");
    let host = settings.url.host().unwrap_or_default().to_owned();
    let port = settings
        .url
        .port_u16()
        .unwrap_or(if tls { 443 } else { 80 });
    let base = settings.url.to_string();
    let url = format!(
        "{}{path}",
        base.trim_end_matches('/')
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1)
    );
    let mut request = url.into_client_request().map_err(|e| e.to_string())?;
    let headers = request.headers_mut();
    headers.insert(
        "authorization",
        format!("Bearer {}", settings.token.expose_secret())
            .parse()
            .map_err(|_| "the token is not a valid header value".to_owned())?,
    );
    headers.insert(protocol::HEADER, protocol::VERSION.into());
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host.as_str(), port)))
        .await
        .map_err(|_| "timed out".to_owned())?
        .map_err(|e| e.to_string())?;
    let io: Box<dyn Io> = if tls {
        let name = ServerName::try_from(host.clone()).map_err(|e| e.to_string())?;
        let connector = tokio_rustls::TlsConnector::from(settings.tls.clone());
        Box::new(
            connector
                .connect(name, tcp)
                .await
                .map_err(|e| e.to_string())?,
        )
    } else {
        Box::new(tcp)
    };
    let (socket, _) = tokio_tungstenite::client_async(request, io)
        .await
        .map_err(refusal)?;
    Ok(socket)
}

/// Why remotehub did not take the WebSocket, in words an administrator can
/// act on.
fn refusal(error: tungstenite::Error) -> String {
    let tungstenite::Error::Http(response) = &error else {
        return error.to_string();
    };
    match response.status() {
        StatusCode::UPGRADE_REQUIRED => {
            let theirs = response
                .headers()
                .get(protocol::HEADER)
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

/// Copies bytes both ways between the device and remotehub until both sides
/// have finished; the server's `connectors::carry` is the other end.
async fn carry(device: TcpStream, socket: WebSocketStream<Box<dyn Io>>) {
    let (mut to_remotehub, mut from_remotehub) = socket.split();
    let (mut read, mut write) = device.into_split();
    let up = async {
        let mut buffer = vec![0u8; CHUNK];
        loop {
            match read.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
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
        let response = tungstenite::http::Response::builder()
            .status(426)
            .header(protocol::HEADER, "2")
            .body(None)
            .unwrap();
        let reason = refusal(tungstenite::Error::Http(response.into()));
        assert!(
            reason.starts_with("remotehub speaks connector protocol version 2, this connector 1"),
            "{reason}"
        );
    }
}
