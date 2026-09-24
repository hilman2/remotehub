//! remotehub-browser: the agent of the browser container (ADR 0007).
//!
//! Configuration from the environment:
//! - `REMOTEHUB_BROWSER_LISTEN`: address for remotehub, default `0.0.0.0:4823`
//! - `REMOTEHUB_BROWSER_SESSIONS`: sessions at once, default 8; session n
//!   gets display n and VNC port 5900 + n
//! - `REMOTEHUB_BROWSER_WORK`: profiles and password files, default
//!   `/tmp/remotehub-browser`

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use remotehub_browser::session::{self, Displays, Settings};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let setting = |name: &str, default: &str| std::env::var(name).unwrap_or(default.to_owned());
    let listen = setting("REMOTEHUB_BROWSER_LISTEN", "0.0.0.0:4823");
    let sessions: u16 = setting("REMOTEHUB_BROWSER_SESSIONS", "8")
        .parse()
        .ok()
        .filter(|n| (1..=99).contains(n))
        .expect("REMOTEHUB_BROWSER_SESSIONS: a number from 1 to 99");
    let work = PathBuf::from(setting("REMOTEHUB_BROWSER_WORK", "/tmp/remotehub-browser"));
    // Left over from a previous run: profiles hold cookies of signed-in pages.
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    std::fs::create_dir_all("/tmp/.X11-unix")?;

    let settings = Arc::new(Settings {
        chromium: "chromium".into(),
        xvnc: "Xvnc".into(),
        vncpasswd: "vncpasswd".into(),
        work,
        displays: 1..=sessions,
        fill_patience: Duration::from_secs(30),
    });
    let displays = Arc::new(Displays::default());
    let listener = TcpListener::bind(&listen).await?;
    tracing::info!(%listen, sessions, "remotehub-browser listening");
    loop {
        let (stream, peer) = listener.accept().await?;
        tracing::debug!(%peer, "connection");
        tokio::spawn(session::serve(stream, settings.clone(), displays.clone()));
    }
}
