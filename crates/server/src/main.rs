use std::io::Read;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use remotehub_gateway::ssh_ca::SshCa;
use remotehub_i18n::{self as i18n, Locale, Message};
use remotehub_server::api::health;
use remotehub_server::audit::{Action, Actor, Entry};
use remotehub_server::config::{self, Config};
use remotehub_server::{
    AppState, Settings, VERSION, app, audit, break_glass, db, escrow, session, setup,
};
use remotehub_vault::{DynVault, FileKeyring, KeyProvider, Vault, generate_key_line};
use tracing_subscriber::EnvFilter;
use zeroize::Zeroizing;

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
    /// Print the master key file, recovered from the database with the
    /// private key of the organisation recovery key, as printed for the
    /// safe. Reads that text from standard input; needs only
    /// REMOTEHUB_DATABASE_URL.
    RecoverMasterKey,
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
    /// Print the link to the setup wizard, with a new one-time code that
    /// replaces the previous one. Works until setup is complete.
    SetupCode,
    /// Manage local accounts in Ory Kratos (REMOTEHUB_KRATOS_URL).
    Account {
        #[command(subcommand)]
        action: AccountAction,
    },
}

#[derive(Subcommand)]
enum AccountAction {
    /// Create an account and print the one-time code its owner starts with.
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
        Command::RecoverMasterKey => recover_master_key().await,
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
        Command::SetupCode => setup_code().await,
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
    // A master key added to the file since the last start (#96).
    match escrow::keep(&pool, &vault).await {
        Ok(0) => {}
        Ok(sealed) => tracing::info!(sealed, "master keys kept for the recovery key"),
        Err(error) => tracing::error!(%error, "cannot keep the master keys for the recovery key"),
    }

    // The directory as the settings page stored it (#144). A stored one that
    // cannot be opened is logged, and the server starts without it: the
    // settings page is where it gets repaired.
    let directory = match remotehub_server::directory::open(&pool, &vault).await {
        Ok(Some(directory)) => {
            tracing::info!("signing in against the stored directory");
            Some(directory)
        }
        Ok(None) => {
            tracing::info!("no directory set up: local and break-glass accounts sign in");
            None
        }
        Err(error) => {
            tracing::error!(%error, "cannot open the stored directory");
            None
        }
    };

    let mut settings = Settings {
        public_origin: config.public_origin,
        session: config.session,
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
        caddy: None,
    };
    settings.caddy = config
        .caddy
        .clone()
        .map(|caddy| Arc::new(remotehub_server::caddy::Caddy::new(caddy, settings.host())));
    // Caddy starts once remotehub is healthy and imports the snippet written
    // here (#146): with the certificate of your own from the database, which
    // also brings it back after a restore onto a new host, else automatic.
    if let Some(caddy) = &settings.caddy {
        use remotehub_server::caddy::Tls;
        let tls = match remotehub_server::certificate::stored(&pool, &vault).await {
            Ok(Some(own)) => {
                let (chain, key) = remotehub_server::certificate::to_pem(&own);
                Some(Tls::Own { chain, key })
            }
            Ok(None) => Some(Tls::Automatic),
            // The files stay as they are.
            Err(error) => {
                tracing::error!(%error, "cannot read the stored certificate");
                None
            }
        };
        if let Some(Err(error)) = tls.map(|tls| caddy.prepare(&tls)) {
            tracing::error!(%error, "cannot write the certificate settings for Caddy");
        }
    }
    let state = AppState::new(pool.clone(), directory, settings, vault);
    tokio::spawn(purge_sessions(pool, config.session.idle));
    tokio::spawn(remotehub_server::refresh::run(state.clone()));

    // Kratos hands over its mails on a port of their own (#145), which only
    // the compose network reaches.
    if let Some(token) = config.courier_token.clone() {
        let courier = remotehub_server::api::courier::router(state.clone(), token);
        let listener = tokio::net::TcpListener::bind(config.courier_listen)
            .await
            .with_context(|| format!("cannot listen on {}", config.courier_listen))?;
        tracing::info!(listen = %config.courier_listen, "taking mails from Kratos");
        tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, courier).await {
                tracing::error!(%error, "the courier's port stopped");
            }
        });
    }

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

async fn setup_code() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let pool = db::connect(&config.database_url, config.database_password.as_ref())
        .await
        .context("cannot connect to the database")?;
    db::MIGRATOR
        .run(&pool)
        .await
        .context("cannot apply database migrations")?;
    match setup::new_code(&pool).await {
        Ok(code) => {
            say(Message::SetupReady {});
            // The code goes straight to the terminal, like an invitation's.
            println!();
            println!("  {}", setup::link(&config.public_origin, &code));
            println!();
            Ok(())
        }
        Err(setup::SetupError::Complete) => {
            say(Message::SetupComplete {});
            std::process::exit(1);
        }
        Err(error) => Err(error.into()),
    }
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

async fn recover_master_key() -> anyhow::Result<()> {
    let (url, password) = config::database(&|name| std::env::var(name).ok())?;
    let pool = db::connect(&url, password.as_ref())
        .await
        .context("cannot connect to the database")?;
    let mut text = Zeroizing::new(String::new());
    std::io::stdin()
        .read_to_string(&mut text)
        .context("cannot read the private key from standard input")?;
    let private_key = remotehub_vault::escrow::parse_printed(&text)
        .context("that is not a private key as remotehub prints it")?;
    let file = escrow::recover(&pool, &private_key).await?;
    // Straight to standard output, like `generate-key`: redirect it into
    // the master key file.
    print!("{}", file.as_str());
    Ok(())
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
