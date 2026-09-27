//! The password generator's settings (#194). The browser generates; the
//! server only keeps what it should make.
//!
//! - `GET /api/generator`: the organisation's settings, built in until an
//!   administrator sets them, and the user's own, if they keep any.
//! - `PUT`/`DELETE /api/generator/own`: the user's own default; without it,
//!   the organisation's applies. A preference, not audited.
//! - `PUT /api/settings/generator`: the organisation's, for administrators;
//!   audited with the settings.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::catalog::{body, invalid};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::session::Session;

/// What the generator makes; the fields as `web/src/lib/vault/generate.ts`
/// reads them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Settings {
    /// `password` or `passphrase`.
    kind: String,
    length: i32,
    lower: bool,
    upper: bool,
    digits: bool,
    symbols: bool,
    /// Whether a password may hold 0 O 1 l I.
    look_alikes: bool,
    words: i32,
    separator: String,
}

impl Settings {
    fn built_in() -> Self {
        Settings {
            kind: "password".to_owned(),
            length: 20,
            lower: true,
            upper: true,
            digits: true,
            symbols: true,
            look_alikes: false,
            words: 6,
            separator: "-".to_owned(),
        }
    }

    /// The settings if they are ones the generator can follow; the bounds
    /// are the table's.
    fn checked(self) -> Result<Self, Problem> {
        if !matches!(self.kind.as_str(), "password" | "passphrase") {
            return Err(invalid("kind"));
        }
        if !(8..=128).contains(&self.length) {
            return Err(invalid("length"));
        }
        if !(self.lower || self.upper || self.digits || self.symbols) {
            return Err(invalid("characters"));
        }
        if !(3..=20).contains(&self.words) {
            return Err(invalid("words"));
        }
        if self.separator.chars().count() > 3 || self.separator.chars().any(char::is_control) {
            return Err(invalid("separator"));
        }
        Ok(self)
    }
}

/// The organisation's settings with `user` None, else that user's own.
async fn stored(state: &AppState, user: Option<Uuid>) -> Result<Option<Settings>, Problem> {
    Ok(sqlx::query_as(
        "SELECT kind, length, lower, upper, digits, symbols, look_alikes, words, separator
         FROM generator_settings WHERE user_id IS NOT DISTINCT FROM $1",
    )
    .bind(user)
    .fetch_optional(&state.db)
    .await?)
}

async fn store(
    executor: impl sqlx::PgExecutor<'_>,
    user: Option<Uuid>,
    settings: &Settings,
) -> Result<(), Problem> {
    // The unique key treats NULL as one value, so the organisation's row
    // is replaced as a user's is.
    sqlx::query(
        "INSERT INTO generator_settings
             (user_id, kind, length, lower, upper, digits, symbols, look_alikes, words, separator)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (user_id) DO UPDATE SET
             kind = $2, length = $3, lower = $4, upper = $5, digits = $6, symbols = $7,
             look_alikes = $8, words = $9, separator = $10, updated_at = now()",
    )
    .bind(user)
    .bind(&settings.kind)
    .bind(settings.length)
    .bind(settings.lower)
    .bind(settings.upper)
    .bind(settings.digits)
    .bind(settings.symbols)
    .bind(settings.look_alikes)
    .bind(settings.words)
    .bind(&settings.separator)
    .execute(executor)
    .await?;
    Ok(())
}

#[derive(Serialize)]
pub struct Stored {
    organisation: Settings,
    own: Option<Settings>,
}

pub async fn get(State(state): State<AppState>, session: Session) -> Result<Json<Stored>, Problem> {
    Ok(Json(Stored {
        organisation: stored(&state, None)
            .await?
            .unwrap_or_else(Settings::built_in),
        own: stored(&state, Some(session.user_id)).await?,
    }))
}

pub async fn save_own(
    State(state): State<AppState>,
    session: Session,
    input: Result<Json<Settings>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let settings = body(input)?.checked()?;
    store(&state.db, Some(session.user_id), &settings).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_own(
    State(state): State<AppState>,
    session: Session,
) -> Result<StatusCode, Problem> {
    sqlx::query("DELETE FROM generator_settings WHERE user_id = $1")
        .bind(session.user_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn save_organisation(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<Settings>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    if !session.is_admin() {
        return Err(Problem::new(ErrorCode::Forbidden));
    }
    let settings = body(input)?.checked()?;
    let mut tx = state.db.begin().await?;
    store(&mut *tx, None, &settings).await?;
    audit::record(
        &mut *tx,
        Entry {
            actor: Actor {
                id: Some(session.user_id),
                name: &session.username,
            },
            action: Action::GeneratorChanged,
            object: None,
            details: json!(settings),
            address: Some(&address),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
