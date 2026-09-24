use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use remotehub_directory::Sid;
use remotehub_directory::ldap::LdapDirectory;
use remotehub_server::api::health;
use remotehub_server::audit::{Action, Actor, Entry};
use remotehub_server::auth::Authenticator;
use remotehub_server::config::{self, Config};
use remotehub_server::{AppState, Settings, VERSION, app, audit, break_glass, db, session};
use remotehub_vault::{DynVault, FileKeyring, KeyProvider, Vault, generate_key_line};
use tracing_subscriber::EnvFilter;

/// remotehub: browser-based remote access and credential vault.
/// Configuration comes from REMOTEHUB_* environment variables.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server (the default).
    Serve,
    /// Print a new line for the master key file (REMOTEHUB_MASTER_KEY_FILE).
    /// To rotate, append a line with the next version and restart.
    GenerateKey {
        #[arg(long, default_value_t = 1)]
        version: i32,
    },
    /// Recompute the audit log's hash chain; exits with 1 if it is broken.
    VerifyAudit,
    /// Ask the server running in this container for /api/health; exits with
    /// 1 unless it answers 200. The image's health check: it has no shell
    /// and no curl.
    Healthcheck,
    /// Manage break-glass accounts: local emergency accounts that work
    /// without the directory. Password and TOTP secret are shown only once.
    BreakGlass {
        #[command(subcommand)]
        action: BreakGlassAction,
    },
}

#[derive(Subcommand)]
enum BreakGlassAction {
    /// Create an account with a generated password and TOTP secret.
    Create { username: String },
    /// Replace password and TOTP secret; open sessions end.
    Reset { username: String },
    /// Delete an account.
    Delete { username: String },
    /// List the accounts.
    List,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command.unwrap_or(Command::Serve) {
        Command::Serve => serve().await,
        Command::GenerateKey { version } => {
            anyhow::ensure!(version >= 1, "the version starts at 1");
            println!("{}", generate_key_line(version).as_str());
            Ok(())
        }
        Command::VerifyAudit => verify_audit().await,
        Command::Healthcheck => {
            let listen = config::listen_address(&|name| std::env::var(name).ok())?;
            if let Err(error) = health::probe(listen).await {
                eprintln!("unhealthy: {error}");
                std::process::exit(1);
            }
            Ok(())
        }
        Command::BreakGlass { action } => manage_break_glass(action).await,
    }
}

