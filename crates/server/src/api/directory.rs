//! `/api/settings/directory`: the directory connection (#144), for
//! administrators.
//!
//! - `GET`: the connection without its password, and whether one is stored.
//! - `POST …/check`: checks the given settings step by step; the stored
//!   password stands in for a missing one.
//! - `PUT`: checks, then stores what passed and signs in against it from the
//!   next sign-in on. A new URL or base DN ends every directory user's
//!   session: their groups may mean something else now.
//! - `DELETE`: removes the connection, and every directory user's session.
//!
//! Changes are audited with the names of the fields that changed, never
//! with the password.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use remotehub_directory::check::{self, Failure, Found};
use remotehub_directory::ldap::LdapDirectory;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::catalog::{body, invalid};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::directory::{self, Connection, DEFAULT_TIMEOUT_SECONDS, DirectoryError};
use crate::session::Session;

fn require_admin(session: &Session) -> Result<(), Problem> {
    if session.is_admin() {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::Forbidden))
    }
}

fn failed(error: DirectoryError) -> Problem {
    tracing::error!(%error, "directory settings");
    Problem::new(ErrorCode::Internal)
}

#[derive(Serialize)]
pub struct Stored {
    connection: Option<Connection>,
}

pub async fn get(State(state): State<AppState>, session: Session) -> Result<Json<Stored>, Problem> {
    require_admin(&session)?;
    Ok(Json(Stored {
        connection: directory::connection(&state.db).await?,
    }))
}

#[derive(Deserialize)]
pub struct Input {
    url: String,
    #[serde(default)]
    starttls: bool,
    #[serde(default)]
    ca_pem: Option<String>,
    bind_dn: String,
    /// None or empty: the stored one.
    #[serde(default)]
    password: Option<String>,
    base_dn: String,
    #[serde(default)]
    user_filter: Option<String>,
    #[serde(default)]
    timeout_seconds: Option<i32>,
}

/// The connection `input` describes, checked for form, and its new password.
fn connection(input: Input) -> Result<(Connection, Option<SecretString>), Problem> {
    let url = input.url.trim().to_owned();
    let scheme = url.to_ascii_lowercase();
    if !(scheme.starts_with("ldaps://") || scheme.starts_with("ldap://")) || url.len() > 500 {
        return Err(invalid("url"));
    }
    // Passwords never travel unencrypted.
    if scheme.starts_with("ldap://") && !input.starttls {
        return Err(invalid("starttls"));
    }
    let text = |value: String, field: &'static str| {
        let value = value.trim().to_owned();
        if value.is_empty() || value.len() > 500 || value.chars().any(char::is_control) {
            Err(invalid(field))
        } else {
            Ok(value)
        }
    };
    let optional =
        |value: Option<String>| value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    let ca_pem = optional(input.ca_pem);
    if ca_pem
        .as_deref()
        .is_some_and(|pem| !pem.contains("-----BEGIN CERTIFICATE-----") || pem.len() > 65_536)
    {
        return Err(invalid("ca_pem"));
    }
    let user_filter = optional(input.user_filter);
    if user_filter
        .as_deref()
        .is_some_and(|f| !(f.starts_with('(') && f.ends_with(')')) || f.len() > 2000)
    {
        return Err(invalid("user_filter"));
    }
    let timeout_seconds = input.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    if !(1..=60).contains(&timeout_seconds) {
        return Err(invalid("timeout_seconds"));
    }
    let connection = Connection {
        url,
        starttls: input.starttls,
        ca_pem,
        bind_dn: text(input.bind_dn, "bind_dn")?,
        base_dn: text(input.base_dn, "base_dn")?,
        user_filter,
        timeout_seconds,
    };
    let password = input
        .password
        .filter(|p| !p.is_empty())
        .map(SecretString::from);
    Ok((connection, password))
}

