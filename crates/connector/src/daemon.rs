//! The running connector: the way to remotehub and the web interface, as
//! `remotehub-connector run` and the Windows service start it.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

use crate::access::Gate;
use crate::agent::{self, AgentSettings, Site};
use crate::journal::Journal;
use crate::settings::{self, UiSettings};
use crate::ui::{self, Ui};
use crate::users::Users;
use crate::{VERSION, files, https};

/// Where the log output goes.
pub enum Logs {
    /// Standard output, as a container's log.
    Stdout,
    /// A file in the data directory, for a service without a console.
    File,
}

/// `connector.log` in the data directory starts over beyond this size.
const LOG_FILE_LIMIT: u64 = 10 * 1024 * 1024;

/// Sets up the log output; `REMOTEHUB_LOG_FORMAT=json` writes JSON.
pub fn init_logs(data: &Path, logs: Logs) -> anyhow::Result<()> {
    let lookup = settings::lookup(data);
    let json = lookup("REMOTEHUB_LOG_FORMAT").is_some_and(|f| f == "json");
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match logs {
        Logs::Stdout if json => builder.json().init(),
        Logs::Stdout => builder.init(),
        Logs::File => {
            files::data_dir(data)?;
            let path = data.join("connector.log");
            if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_FILE_LIMIT) {
                std::fs::rename(&path, path.with_extension("log.1"))?;
            }
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .with_context(|| format!("opening {}", path.display()))?;
            let builder = builder
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(file));
            if json {
                builder.json().init();
            } else {
                builder.init();
            }
        }
    }
    Ok(())
}

/// Runs the connector with its data in `data` until the future is dropped,
/// or returns why it cannot start.
pub async fn run(data: &Path) -> anyhow::Result<()> {
    let lookup = settings::lookup(data);
    let agent_settings = AgentSettings::from_env(&lookup)?;
    let ui_settings = UiSettings::from_env(&lookup)?;
    tracing::info!(version = VERSION, url = %agent_settings.url, data = %data.display(), "starting the site connector");

    files::data_dir(data).with_context(|| format!("creating {}", data.display()))?;
    let journal = Arc::new(Journal::new(data));
    let site = Site {
        gate: Gate::new(data, journal.clone())?,
        journal,
        connections: Arc::default(),
        requests: Arc::default(),
    };
    let users = Users::new(data);
    if users.list()?.is_empty() {
        tracing::warn!(
            "nobody can sign in to the web interface yet: remotehub-connector user add NAME"
        );
    }
    let (acceptor, fingerprint) = https::acceptor(ui_settings.certificate.as_ref(), data)?;
    let listener = tokio::net::TcpListener::bind(ui_settings.listen)
        .await
        .with_context(|| format!("listening on {}", ui_settings.listen))?;
    tracing::info!(listen = %ui_settings.listen, %fingerprint, "web interface");
    let remotehub = agent_settings.url.host().unwrap_or_default().to_owned();
    let router = ui::router(Ui::new(site.clone(), users, remotehub));
    tokio::select! {
        () = agent::run(agent_settings, site) => {}
        () = https::serve(listener, acceptor, router) => {}
    }
    Ok(())
}