async fn serve() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    init_tracing(config.log_json);
    tracing::info!(version = VERSION, listen = %config.listen, public = %config.public_origin, "starting remotehub");

    let vault = load_vault(&config.master_key_file)?;

    let pool = db::connect(&config.database_url, config.database_password.as_ref())
        .await
        .context("cannot connect to the database")?;
    db::MIGRATOR
        .run(&pool)
        .await
        .context("cannot apply database migrations")?;

    let ldap = match config.ldap {
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
    let admin_groups = resolve_admin_groups(&config.admin_groups, ldap.as_deref()).await?;
    let directory = ldap.map(|ldap| ldap as Arc<dyn Authenticator>);

    let settings = Settings {
        public_origin: config.public_origin,
        session: config.session,
        admin_groups,
        own_account_connections: config.own_account_connections,
        guacd: config.guacd,
        rdp_keyboard_layout: config.rdp_keyboard_layout,
    };
    let state = AppState::new(pool.clone(), directory, settings, vault);
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

/// Turns the configured admin groups into SIDs; names are looked up in the
/// directory (exact name, case-insensitive).
async fn resolve_admin_groups(
    entries: &[String],
    ldap: Option<&LdapDirectory>,
) -> anyhow::Result<Vec<String>> {
    let mut sids = Vec::new();
    for entry in entries {
        if entry.parse::<Sid>().is_ok() {
            sids.push(entry.clone());
            continue;
        }
        let ldap = ldap.with_context(|| {
            format!(
                "REMOTEHUB_ADMIN_GROUPS names the group {entry:?}, but no directory is configured"
            )
        })?;
        let group = ldap
            .search_groups(entry, 50)
            .await
            .with_context(|| format!("cannot look up the admin group {entry:?}"))?
            .into_iter()
            .find(|g| g.name.eq_ignore_ascii_case(entry))
            .with_context(|| format!("the admin group {entry:?} does not exist"))?;
        tracing::info!(group = %group.name, sid = %group.sid, "admin group");
        sids.push(group.sid.to_string());
    }
    if sids.is_empty() {
        tracing::warn!(
            "REMOTEHUB_ADMIN_GROUPS is empty: only break-glass accounts can administer remotehub"
        );
    }
    Ok(sids)
}

async fn manage_break_glass(action: BreakGlassAction) -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let vault = load_vault(&config.master_key_file)?;
    let pool = db::connect(&config.database_url, config.database_password.as_ref())
        .await
        .context("cannot connect to the database")?;
    db::MIGRATOR
        .run(&pool)
        .await
        .context("cannot apply database migrations")?;
    let sign_in_url = format!("{}/sign-in/break-glass", config.public_origin);

    let (issued, action) = match action {
        BreakGlassAction::List => {
            for account in break_glass::list(&pool).await? {
                println!("{}", account.username);
            }
            return Ok(());
        }
        BreakGlassAction::Delete { username } => {
            let account = break_glass::delete(&pool, &username).await?;
            audit_cli(
                &pool,
                Action::BreakGlassDeleted,
                account.user_id,
                &account.username,
            )
            .await?;
            println!("Break-glass account {:?} deleted.", account.username);
            return Ok(());
        }
        BreakGlassAction::Create { username } => (
            break_glass::create(&pool, &vault, &username).await?,
            Action::BreakGlassCreated,
        ),
        BreakGlassAction::Reset { username } => (
            break_glass::reset(&pool, &vault, &username).await?,
            Action::BreakGlassReset,
        ),
    };
    let user_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM users WHERE kind = 'local' AND lower(username) = lower($1)",
    )
    .bind(&issued.username)
    .fetch_one(&pool)
    .await?;
    audit_cli(&pool, action, user_id, &issued.username).await?;
    println!(
        "Break-glass account {:?} is ready. This is shown only once; keep it offline, \
         e.g. in a safe.\n\n  Password:    {}\n  TOTP secret: {}\n  TOTP URI:    {}\n\n\
         Sign in at {sign_in_url}",
        issued.username,
        issued.password.as_str(),
        issued.totp_secret.as_str(),
        issued.totp_uri.as_str(),
    );
    Ok(())
}

async fn audit_cli(
    pool: &sqlx::PgPool,
    action: Action,
    user_id: uuid::Uuid,
    username: &str,
) -> anyhow::Result<()> {
    audit::record(
        pool,
        Entry {
            actor: Actor {
                id: None,
                name: "cli",
            },
            action,
            object: Some(("user", user_id)),
            details: serde_json::json!({ "username": username }),
            address: None,
        },
    )
    .await?;
    Ok(())
}

async fn verify_audit() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let pool = db::connect(&config.database_url, config.database_password.as_ref())
        .await
        .context("cannot connect to the database")?;
    let result = audit::verify(&pool).await?;
    match result.first_broken {
        None => {
            println!("audit log intact: {} entries", result.entries);
            Ok(())
        }
        Some(seq) => {
            eprintln!(
                "audit log BROKEN from entry {seq} on ({} entries)",
                result.entries
            );
            std::process::exit(1);
        }
    }
}

fn load_vault(path: &Path) -> anyhow::Result<DynVault> {
    warn_if_readable_by_others(path);
    let keyring = FileKeyring::load(path).context("cannot load the master key file")?;
    let (id, version) = keyring.current();
    tracing::info!(file = %path.display(), key = id, version, "vault ready");
    Ok(Vault::new(Box::new(keyring)))
}

#[cfg(unix)]
fn warn_if_readable_by_others(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path)
        && meta.permissions().mode() & 0o077 != 0
    {
        tracing::warn!(file = %path.display(), "the master key file is accessible to other users; use mode 0400");
    }
}

#[cfg(not(unix))]
fn warn_if_readable_by_others(_: &Path) {}

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
