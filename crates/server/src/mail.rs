//! The mail server (#145): stored in the table `mail`, its password sealed
//! like a vault entry. remotehub sends every mail itself: invitations, new
//! sign-in codes, and the codes Kratos makes for a forgotten password
//! (`api/courier.rs`). The settings are read for every mail; mails are rare.

use std::time::Duration;

use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Certificate, Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use remotehub_i18n::{self as i18n, Locale, Message};
use remotehub_vault::DynVault;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sqlx::{PgExecutor, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::secrets::{self, SecretError};

const PASSWORD_FIELD: &str = "smtp_password";
/// Limit for connecting and for each SMTP command.
const TIMEOUT: Duration = Duration::from_secs(15);

/// Everything about the mail server but its password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Server {
    pub host: String,
    pub port: i32,
    /// `tls`, `starttls` or `none`.
    pub security: String,
    pub username: Option<String>,
    pub from_address: String,
    pub from_name: String,
    pub ca_pem: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Secret(#[from] SecretError),
}

#[derive(sqlx::FromRow)]
struct Stored {
    id: Uuid,
    password_version: Option<i32>,
    #[sqlx(flatten)]
    server: Server,
}

macro_rules! select {
    () => {
        "SELECT id, password_version, host, port, security, username, from_address, from_name,
                ca_pem
         FROM mail"
    };
}

/// The stored server, without its password.
pub async fn server<'e>(db: impl PgExecutor<'e>) -> Result<Option<Server>, sqlx::Error> {
    Ok(sqlx::query_as::<_, Stored>(select!())
        .fetch_optional(db)
        .await?
        .map(|stored| stored.server))
}

/// The stored server and its password.
async fn stored(
    db: &PgPool,
    vault: &DynVault,
) -> Result<Option<(Server, Option<SecretString>)>, MailError> {
    let Some(stored) = sqlx::query_as::<_, Stored>(select!())
        .fetch_optional(db)
        .await?
    else {
        return Ok(None);
    };
    let password = match stored.password_version {
        None => None,
        Some(version) => secrets::load(db, vault, stored.id, version, PASSWORD_FIELD)
            .await?
            .and_then(|plain| String::from_utf8(plain.to_vec()).ok())
            .map(SecretString::from),
    };
    Ok(Some((stored.server, password)))
}

/// The password to send with: the new one, or the stored one if the user
/// name stays the same.
pub async fn password_for(
    db: &PgPool,
    vault: &DynVault,
    server: &Server,
    new: Option<SecretString>,
) -> Result<Option<SecretString>, MailError> {
    if new.is_some() || server.username.is_none() {
        return Ok(new);
    }
    Ok(match stored(db, vault).await? {
        Some((before, password)) if before.username == server.username => password,
        _ => None,
    })
}

/// Stores `server` in place of the one before. `password` is the password
/// to keep: a new one is sealed, none drops the stored one.
pub async fn save(
    tx: &mut Transaction<'_, Postgres>,
    vault: &DynVault,
    server: &Server,
    password: Option<&SecretString>,
    by: Uuid,
) -> Result<Option<Server>, MailError> {
    let before = sqlx::query_as::<_, Stored>(concat!(select!(), " FOR UPDATE"))
        .fetch_optional(&mut **tx)
        .await?;
    let id = match &before {
        Some(stored) => stored.id,
        None => {
            sqlx::query_scalar("SELECT gen_random_uuid()")
                .fetch_one(&mut **tx)
                .await?
        }
    };
    // A new version each time: sealed versions are never overwritten.
    let version = match password {
        None => None,
        Some(password) => {
            let version = before
                .as_ref()
                .and_then(|s| s.password_version)
                .unwrap_or(0)
                + 1;
            secrets::store(
                &mut **tx,
                vault,
                id,
                version,
                PASSWORD_FIELD,
                password.expose_secret().as_bytes(),
            )
            .await?;
            Some(version)
        }
    };
    sqlx::query(
        "INSERT INTO mail (id, host, port, security, username, password_version, from_address,
                           from_name, ca_pem, updated_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (singleton) DO UPDATE SET
             host = EXCLUDED.host, port = EXCLUDED.port, security = EXCLUDED.security,
             username = EXCLUDED.username, password_version = EXCLUDED.password_version,
             from_address = EXCLUDED.from_address, from_name = EXCLUDED.from_name,
             ca_pem = EXCLUDED.ca_pem, updated_at = now(), updated_by = EXCLUDED.updated_by",
    )
    .bind(id)
    .bind(&server.host)
    .bind(server.port)
    .bind(&server.security)
    .bind(&server.username)
    .bind(version)
    .bind(&server.from_address)
    .bind(&server.from_name)
    .bind(&server.ca_pem)
    .bind(by)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM secret_fields WHERE owner_id = $1 AND field = $2
                                   AND ($3::integer IS NULL OR version < $3)",
    )
    .bind(id)
    .bind(PASSWORD_FIELD)
    .bind(version)
    .execute(&mut **tx)
    .await?;
    Ok(before.map(|stored| stored.server))
}

