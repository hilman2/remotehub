use std::path::Path;

use anyhow::Context;
use clap::{ArgGroup, Parser, Subcommand};
use remotehub_connector::access::{self, Access, Changer};
use remotehub_connector::daemon::{self, Logs};
use remotehub_connector::journal::Journal;
use remotehub_connector::users::{Issued, Users};
use remotehub_connector::{files, settings};
use remotehub_i18n::{self as i18n, Locale, Message};
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{OffsetDateTime, PrimitiveDateTime};

/// remotehub site connector: reaches devices in this network for the
/// remotehub at REMOTEHUB_URL while access is open. Configuration comes from
/// REMOTEHUB_* environment variables and connector.conf in the data
/// directory, REMOTEHUB_CONNECTOR_DATA.
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
    /// Install and start the Windows service. Asks for the token.
    #[cfg(windows)]
    Install {
        /// remotehub's address, e.g. https://remotehub.example.com.
        #[arg(long)]
        url: String,
        /// Address ranges the connector may reach, e.g. 10.20.0.0/16.
        #[arg(long)]
        allow: Option<String>,
    },
    /// Replace the installed program with this one and restart the service.
    #[cfg(windows)]
    Update,
    /// Remove the Windows service and the program.
    #[cfg(windows)]
    Uninstall {
        /// Also remove the data: access state, log, users.
        #[arg(long)]
        purge: bool,
    },
    /// Run as the Windows service; the service manager calls this.
    #[cfg(windows)]
    #[command(hide = true)]
    Service,
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

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let data = settings::data_dir(&|name: &str| std::env::var(name).ok())?;
    let locale = cli_locale();
    match cli.command.unwrap_or(Command::Run) {
        Command::Run => {
            daemon::init_logs(&data, Logs::Stdout)?;
            tokio::runtime::Runtime::new()?.block_on(async {
                tokio::select! {
                    result = daemon::run(&data) => result,
                    () = shutdown_signal() => {
                        tracing::info!("stopping");
                        Ok(())
                    }
                }
            })
        }
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
        #[cfg(windows)]
        Command::Install { url, allow } => {
            remotehub_connector::windows::install(&url, allow.as_deref(), locale)
        }
        #[cfg(windows)]
        Command::Update => remotehub_connector::windows::update(locale),
        #[cfg(windows)]
        Command::Uninstall { purge } => remotehub_connector::windows::uninstall(purge, locale),
        #[cfg(windows)]
        Command::Service => remotehub_connector::windows::dispatch(),
    }
}

/// The language of the command line's output: the POSIX variables if one
/// is set, else on Windows the user's language, else English.
fn cli_locale() -> Locale {
    let set = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .any(|name| std::env::var(name).is_ok_and(|value| !value.is_empty()));
    if set {
        return Locale::from_posix(|name| std::env::var(name).ok());
    }
    #[cfg(windows)]
    if let Some(locale) = remotehub_connector::windows::user_locale() {
        return locale;
    }
    Locale::En
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
