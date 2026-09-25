use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use remotehub_directory::Sid;
use remotehub_directory::ldap::LdapDirectory;
use remotehub_gateway::ssh_ca::SshCa;
use remotehub_i18n::{self as i18n, Locale, Message};
use remotehub_server::api::health;
use remotehub_server::audit::{Action, Actor, Entry};
use remotehub_server::auth::Authenticator;
use remotehub_server::config::{self, Config};
use remotehub_server::connector_agent::{self, AgentSettings};
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
    /// Print a new SSH CA key for REMOTEHUB_SSH_CA_KEY_FILE. Replacing the
    /// key means every target must trust the new public key.
    GenerateSshCa,
    /// Recompute the audit log's hash chain; exits with 1 if it is broken.
    VerifyAudit,
    /// Ask the server running in this container for /api/health; exits with
    /// 1 unless it answers 200. The image's health check: it has no shell
    /// and no curl.
    Healthcheck,
    /// Run as a site connector: reach devices in this network for the
    /// remotehub at REMOTEHUB_URL, signed in with REMOTEHUB_CONNECTOR_TOKEN.
    Connector,
    /// Manage break-glass accounts: local emergency accounts that work
    /// without the directory. Password and TOTP secret are shown only once.
    BreakGlass {
        #[command(subcommand)]
        action: BreakGlassAction,
    },
    /// Manage local accounts in Ory Kratos (REMOTEHUB_KRATOS_URL).
    Account {
        #[command(subcommand)]
        action: AccountAction,
    },
}

#[derive(Subcommand)]
enum AccountAction {
    /// Create an account and print the one-time code its owner starts with.
    /// Administrators are the accounts in REMOTEHUB_ADMIN_ACCOUNTS.
    Invite {
        email: String,
        /// The name remotehub shows; the e-mail address without one.
        #[arg(long, default_value = "")]
        name: String,
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
        Command::GenerateSshCa => {
            print!("{}", SshCa::generate().as_str());
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
        Command::Account { action } => manage_accounts(action).await,
        Command::Connector => {
            let settings = AgentSettings::from_env()?;
            init_tracing(std::env::var("REMOTEHUB_LOG_FORMAT").is_ok_and(|f| f == "json"));
            tracing::info!(version = VERSION, url = %settings.url, "starting the site connector");
            connector_agent::run(settings).await;
            Ok(())
        }
    }
}

async fn serve() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    init_tracing(config.log_json);
    tracing::info!(version = VERSION, listen = %config.listen, public = %config.public_origin, "starting remotehub");

    let vault = load_vault(&config.master_key_file)?;
    let ssh_ca = config
        .ssh_ca_key_file
        .as_deref()
        .map(load_ssh_ca)
        .transpose()?;

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
        browser: config.browser,
        rdp_keyboard_layout: config.rdp_keyboard_layout,
        trusted_proxies: config.trusted_proxies,
        ssh_ca,
        kratos: config
            .kratos
            .as_ref()
            .map(remotehub_server::kratos::Kratos::new),
        admin_accounts: config.admin_accounts,
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
            say(Message::BreakGlassDeleted {
                username: account.username,
            });
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
        "SELECT id FROM users WHERE kind = 'break_glass' AND lower(username) = lower($1)",
    )
    .bind(&issued.username)
    .fetch_one(&pool)
    .await?;
    audit_cli(&pool, action, user_id, &issued.username).await?;
    say(Message::BreakGlassReady {
        username: issued.username.clone(),
    });
    // The secrets go straight from their zeroizing buffers to the terminal;
    // only the labels are translated, padded to the longest one.
    let locale = cli_locale();
    let lines = [
        (
            Message::BreakGlassPasswordLabel {},
            issued.password.as_str(),
        ),
        (
            Message::BreakGlassTotpSecretLabel {},
            issued.totp_secret.as_str(),
        ),
        (Message::BreakGlassTotpUriLabel {}, issued.totp_uri.as_str()),
    ]
    .map(|(label, value)| (i18n::render(locale, &label), value));
    let width = lines
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0);
    println!();
    for (label, value) in lines {
        println!("  {label:<width$} {value}");
    }
    println!();
    say(Message::BreakGlassSignIn { url: sign_in_url });
    Ok(())
}