/// Removes the server and its password; whether there was one.
pub async fn remove(tx: &mut Transaction<'_, Postgres>) -> Result<bool, sqlx::Error> {
    let removed: Option<Uuid> = sqlx::query_scalar("DELETE FROM mail RETURNING id")
        .fetch_optional(&mut **tx)
        .await?;
    if let Some(id) = removed {
        sqlx::query("DELETE FROM secret_fields WHERE owner_id = $1")
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(removed.is_some())
}

/// A mail to send, as text; the HTML part is made from it.
pub struct Outgoing {
    pub to: String,
    pub subject: String,
    pub text: String,
}

impl Outgoing {
    /// A mail in `locale` from remotehub's catalogs.
    pub fn new(to: &str, locale: Locale, subject: &Message, body: &Message) -> Self {
        Outgoing {
            to: to.to_owned(),
            subject: i18n::render(locale, subject),
            text: i18n::render(locale, body),
        }
    }
}

/// When a code of Kratos runs out, as a mail shows it: `2026-09-27 14:30 UTC`
/// for Kratos' `2026-09-27T14:30:00.123Z`.
pub fn expiry(rfc3339: &str) -> String {
    match rfc3339.split_once('T') {
        Some((date, time)) if time.len() >= 5 => format!("{date} {} UTC", &time[..5]),
        _ => rfc3339.to_owned(),
    }
}

/// Where sending failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// No mail server is set.
    NotConfigured,
    /// The sender's or the recipient's address is no address.
    Address,
    Connect,
    Tls,
    SignIn,
    /// The server refused the sender, the recipient or the mail.
    Rejected,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub step: Step,
    /// The server's answer or the library's words.
    pub detail: String,
}

impl Failure {
    fn new(step: Step, detail: impl ToString) -> Self {
        Failure {
            step,
            detail: detail.to_string(),
        }
    }
}

/// Sends `mail` through the stored server.
pub async fn send(db: &PgPool, vault: &DynVault, mail: Outgoing) -> Result<(), Failure> {
    let (server, password) = stored(db, vault)
        .await
        .map_err(|e| Failure::new(Step::Other, e))?
        .ok_or_else(|| Failure::new(Step::NotConfigured, "no mail server"))?;
    send_with(&server, password, mail).await
}

/// Sends `mail` through `server`, e.g. one not stored yet.
pub async fn send_with(
    server: &Server,
    password: Option<SecretString>,
    mail: Outgoing,
) -> Result<(), Failure> {
    let from = server
        .from_address
        .parse()
        .map(|address| Mailbox::new(Some(server.from_name.clone()), address))
        .map_err(|e| Failure::new(Step::Address, e))?;
    let to: Mailbox = mail
        .to
        .parse()
        .map_err(|e| Failure::new(Step::Address, e))?;
    let html = html(&mail.text);
    let message = lettre::Message::builder()
        .from(from)
        .to(to)
        .subject(mail.subject)
        .multipart(MultiPart::alternative_plain_html(mail.text, html))
        .map_err(|e| Failure::new(Step::Other, e))?;

    let mut tls = TlsParameters::builder(server.host.clone());
    if let Some(pem) = &server.ca_pem {
        for block in pem_blocks(pem) {
            let certificate =
                Certificate::from_pem(block.as_bytes()).map_err(|e| Failure::new(Step::Tls, e))?;
            tls = tls.add_root_certificate(certificate);
        }
    }
    let tls = tls.build_rustls().map_err(|e| Failure::new(Step::Tls, e))?;
    let port = u16::try_from(server.port).map_err(|e| Failure::new(Step::Connect, e))?;
    let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&server.host)
        .port(port)
        .timeout(Some(TIMEOUT))
        .tls(match server.security.as_str() {
            "tls" => Tls::Wrapper(tls),
            "starttls" => Tls::Required(tls),
            _ => Tls::None,
        });
    if let (Some(user), Some(password)) = (&server.username, password) {
        builder = builder.credentials(Credentials::new(
            user.clone(),
            password.expose_secret().to_owned(),
        ));
    }
    builder
        .build()
        .send(message)
        .await
        .map(|_| ())
        .map_err(|error| {
            let step = if error.is_tls() {
                Step::Tls
            } else if error
                .status()
                .is_some_and(|code| matches!(code.to_string().as_str(), "530" | "534" | "535"))
            {
                Step::SignIn
            } else if error.is_permanent() || error.is_transient() {
                Step::Rejected
            } else {
                Step::Connect
            };
            Failure::new(step, error)
        })
}

/// The certificates of a PEM, one block each.
fn pem_blocks(pem: &str) -> Vec<String> {
    const END: &str = "-----END CERTIFICATE-----";
    pem.split_inclusive(END)
        .filter(|block| block.contains("-----BEGIN CERTIFICATE-----"))
        .map(|block| block.trim().to_owned())
        .collect()
}

/// The text as simple HTML: paragraphs at blank lines, breaks at newlines.
fn html(text: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let paragraphs: Vec<String> = escaped
        .split("\n\n")
        .map(|p| format!("<p>{}</p>", p.trim().replace('\n', "<br>")))
        .collect();
    format!(
        "<!doctype html><html><body style=\"font-family: sans-serif\">{}</body></html>",
        paragraphs.join("")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_keeps_the_text_and_escapes_it() {
        let made = html("Hello <you>\nline two\n\nCode: 12&34");
        assert!(made.contains("<p>Hello &lt;you&gt;<br>line two</p><p>Code: 12&amp;34</p>"));
    }

    #[test]
    fn shows_when_a_code_runs_out_in_minutes() {
        assert_eq!(expiry("2026-09-27T14:30:59.123Z"), "2026-09-27 14:30 UTC");
        assert_eq!(expiry("tomorrow"), "tomorrow");
    }

    #[test]
    fn splits_a_pem_into_its_certificates() {
        let one = "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----";
        let two = format!("{one}\n\n{one}\n");
        assert_eq!(pem_blocks(&two), [one, one]);
        assert!(pem_blocks("nothing").is_empty());
    }
}
