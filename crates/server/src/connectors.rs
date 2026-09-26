//! Site connectors on remotehub's side (ADR 0008).
//!
//! A connector keeps a control WebSocket open to remotehub. When an engine
//! needs a device behind it, remotehub asks over that socket for a stream to
//! the device's host and port; the connector connects there and opens a
//! WebSocket of its own for the stream's bytes, which [`Connectors`] hands to
//! whoever asked. Engines never see this: they connect to a [`Forward`], a
//! listener that carries each connection through a new stream.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use remotehub_connector::protocol::{self, Control, State};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

/// How long a connector may take to reach the device and call back.
pub const STREAM_TIMEOUT: Duration = Duration::from_secs(15);
/// Bytes per WebSocket message from an engine to the device.
const CHUNK: usize = 64 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConnectorError {
    #[error("the connector is not connected")]
    Offline,
    #[error("the connector did not open the stream in time")]
    Timeout,
    #[error("the connector could not reach the target: {0}")]
    Refused(String),
}

/// The SHA-256 a connector's token is stored and looked up by.
pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// A stream's id: whoever knows it and the token gets the stream, so it is
/// random rather than counted.
fn random_id() -> Uuid {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the OS has randomness");
    uuid::Builder::from_random_bytes(bytes).into_uuid()
}

/// The connectors that are online and the streams asked of them.
#[derive(Default)]
pub struct Connectors {
    online: Mutex<HashMap<Uuid, (u64, mpsc::Sender<Control>)>>,
    pending: Mutex<HashMap<Uuid, Pending>>,
    generation: std::sync::atomic::AtomicU64,
    /// Streams carrying an engine's connection now, per connector.
    carrying: Mutex<HashMap<Uuid, usize>>,
    /// Streams carried since remotehub started, per connector.
    carried: Mutex<HashMap<Uuid, u64>>,
    /// Whether the customer lets remotehub in (#165), as each connector last
    /// reported it.
    access: Mutex<HashMap<Uuid, (State, Instant)>>,
}

/// A report older than this counts as none: the connector reports every
/// [`protocol::STATE_EVERY`], so it missed three.
const REPORT_STALE: Duration = Duration::from_secs(3 * protocol::STATE_EVERY.as_secs());

/// Counts one carried stream while it is held.
struct Carrying<'a> {
    connectors: &'a Connectors,
    connector: Uuid,
}

impl Drop for Carrying<'_> {
    fn drop(&mut self) {
        let mut carrying = self
            .connectors
            .carrying
            .lock()
            .expect("no panics while locked");
        if let Some(count) = carrying.get_mut(&self.connector) {
            *count -= 1;
            if *count == 0 {
                carrying.remove(&self.connector);
            }
        }
    }
}

struct Pending {
    connector: Uuid,
    reply: oneshot::Sender<Result<WebSocket, String>>,
}

/// A connector's control socket, as long as it is held. A newer one of the
/// same connector replaces it.
pub struct Registration {
    connectors: Arc<Connectors>,
    connector: Uuid,
    generation: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut online = self
            .connectors
            .online
            .lock()
            .expect("no panics while locked");
        if online
            .get(&self.connector)
            .is_some_and(|(generation, _)| *generation == self.generation)
        {
            online.remove(&self.connector);
        }
    }
}

impl Connectors {
    /// Marks the connector online; what arrives on the receiver goes to its
    /// control socket.
    pub fn register(self: &Arc<Self>, connector: Uuid) -> (mpsc::Receiver<Control>, Registration) {
        let (sender, receiver) = mpsc::channel(64);
        let generation = self
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.online
            .lock()
            .expect("no panics while locked")
            .insert(connector, (generation, sender));
        let registration = Registration {
            connectors: self.clone(),
            connector,
            generation,
        };
        (receiver, registration)
    }

    pub fn is_online(&self, connector: Uuid) -> bool {
        self.online
            .lock()
            .expect("no panics while locked")
            .contains_key(&connector)
    }

    /// How many connections the connector carries now.
    pub fn streams(&self, connector: Uuid) -> usize {
        self.carrying
            .lock()
            .expect("no panics while locked")
            .get(&connector)
            .copied()
            .unwrap_or_default()
    }