/// How long an invitation's code lasts.
const INVITATION: Duration = Duration::from_secs(48 * 3600);

async fn manage_accounts(action: AccountAction) -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let kratos = config
        .kratos
        .as_ref()
        .map(remotehub_server::kratos::Kratos::new)
        .context("REMOTEHUB_KRATOS_URL is not set: local accounts are off")?;
    let pool = db::connect(&config.database_url, config.database_password.as_ref())
        .await
        .context("cannot connect to the database")?;
    db::MIGRATOR
        .run(&pool)
        .await
        .context("cannot apply database migrations")?;
    let AccountAction::Invite { email, name } = action;
    let email = email.trim().to_lowercase();
    let invitation = kratos
        .invite(&email, name.trim(), INVITATION)
        .await
        .with_context(|| format!("cannot invite {email}"))?;
    let mut tx = pool.begin().await?;
    let user =
        remotehub_server::kratos::add_invited(&mut *tx, &invitation, &email, name.trim()).await?;
    audit::record(
        &mut *tx,
        Entry {
            actor: Actor {
                id: None,
                name: "cli",
            },
            action: Action::AccountInvited,
            object: Some(("user", user)),
            details: serde_json::json!({
                "email": email, "identity_id": invitation.identity_id,
            }),
            address: None,
        },
    )
    .await?;
    tx.commit().await?;
    say(Message::AccountInvited {
        email,
        expires: invitation.expires_at.clone(),
    });
    // The code goes straight to the terminal, like a break-glass password.
    let locale = cli_locale();
    println!();
    println!(
        "  {} {}",
        i18n::render(locale, &Message::AccountLinkLabel {}),
        invitation.recovery_link
    );
    println!(
        "  {} {}",
        i18n::render(locale, &Message::AccountCodeLabel {}),
        invitation.recovery_code
    );
    println!();
    Ok(())
}

/// The locale of the admin's terminal (`LC_ALL`, `LC_MESSAGES`, `LANG`).
fn cli_locale() -> Locale {
    Locale::from_posix(|name| std::env::var(name).ok())
}

/// Prints a message in the terminal's locale.
fn say(message: Message) {
    println!("{}", i18n::render(cli_locale(), &message));
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
            say(Message::AuditIntact {
                entries: result.entries,
            });
            Ok(())
        }
        Some(first) => {
            let broken = Message::AuditBroken {
                first,
                entries: result.entries,
            };
            eprintln!("{}", i18n::render(cli_locale(), &broken));
            std::process::exit(1);
        }
    }
}

fn load_vault(path: &Path) -> anyhow::Result<DynVault> {
    warn_if_readable_by_others(path, "master key file");
    let keyring = FileKeyring::load(path).context("cannot load the master key file")?;
    let (id, version) = keyring.current();
    tracing::info!(file = %path.display(), key = id, version, "vault ready");
    Ok(Vault::new(Box::new(keyring)))
}

fn load_ssh_ca(path: &Path) -> anyhow::Result<Arc<SshCa>> {
    warn_if_readable_by_others(path, "SSH CA key file");
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read the SSH CA key file {}", path.display()))?;
    let ca = SshCa::from_openssh(&text).context("invalid SSH CA key file")?;
    tracing::info!(file = %path.display(), public_key = %ca.public_key(), "SSH CA ready");
    Ok(Arc::new(ca))
}

/// `what` names the file in the warning, e.g. "master key file".
#[cfg(unix)]
fn warn_if_readable_by_others(path: &Path, what: &str) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path)
        && meta.permissions().mode() & 0o077 != 0
    {
        tracing::warn!(file = %path.display(), "the {what} is accessible to other users; use mode 0400");
    }
}

#[cfg(not(unix))]
fn warn_if_readable_by_others(_: &Path, _: &str) {}

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
