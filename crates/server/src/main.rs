use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use remotehub_directory::ldap::LdapDirectory;
use remotehub_server::auth::Authenticator;
use remotehub_server::config::Config;
use remotehub_server::{AppState, Settings, VERSION, app, db, session};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    init_tracing(config.log_json);
    tracing::info!(version = VERSION, listen = %config.listen, public = %config.public_origin, "starting remotehub");

    let pool = db::connect(&config.database_url)
        .await
        .context("cannot connect to the database")?;
    db::MIGRATOR
        .run(&pool)
        .await
        .context("cannot apply database migrations")?;

    let directory: Option<Arc<dyn Authenticator>> = match config.ldap {
        Some(ldap) => {
            tracing::info!(url = %ldap.url, base = %ldap.base_dn, "signing in against LDAP");
            Some(Arc::new(
                LdapDirectory::new(ldap).context("invalid LDAP settings")?,
            ))
        }
        None => {
            tracing::warn!("no directory configured: only break-glass accounts can sign in");
            None
        }
    };

    let settings = Settings {
        public_origin: config.public_origin,
        session: config.session,
    };
    let state = AppState::new(pool.clone(), directory, settings);
    tokio::spawn(purge_sessions(pool, config.session.idle));

    let app = app(state, config.web_dir.as_deref());
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("cannot listen on {}", config.listen))?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    tracing::info!("stopped");
    Ok(())
}

/// Deletes sessions that can no longer be used, every ten minutes.
async fn purge_sessions(pool: sqlx::PgPool, idle: Duration) {
    let mut interval = tokio::time::interval(Duration::from_secs(600));
    loop {
        interval.tick().await;
        match session::purge(&pool, idle).await {
            Ok(0) => {}
            Ok(n) => tracing::debug!(sessions = n, "purged ended sessions"),
            Err(error) => tracing::warn!(%error, "cannot purge sessions"),
        }
    }
}

fn init_tracing(json: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if json {
        builder.json().init();
    } else {
        builder.init();
    }
}

/// Ctrl+C or SIGTERM (docker stop) end the server after open requests finish.
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
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutting down");
}
