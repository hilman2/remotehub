//! The only way out for a session's Chromium: an HTTP proxy on the loopback
//! interface that allows `CONNECT` to the device's host and port and refuses
//! everything else. Links, redirects and scripts on the device's page thus
//! reach neither other devices nor the internet.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// Longest request head; a `CONNECT` is one short line plus a few headers.
const MAX_HEAD: usize = 8 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);

/// Listens on `127.0.0.1` for Chromium, allowing only `authority`
/// (`host:port` as [`crate::protocol::Open::authority`] builds it). Returns
/// the port and the task; aborting the task stops the proxy, tunnels already
/// open end with the browser.
pub async fn start(authority: String) -> std::io::Result<(u16, JoinHandle<()>)> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let authority = authority.clone();
            tokio::spawn(async move {
                let _ = serve(stream, &authority).await;
            });
        }
    });
    Ok((port, task))
}

async fn serve(mut client: TcpStream, allowed: &str) -> std::io::Result<()> {
    let Ok(Some((head, rest))) = tokio::time::timeout(TIMEOUT, read_head(&mut client)).await else {
        return Ok(());
    };
    let mut words = head.lines().next().unwrap_or_default().split_whitespace();
    let (method, target) = (
        words.next().unwrap_or_default(),
        words.next().unwrap_or_default(),
    );
    if method != "CONNECT" || !target.eq_ignore_ascii_case(allowed) {
        tracing::info!(method, target, "proxy refused a request outside the device");
        return client
            .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await;
    }
    let upstream = match tokio::time::timeout(TIMEOUT, TcpStream::connect(target)).await {
        Ok(Ok(upstream)) => upstream,
        _ => {
            return client
                .write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
        }
    };
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    let mut upstream = upstream;
    // Bytes the client sent right behind the head belong to the tunnel.
    upstream.write_all(&rest).await?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

/// The request head up to the empty line, and whatever followed it; none if
/// the client closed early or the head is too long.
async fn read_head(stream: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buffer.split_off(end + 4);
            return Some((String::from_utf8_lossy(&buffer).into_owned(), rest));
        }
        if buffer.len() > MAX_HEAD {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device that answers every connection with `hello`.
    async fn device() -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let authority = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let _ = stream.write_all(b"hello").await;
            }
        });
        (authority, task)
    }

    async fn ask(proxy: u16, request: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", proxy)).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut answer = String::new();
        let _ =
            tokio::time::timeout(Duration::from_secs(2), stream.read_to_string(&mut answer)).await;
        answer
    }

    #[tokio::test]
    async fn the_device_is_reachable() {
        let (authority, _device) = device().await;
        let (proxy, _task) = start(authority.clone()).await.unwrap();
        let answer = ask(
            proxy,
            &format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n"),
        )
        .await;
        assert_eq!(answer, "HTTP/1.1 200 Connection Established\r\n\r\nhello");
    }

    #[tokio::test]
    async fn nothing_else_is() {
        let (authority, _device) = device().await;
        let (other, _other) = device().await;
        let (proxy, _task) = start(authority.clone()).await.unwrap();
        for request in [
            format!("CONNECT {other} HTTP/1.1\r\n\r\n"),
            format!("GET http://{authority}/ HTTP/1.1\r\n\r\n"),
            format!("CONNECT {authority}x HTTP/1.1\r\n\r\n"),
        ] {
            let answer = ask(proxy, &request).await;
            assert!(
                answer.starts_with("HTTP/1.1 403"),
                "{request:?}: {answer:?}"
            );
            assert!(!answer.contains("hello"), "{request:?}: {answer:?}");
        }
    }
}
