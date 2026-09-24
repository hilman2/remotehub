//! `GET /api/health`: liveness, version and database reachability. Used by
//! the container health check (`remotehub healthcheck`, [`probe`]), the
//! update script and the UI footer.

use std::net::SocketAddr;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::{AppState, VERSION, db};

#[derive(Debug, Serialize)]
pub struct Health {
    pub status: Status,
    pub version: &'static str,
    pub database: Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Unavailable,
}

/// Asks the server listening on `address` for its health: `Ok` if it answers
/// 200. On Linux, `0.0.0.0` and `::` reach the server on loopback.
pub async fn probe(address: SocketAddr) -> Result<(), String> {
    let ask = async {
        let mut stream = TcpStream::connect(address).await?;
        stream
            .write_all(b"GET /api/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        Ok::<_, std::io::Error>(response)
    };
    let response = tokio::time::timeout(Duration::from_secs(5), ask)
        .await
        .map_err(|_| format!("{address} did not answer within 5 s"))?
        .map_err(|error| format!("{address}: {error}"))?;
    let status_line = response.split(|&b| b == b'\r').next().unwrap_or_default();
    let status_line = String::from_utf8_lossy(status_line);
    if status_line.starts_with("HTTP/1.1 200 ") {
        Ok(())
    } else {
        Err(format!("{address} answered {status_line:?}"))
    }
}

pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<Health>) {
    let database = if db::is_reachable(&state.db).await {
        Status::Ok
    } else {
        Status::Unavailable
    };
    let code = match database {
        Status::Ok => StatusCode::OK,
        Status::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
    };
    (
        code,
        Json(Health {
            status: database,
            version: VERSION,
            database,
        }),
    )
}
