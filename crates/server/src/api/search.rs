//! What a user picked after searching the devices (#81), so the browser can
//! put it first next time. Only the user's own picks, never anyone else's;
//! ranking happens in the browser (`web/src/lib/search/rank.ts`).
//!
//! - `GET /api/search/picks`: the caller's picks
//! - `POST /api/search/picks`: one more pick of `key` for `query`

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::catalog::{body, invalid};
use super::problem::Problem;
use crate::AppState;
use crate::session::Session;

/// Picks kept per user; the ones used longest ago go. The browser keeps as
/// many (`MAX_PICKS` there).
const KEEP: i64 = 300;
const MAX_QUERY: usize = 100;

#[derive(Serialize, sqlx::FromRow)]
pub struct Pick {
    key: String,
    query: String,
    count: i32,
    /// Milliseconds since the epoch.
    last: i64,
}

#[derive(Deserialize)]
pub struct NewPick {
    key: String,
    query: String,
}

pub async fn picks(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<Pick>>, Problem> {
    let picks = sqlx::query_as(
        "SELECT key, query, count, (extract(epoch FROM last_at) * 1000)::bigint AS last
         FROM search_picks WHERE user_id = $1 ORDER BY last_at DESC",
    )
    .bind(session.user_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(picks))
}

/// `device:<uuid>`, `folder:<uuid>` or `credential:<uuid>`.
fn valid_key(key: &str) -> bool {
    key.split_once(':').is_some_and(|(kind, id)| {
        matches!(kind, "folder" | "device" | "credential")
            && id.len() == 36
            && Uuid::parse_str(id).is_ok()
    })
}

pub async fn pick(
    State(state): State<AppState>,
    session: Session,
    input: Result<Json<NewPick>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let input = body(input)?;
    let key = input.key.to_ascii_lowercase();
    if !valid_key(&key) {
        return Err(invalid("key"));
    }
    let query = input.query.trim();
    if query.chars().count() > MAX_QUERY || query.chars().any(char::is_control) {
        return Err(invalid("query"));
    }
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO search_picks (user_id, key, query) VALUES ($1, $2, $3)
         ON CONFLICT (user_id, key, query)
         DO UPDATE SET count = search_picks.count + 1, last_at = now()",
    )
    .bind(session.user_id)
    .bind(&key)
    .bind(query)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM search_picks WHERE user_id = $1 AND (key, query) NOT IN (
             SELECT key, query FROM search_picks WHERE user_id = $1
             ORDER BY last_at DESC LIMIT $2)",
    )
    .bind(session.user_id)
    .bind(KEEP)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_name_an_object() {
        let id = "0d9c2f3e-4b1a-4c55-9a7e-0f2b3c4d5e6f";
        for good in [
            format!("device:{id}"),
            format!("folder:{id}"),
            format!("credential:{id}"),
        ] {
            assert!(valid_key(&good), "{good}");
        }
        for bad in [
            format!("user:{id}"),
            format!("device:{id}x"),
            "device:".to_owned(),
            id.to_owned(),
            format!("device:{}", &id[..35]),
        ] {
            assert!(!valid_key(&bad), "{bad}");
        }
    }
}
