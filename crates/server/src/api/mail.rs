//! `/api/settings/mail`: the mail server (#145), for administrators.
//!
//! - `GET`: the server without its password.
//! - `PUT`: stores it. A password never goes over an unencrypted
//!   connection; without a new one, the stored one stays while the user
//!   name does.
//! - `DELETE`: removes it; remotehub sends no mail then.
//! - `POST …/test`: sends a test mail with the given settings, which need
//!   not be stored, and says where it failed.
//!
//! Changes are audited with the names of the fields that changed, never
//! with the password.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use remotehub_i18n::{Locale, Message};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::catalog::{body, invalid};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::mail::{self, Failure, MailError, Outgoing, Server};
use crate::session::Session;

fn require_admin(session: &Session) -> Result<(), Problem> {
    if session.is_admin() {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::Forbidden))
    }
}

fn failed(error: MailError) -> Problem {
    tracing::error!(%error, "mail settings");
    Problem::new(ErrorCode::Internal)
}

#[derive(Serialize)]
pub struct Stored {
    server: Option<Server>,
}

pub async fn get(State(state): State<AppState>, session: Session) -> Result<Json<Stored>, Problem> {
    require_admin(&session)?;
    Ok(Json(Stored {
        server: mail::server(&state.db).await?,
    }))
}

#[derive(Deserialize)]
pub struct Input {
    host: String,
    #[serde(default)]
    port: Option<i32>,
    security: String,
    #[serde(default)]
    username: Option<String>,
    /// None or empty: the stored one, while the user name stays.
    #[serde(default)]
    password: Option<String>,
    from_address: String,
    #[serde(default)]
    from_name: Option<String>,
    #[serde(default)]
    ca_pem: Option<String>,
}

/// The server `input` describes, checked for form, and its new password.
fn server(input: Input) -> Result<(Server, Option<SecretString>), Problem> {
    let host = input.host.trim().to_owned();
    if host.is_empty() || host.len() > 253 || host.contains(char::is_whitespace) {
        return Err(invalid("host"));
    }
    let default_port = match input.security.as_str() {
        "tls" => 465,
        "starttls" => 587,
        "none" => 25,
        _ => return Err(invalid("security")),
    };
    let port = input.port.unwrap_or(default_port);
    if !(1..=65535).contains(&port) {
        return Err(invalid("port"));
    }
    let optional =
        |value: Option<String>| value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    let username = optional(input.username);
    let password = input
        .password
        .filter(|p| !p.is_empty())
        .map(SecretString::from);
    // A password never goes over an unencrypted connection.
    if input.security == "none" && (username.is_some() || password.is_some()) {
        return Err(invalid("password"));
    }
    if password.is_some() && username.is_none() {
        return Err(invalid("username"));
    }
    let from_address = input.from_address.trim().to_owned();
    if from_address.parse::<lettre::Address>().is_err() {
        return Err(invalid("from_address"));
    }
    let from_name = optional(input.from_name).unwrap_or_else(|| "remotehub".to_owned());
    if from_name.chars().count() > 200 || from_name.chars().any(char::is_control) {
        return Err(invalid("from_name"));
    }
    let ca_pem = optional(input.ca_pem);
    if ca_pem
        .as_deref()
        .is_some_and(|pem| !pem.contains("-----BEGIN CERTIFICATE-----") || pem.len() > 65_536)
    {
        return Err(invalid("ca_pem"));
    }
    Ok((
        Server {
            host,
            port,
            security: input.security,
            username,
            from_address,
            from_name,
            ca_pem,
        },
        password,
    ))
}

fn entry<'a>(
    session: &'a Session,
    action: Action,
    details: serde_json::Value,
    address: &'a str,
) -> Entry<'a> {
    Entry {
        actor: Actor {
            id: Some(session.user_id),
            name: &session.username,
        },
        action,
        object: None,
        details,
        address: Some(address),
    }
}

/// The names of the fields that differ between `before` and `after`.
fn changed(before: Option<&Server>, after: &Server) -> Vec<&'static str> {
    let fields = |s: &Server| {
        [
            ("host", s.host.clone()),
            ("port", s.port.to_string()),
            ("security", s.security.clone()),
            ("username", s.username.clone().unwrap_or_default()),
            ("from_address", s.from_address.clone()),
            ("from_name", s.from_name.clone()),
            ("ca_pem", s.ca_pem.clone().unwrap_or_default()),
        ]
    };
    let after = fields(after);
    match before {
        None => after.iter().map(|(name, _)| *name).collect(),
        Some(before) => fields(before)
            .iter()
            .zip(&after)
            .filter(|(b, a)| b.1 != a.1)
            .map(|(b, _)| b.0)
            .collect(),
    }
}

pub async fn save(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<Input>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    require_admin(&session)?;
    let (server, new_password) = server(body(input)?)?;
    let password = mail::password_for(&state.db, &state.vault, &server, new_password.clone())
        .await
        .map_err(failed)?;
    let mut tx = state.db.begin().await?;
    let before = mail::save(
        &mut tx,
        &state.vault,
        &server,
        password.as_ref(),
        session.user_id,
    )
    .await
    .map_err(failed)?;
    let mut fields = changed(before.as_ref(), &server);
    if new_password.is_some() {
        fields.push("password");
    }
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::MailChanged,
            json!({ "fields": fields }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
) -> Result<StatusCode, Problem> {
    require_admin(&session)?;
    let mut tx = state.db.begin().await?;
    if !mail::remove(&mut tx).await? {
        return Err(Problem::new(ErrorCode::NotFound));
    }
    audit::record(
        &mut *tx,
        entry(&session, Action::MailRemoved, json!({}), &address),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct TestInput {
    #[serde(flatten)]
    server: Input,
    to: String,
    /// The language of the test mail, as the UI shows it.
    #[serde(default)]
    language: String,
}

#[derive(Serialize)]
pub struct Tested {
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<Failure>,
}

pub async fn test(
    State(state): State<AppState>,
    session: Session,
    input: Result<Json<TestInput>, JsonRejection>,
) -> Result<Json<Tested>, Problem> {
    require_admin(&session)?;
    let input = body(input)?;
    let (server, new_password) = server(input.server)?;
    let password = mail::password_for(&state.db, &state.vault, &server, new_password)
        .await
        .map_err(failed)?;
    let locale = Locale::from_accept_language(&input.language);
    let sent = mail::send_with(
        &server,
        password,
        Outgoing::new(
            input.to.trim(),
            locale,
            &Message::MailTestSubject {},
            &Message::MailTestBody {},
        ),
    )
    .await;
    Ok(Json(Tested {
        failure: sent.err(),
    }))
}
