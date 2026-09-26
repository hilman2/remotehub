use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use clap::{ArgGroup, Parser, Subcommand};
use remotehub_connector::access::{self, Access, Changer, Gate};
use remotehub_connector::agent::{self, AgentSettings, Site};
use remotehub_connector::journal::Journal;
use remotehub_connector::settings::{self, UiSettings};
use remotehub_connector::ui::{self, Ui};
use remotehub_connector::users::{Issued, Users};
use remotehub_connector::{VERSION, files, https};
use remotehub_i18n::{self as i18n, Locale, Message};
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{OffsetDateTime, PrimitiveDateTime};
use tracing_subscriber::EnvFilter;

/// remotehub site connector: reaches devices in this network for the
/// remotehub at REMOTEHUB_URL while access is open. Configuration comes from
/// REMOTEHUB_* environment variables; the data directory is
/// REMOTEHUB_CONNECTOR_DATA.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the connector and its web interface (the default).
    Run,
    /// Let remotehub in: for some hours, until a point in time, or without
    /// end. The running connector follows within a second.
    #[command(group(ArgGroup::new("how").required(true).args(["hours", "until", "permanent"])))]
    Open {
        /// For this many hours, from 1 to 168.
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..=168))]
        hours: Option<i64>,
        /// Until then: RFC 3339 such as 2026-10-01T18:00:00+02:00, or
        /// 2026-10-01T16:00 in UTC.
        #[arg(long)]
        until: Option<String>,
        /// Until someone closes it.
        #[arg(long)]
        permanent: bool,
    },
    /// Keep remotehub out; running connections end at once.
    Close,
    /// Show whether access is open.
    Status,
    /// Manage the users of the web interface.
    User {
        #[command(subcommand)]
        action: UserAction,
    },
}

#[derive(Subcommand)]
enum UserAction {
    /// Create a user with a generated password, shown once.
    Add {
        name: String,
        /// Also a TOTP secret for an authenticator app, shown once.
        #[arg(long)]
        totp: bool,
    },
    /// Replace the password, and the TOTP secret or none.
    Reset {
        name: String,
        #[arg(long)]
        totp: bool,
    },
    /// Delete a user.
    Delete { name: String },
    /// List the users.
    List,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let lookup = |name: &str| std::env::var(name).ok();
    let data = settings::data_dir(&lookup)?;
    let locale = Locale::from_posix(lookup);
    match Cli::parse().command.unwrap_or(Command::Run) {
        Command::Run => run(&data).await,
        Command::Open {
            hours,
            until,
            permanent,
        } => {
            let until = match (hours, until) {
                _ if permanent => None,
                (Some(hours), _) => Some(OffsetDateTime::now_utc() + time::Duration::hours(hours)),
                (None, Some(text)) => Some(parse_until(&text)?),
                (None, None) => unreachable!("clap requires one of them"),
            };
            change(&data, Access::Open { until })?;
            print_status(&data, locale)
        }
        Command::Close => {
            change(&data, Access::Closed)?;
            print_status(&data, locale)
        }
        Command::Status => print_status(&data, locale),
        Command::User { action } => manage_users(&data, locale, action),
    }
}

async fn run(data: &Path) -> anyhow::Result<()> {
    let lookup = |name: &str| std::env::var(name).ok();
    let agent_settings = AgentSettings::from_env(&lookup)?;
    let ui_settings = UiSettings::from_env(&lookup)?;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if lookup("REMOTEHUB_LOG_FORMAT").is_some_and(|f| f == "json") {
        builder.json().init();
    } else {
        builder.init();
    }
    tracing::info!(version = VERSION, url = %agent_settings.url, data = %data.display(), "starting the site connector");

    files::data_dir(data).with_context(|| format!("creating {}", data.display()))?;
    let journal = Arc::new(Journal::new(data));
    let site = Site {
        gate: Gate::new(data, journal.clone())?,
        journal,
        connections: Arc::default(),
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
        () = shutdown_signal() => tracing::info!("stopping"),
    }
    Ok(())
}

fn change(data: &Path, access: Access) -> anyhow::Result<()> {
    files::data_dir(data).with_context(|| format!("creating {}", data.display()))?;
    access::change(data, &Journal::new(data), access, Changer::CommandLine)?;
    Ok(())
}

/// RFC 3339, or a date and time without offset in UTC.
fn parse_until(text: &str) -> anyhow::Result<OffsetDateTime> {
    let until = OffsetDateTime::parse(text, &Rfc3339)
        .or_else(|_| {
            PrimitiveDateTime::parse(
                text,
                format_description!("[year]-[month]-[day]T[hour]:[minute]"),
            )
            .map(PrimitiveDateTime::assume_utc)
        })
        .with_context(|| format!("--until {text}: not a point in time"))?;
    anyhow::ensure!(
        until > OffsetDateTime::now_utc(),
        "--until {text}: not in the future"
    );
    Ok(until)
}

fn print_status(data: &Path, locale: Locale) -> anyhow::Result<()> {
    let stored = access::load(data)?;
    let now = OffsetDateTime::now_utc();
    let message = match stored.access {
        Access::Open { until: None } => Message::ConnectorCliOpenPermanent {},
        Access::Open { until: Some(until) } if stored.access.is_open(now) => {
            Message::ConnectorCliOpenUntil {
                until: until.format(&Rfc3339)?,
            }
        }
        _ => Message::ConnectorCliClosed {},
    };
    println!("{}", i18n::render(locale, &message));
    Ok(())
}

fn manage_users(data: &Path, locale: Locale, action: UserAction) -> anyhow::Result<()> {
    files::data_dir(data).with_context(|| format!("creating {}", data.display()))?;
    let users = Users::new(data);
    let say = |message: Message| println!("{}", i18n::render(locale, &message));
    match action {
        UserAction::Add { name, totp } => {
            let issued = users.add(&name, totp)?;
            print_issued(locale, &name, &issued);
        }
        UserAction::Reset { name, totp } => {
            let issued = users.reset(&name, totp)?;
            print_issued(locale, &name, &issued);
        }
        UserAction::Delete { name } => {
            users.delete(&name)?;
            say(Message::ConnectorCliUserDeleted { name });
        }
        UserAction::List => {
            let list = users.list()?;
            if list.is_empty() {
                say(Message::ConnectorCliNoUsers {});
            }
            let with_totp = i18n::render(locale, &Message::ConnectorCliWithTotp {});
            for (name, totp) in list {
                if totp {
                    println!("{name} ({with_totp})");
                } else {
                    println!("{name}");
                }
            }
        }
    }
    Ok(())
}

fn print_issued(locale: Locale, name: &str, issued: &Issued) {
    let say = |message: Message| i18n::render(locale, &message);
    println!(
        "{}",
        say(Message::ConnectorCliUserReady {
            name: name.to_owned()
        })
    );
    println!(
        "{} {}",
        say(Message::ConnectorCliPasswordLabel {}),
        issued.password.as_str()
    );
    if let Some((secret, uri)) = &issued.totp {
        println!(
            "{} {}",
            say(Message::ConnectorCliTotpSecretLabel {}),
            secret.as_str()
        );
        println!(
            "{} {}",
            say(Message::ConnectorCliTotpUriLabel {}),
            uri.as_str()
        );
    }
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