    /// How many connections the connector has carried since remotehub
    /// started.
    pub fn streams_carried(&self, connector: Uuid) -> u64 {
        self.carried
            .lock()
            .expect("no panics while locked")
            .get(&connector)
            .copied()
            .unwrap_or_default()
    }

    fn count(&self, connector: Uuid) -> Carrying<'_> {
        *self
            .carrying
            .lock()
            .expect("no panics while locked")
            .entry(connector)
            .or_default() += 1;
        *self
            .carried
            .lock()
            .expect("no panics while locked")
            .entry(connector)
            .or_default() += 1;
        Carrying {
            connectors: self,
            connector,
        }
    }

    /// Stores what the connector reports about its access, and returns what
    /// it reported before, if that is recent enough to compare with.
    pub fn report(&self, connector: Uuid, state: State) -> Option<State> {
        self.access
            .lock()
            .expect("no panics while locked")
            .insert(connector, (state, Instant::now()))
            .filter(|(_, at)| at.elapsed() < REPORT_STALE)
            .map(|(state, _)| state)
    }

    /// Whether the customer lets remotehub in, as the connector reported it
    /// lately; none if it has not.
    pub fn access(&self, connector: Uuid) -> Option<State> {
        self.access
            .lock()
            .expect("no panics while locked")
            .get(&connector)
            .filter(|(_, at)| at.elapsed() < REPORT_STALE)
            .map(|(state, _)| state.clone())
    }

    /// A stream to `target` through the connector, for the remotehub user
    /// `user`, whom the connector names in its journal.
    pub async fn stream(
        &self,
        connector: Uuid,
        target: &str,
        user: Option<&str>,
        timeout: Duration,
    ) -> Result<WebSocket, ConnectorError> {
        let control = self
            .online
            .lock()
            .expect("no panics while locked")
            .get(&connector)
            .map(|(_, sender)| sender.clone())
            .ok_or(ConnectorError::Offline)?;
        let id = random_id();
        let (reply, answer) = oneshot::channel();
        self.pending
            .lock()
            .expect("no panics while locked")
            .insert(id, Pending { connector, reply });
        let asked = control
            .send(Control::Open {
                id,
                target: target.to_owned(),
                user: user.map(str::to_owned),
            })
            .await;
        let result = match asked {
            Err(_) => Err(ConnectorError::Offline),
            Ok(()) => match tokio::time::timeout(timeout, answer).await {
                Ok(Ok(Ok(socket))) => Ok(socket),
                Ok(Ok(Err(reason))) => Err(ConnectorError::Refused(reason)),
                Ok(Err(_)) => Err(ConnectorError::Offline),
                Err(_) => Err(ConnectorError::Timeout),
            },
        };
        self.pending
            .lock()
            .expect("no panics while locked")
            .remove(&id);
        result
    }

    /// The waiting request for stream `id`, if `connector` was asked for it.
    pub fn claim(
        &self,
        connector: Uuid,
        id: Uuid,
    ) -> Option<oneshot::Sender<Result<WebSocket, String>>> {
        let mut pending = self.pending.lock().expect("no panics while locked");
        match pending.get(&id) {
            Some(waiting) if waiting.connector == connector => {
                pending.remove(&id).map(|waiting| waiting.reply)
            }
            _ => None,
        }
    }

    /// The connector reports that it could not open stream `id`.
    pub fn fail(&self, connector: Uuid, id: Uuid, reason: String) {
        if let Some(reply) = self.claim(connector, id) {
            let _ = reply.send(Err(reason));
        }
    }
}

