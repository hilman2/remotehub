//! remotehub's side of the agent: opens a session and holds it.

use std::time::Duration;

use serde_json::json;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use zeroize::Zeroizing;

use crate::protocol::{MAX_LINE, Reply};

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("not reachable: {0}")]
    Unreachable(std::io::Error),
    #[error("no answer in time")]
    Timeout,
    #[error("the agent refused: {0}")]
    Failed(String),
    #[error("unexpected answer")]
    Protocol,
}

/// What to open; see [`crate::protocol::Open`].
pub struct Request<'a> {
    pub host: &'a str,
    pub port: u16,
    pub spki: &'a str,
    pub width: u32,
    pub height: u32,
    pub timezone: Option<&'a str>,
    /// User name and password to sign in with.
    pub login: Option<(&'a str, &'a str)>,
}

/// A running browser. Dropping it ends the session on the agent.
pub struct Session {
    /// Where guacd reaches the display: the agent's host.
    pub vnc_host: String,
    pub vnc_port: u16,
    pub vnc_password: Zeroizing<String>,
    read: BufReader<OwnedReadHalf>,
    _write: OwnedWriteHalf,
}

/// Asks the agent at `agent` (`host:port`) for a browser on the device.
pub async fn open(
    agent: &str,
    request: &Request<'_>,
    timeout: Duration,
) -> Result<Session, BrowserError> {
    tokio::time::timeout(timeout, start(agent, request))
        .await
        .map_err(|_| BrowserError::Timeout)?
}

async fn start(agent: &str, request: &Request<'_>) -> Result<Session, BrowserError> {
    let stream = TcpStream::connect(agent)
        .await
        .map_err(BrowserError::Unreachable)?;
    let (read, mut write) = stream.into_split();
    let login = request
        .login
        .map(|(username, password)| json!({ "username": username, "password": password }));
    let mut line = Zeroizing::new(
        json!({
            "host": request.host, "port": request.port, "spki": request.spki,
            "width": request.width, "height": request.height,
            "timezone": request.timezone, "login": login,
        })
        .to_string(),
    );
    drop(login);
    line.push('\n');
    write
        .write_all(line.as_bytes())
        .await
        .map_err(BrowserError::Unreachable)?;
    let mut read = BufReader::new(read);
    match next(&mut read).await {
        Some(Reply::Ready {
            vnc_port,
            vnc_password,
        }) => Ok(Session {
            vnc_host: host_of(agent).to_owned(),
            vnc_port,
            vnc_password: Zeroizing::new(vnc_password),
            read,
            _write: write,
        }),
        Some(Reply::Failed { reason }) => Err(BrowserError::Failed(reason)),
        _ => Err(BrowserError::Protocol),
    }
}

impl Session {
    /// The agent's next message; none once the session ended there.
    pub async fn next(&mut self) -> Option<Reply> {
        next(&mut self.read).await
    }
}

async fn next(read: &mut BufReader<OwnedReadHalf>) -> Option<Reply> {
    let mut line = String::new();
    let read = read.take(MAX_LINE as u64).read_line(&mut line).await.ok()?;
    if read == 0 {
        return None;
    }
    serde_json::from_str(&line).ok()
}

/// `browser` of `browser:4823`, `fd00::5` of `[fd00::5]:4823`.
fn host_of(agent: &str) -> &str {
    let host = agent.rsplit_once(':').map_or(agent, |(host, _)| host);
    host.trim_start_matches('[').trim_end_matches(']')
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;
    use tokio::net::TcpListener;

    use super::*;
    use crate::protocol::Open;

    #[test]
    fn the_display_is_on_the_agents_host() {
        assert_eq!(host_of("browser:4823"), "browser");
        assert_eq!(host_of("[fd00::5]:4823"), "fd00::5");
    }

    /// What the client sends is what the agent reads.
    #[tokio::test]
    async fn the_agent_understands_the_request() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let agent = listener.local_addr().unwrap().to_string();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut line = String::new();
            BufReader::new(read).read_line(&mut line).await.unwrap();
            write
                .write_all(
                    b"{\"type\":\"ready\",\"vnc_port\":5901,\"vnc_password\":\"abcdefgh\"}\n",
                )
                .await
                .unwrap();
            serde_json::from_str::<Open>(&line).unwrap()
        });
        let session = open(
            &agent,
            &Request {
                host: "web-target",
                port: 8443,
                spki: "c3BraQ==",
                width: 1280,
                height: 800,
                timezone: Some("Europe/Berlin"),
                login: Some(("tester", "secret")),
            },
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(
            (session.vnc_host.as_str(), session.vnc_port),
            ("127.0.0.1", 5901)
        );
        assert_eq!(session.vnc_password.as_str(), "abcdefgh");
        let open = server.await.unwrap();
        assert_eq!(open.authority(), "web-target:8443");
        assert_eq!(open.spki, "c3BraQ==");
        assert_eq!(open.timezone.as_deref(), Some("Europe/Berlin"));
        let login = open.login.unwrap();
        assert_eq!(login.username, "tester");
        assert_eq!(login.password.expose_secret(), "secret");
    }
}