/// What a check gave: what it found, or where it failed.
#[derive(Serialize)]
pub struct Checked {
    #[serde(skip_serializing_if = "Option::is_none")]
    found: Option<Found>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<Failure>,
}

/// Checks `connection` with `password`, or the stored password.
async fn run_check(
    state: &AppState,
    connection: &Connection,
    password: Option<SecretString>,
) -> Result<(SecretString, Result<Found, Failure>), Problem> {
    let password = directory::password_for(&state.db, &state.vault, password)
        .await
        .map_err(failed)?
        .ok_or_else(|| invalid("password"))?;
    let result = check::check(connection.config(password.clone())).await;
    Ok((password, result))
}

pub async fn check(
    State(state): State<AppState>,
    session: Session,
    input: Result<Json<Input>, JsonRejection>,
) -> Result<Json<Checked>, Problem> {
    require_admin(&session)?;
    let (connection, password) = connection(body(input)?)?;
    let (_, result) = run_check(&state, &connection, password).await?;
    Ok(Json(match result {
        Ok(found) => Checked {
            found: Some(found),
            failure: None,
        },
        Err(failure) => Checked {
            found: None,
            failure: Some(failure),
        },
    }))
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
fn changed(before: Option<&Connection>, after: &Connection) -> Vec<&'static str> {
    let Some(before) = before else {
        return vec![
            "url",
            "starttls",
            "ca_pem",
            "bind_dn",
            "base_dn",
            "user_filter",
            "timeout_seconds",
        ];
    };
    [
        ("url", before.url != after.url),
        ("starttls", before.starttls != after.starttls),
        ("ca_pem", before.ca_pem != after.ca_pem),
        ("bind_dn", before.bind_dn != after.bind_dn),
        ("base_dn", before.base_dn != after.base_dn),
        ("user_filter", before.user_filter != after.user_filter),
        (
            "timeout_seconds",
            before.timeout_seconds != after.timeout_seconds,
        ),
    ]
    .into_iter()
    .filter_map(|(name, differs)| differs.then_some(name))
    .collect()
}

pub async fn save(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<Input>, JsonRejection>,
) -> Result<Json<Found>, Problem> {
    require_admin(&session)?;
    let (connection, new_password) = connection(body(input)?)?;
    let (password, result) = run_check(&state, &connection, new_password.clone()).await?;
    let found = result.map_err(|failure| {
        Problem::new(ErrorCode::DirectoryCheckFailed)
            .param("step", json!(failure.step))
            .param("reason", json!(failure.reason))
            .param("detail", failure.detail)
    })?;
    let directory = LdapDirectory::new(connection.config(password))
        .map_err(|e| failed(DirectoryError::Invalid(e.to_string())))?;

    let mut tx = state.db.begin().await?;
    let before = directory::save(
        &mut tx,
        &state.vault,
        &connection,
        new_password.as_ref(),
        session.user_id,
    )
    .await
    .map_err(failed)?;
    let mut fields = changed(before.as_ref(), &connection);
    if new_password.is_some() {
        fields.push("password");
    }
    let moved = before
        .as_ref()
        .is_some_and(|b| b.url != connection.url || b.base_dn != connection.base_dn);
    let ended = if moved {
        directory::end_directory_sessions(&mut *tx).await?
    } else {
        0
    };
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::DirectoryChanged,
            json!({ "fields": fields, "sessions_ended": ended }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    state.directory.set(Some(Arc::new(directory)));
    Ok(Json(found))
}

pub async fn remove(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
) -> Result<StatusCode, Problem> {
    require_admin(&session)?;
    let mut tx = state.db.begin().await?;
    if !directory::remove(&mut tx).await? {
        return Err(Problem::new(ErrorCode::NotFound));
    }
    let ended = directory::end_directory_sessions(&mut *tx).await?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::DirectoryRemoved,
            json!({ "sessions_ended": ended }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    state.directory.set(None);
    Ok(StatusCode::NO_CONTENT)
}