/// A listener on remotehub's side that carries every connection it accepts
/// to the device through the connector. Dropping it stops listening;
/// connections already carried end on their own.
pub struct Forward {
    pub address: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Forward {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Forward {
    /// Listens on `bind`, port chosen by the system, and accepts connections
    /// only from `peers`: the engine that needs the device, and nobody else
    /// on remotehub's networks. `user` is the remotehub user the connections
    /// are for.
    pub async fn open(
        connectors: Arc<Connectors>,
        connector: Uuid,
        target: String,
        user: String,
        bind: IpAddr,
        peers: Vec<IpAddr>,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind((bind, 0)).await?;
        let address = listener.local_addr()?;
        let task = tokio::spawn(async move {
            while let Ok((engine, peer)) = listener.accept().await {
                if !peers.contains(&peer.ip()) {
                    tracing::warn!(%peer, %target, "forward refused a connection from elsewhere");
                    continue;
                }
                let connectors = connectors.clone();
                let target = target.clone();
                let user = user.clone();
                tokio::spawn(async move {
                    let stream = connectors.stream(connector, &target, Some(&user), STREAM_TIMEOUT);
                    match stream.await {
                        Ok(stream) => {
                            let _counted = connectors.count(connector);
                            carry(engine, stream).await;
                        }
                        Err(error) => {
                            tracing::warn!(%connector, %target, %error, "no stream through the connector");
                        }
                    }
                });
            }
        });
        Ok(Forward { address, task })
    }
}

/// Copies bytes both ways between an engine's connection and a stream until
/// both sides have finished; the connector's `agent::carry` is the other end.
pub async fn carry(engine: TcpStream, stream: WebSocket) {
    let (mut to_device, mut from_device) = stream.split();
    let (mut read, mut write) = engine.into_split();
    let up = async {
        let mut buffer = vec![0u8; CHUNK];
        loop {
            match read.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let data = buffer[..n].to_vec();
                    if to_device.send(Message::Binary(data.into())).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = to_device.send(Message::Close(None)).await;
    };
    let down = async {
        while let Some(Ok(message)) = from_device.next().await {
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

/// The address of this host that `service` (`host:port`) reaches it by, and
/// the addresses `service` connects from: for a [`Forward`] that only that
/// service may use.
pub async fn towards(service: &str) -> std::io::Result<(IpAddr, Vec<IpAddr>)> {
    let peers: Vec<IpAddr> = tokio::net::lookup_host(service)
        .await?
        .map(|address| address.ip())
        .collect();
    let first = *peers.first().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{service}: no address"),
        )
    })?;
    // Connecting a UDP socket sends nothing; it only picks the route, and
    // with it the local address.
    let socket = std::net::UdpSocket::bind(match first {
        IpAddr::V4(_) => "0.0.0.0:0",
        IpAddr::V6(_) => "[::]:0",
    })?;
    socket.connect((first, 9))?;
    Ok((socket.local_addr()?.ip(), peers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_offline_connector_opens_nothing() {
        let connectors = Arc::new(Connectors::default());
        let connector = random_id();
        assert_eq!(
            connectors
                .stream(connector, "x:1", None, Duration::from_secs(1))
                .await
                .err(),
            Some(ConnectorError::Offline)
        );
        let (_control, registration) = connectors.register(connector);
        assert!(connectors.is_online(connector));
        drop(registration);
        assert!(!connectors.is_online(connector));
    }

    /// A reconnected connector stays online when its old socket goes.
    #[tokio::test]
    async fn the_newest_control_socket_counts() {
        let connectors = Arc::new(Connectors::default());
        let connector = random_id();
        let (_old, old) = connectors.register(connector);
        let (_new, _registration) = connectors.register(connector);
        drop(old);
        assert!(connectors.is_online(connector));
    }

    /// Streams go only to the connector that was asked, and a refusal
    /// reaches the one waiting.
    #[tokio::test]
    async fn only_the_asked_connector_answers() {
        let connectors = Arc::new(Connectors::default());
        let connector = random_id();
        let (mut control, _registration) = connectors.register(connector);
        let waiting = {
            let connectors = connectors.clone();
            tokio::spawn(async move {
                connectors
                    .stream(
                        connector,
                        "ssh-target:22",
                        Some("alice"),
                        Duration::from_secs(5),
                    )
                    .await
                    .err()
            })
        };
        let Some(Control::Open { id, target, user }) = control.recv().await else {
            panic!("no open");
        };
        assert_eq!(target, "ssh-target:22");
        assert_eq!(user.as_deref(), Some("alice"));
        assert!(connectors.claim(random_id(), id).is_none());
        connectors.fail(connector, id, "refused".into());
        assert_eq!(
            waiting.await.unwrap(),
            Some(ConnectorError::Refused("refused".into()))
        );
    }
}
