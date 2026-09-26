use clap::Parser;
use remotehub_connector::VERSION;
use remotehub_connector::agent::{self, AgentSettings};
use tracing_subscriber::EnvFilter;

/// remotehub site connector: reaches devices in this network for the
/// remotehub at REMOTEHUB_URL, signed in with REMOTEHUB_CONNECTOR_TOKEN.
/// Configuration comes from REMOTEHUB_* environment variables.
#[derive(Parser)]
#[command(version)]
struct Cli {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    Cli::parse();
    let settings = AgentSettings::from_env()?;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if std::env::var("REMOTEHUB_LOG_FORMAT").is_ok_and(|f| f == "json") {
        builder.json().init();
    } else {
        builder.init();
    }
    tracing::info!(version = VERSION, url = %settings.url, "starting the site connector");
    tokio::select! {
        () = agent::run(settings) => {}
        () = shutdown_signal() => tracing::info!("stopping"),
    }
    Ok(())
}

/// Ctrl+C or SIGTERM. As the container's first process, the connector would
/// otherwise ignore `docker stop` until Docker kills it.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}
